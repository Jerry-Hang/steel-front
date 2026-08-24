//! 占领据点（Capture）目标系统与全局胜负规则判定
//!
//! - `CapturePoint`：据点（id/x/z/半径/占领耗时），`update_point` 纯函数推进归属
//! - `GameRule` / `ObjectiveState`：规则（占领/击杀/限时）与每帧胜负判定 `evaluate`
//!
//! 本模块独立于 game.rs 的 `MissionObjective`（歼灭数）——两者互不干扰，由主会话统一接线。
//! 仅依赖 std + log，零第三方依赖。
//!
//! 网络同步：目标状态由主会话在 game.rs 直接读取 `ObjectiveState.points`（pub 字段）编码为
//! net.rs 的 `NetworkMessage::ObjectiveState`（0x07）广播；本模块不提供序列化（见 net.rs）。
//!
//! 判定规则细节（勿回退）：
//! - 占领进度为标量 0.0..=1.0，向「当前推进方」增长；满 1.0 → 归属切换为该方、进度归零
//!   （owner 字段本身记录归属，归零后「已占领」状态由 owner 表达，撤离后敌对可从零重新累计）。
//! - 推进条件 = 据点内恰好单一玩家阵营且无敌对（NPC/敌方玩家）在场；多阵营玩家同点视为争夺。
//! - 衰减：敌对在场按 2×capture_time 快速衰减（争夺压制）；无人按 1×capture_time 缓慢消散；
//!   owner 阵营玩家在场守点 → 进度维持（默认规则，含敌对在场场景）。
//! - CapturePoints 胜利：任一队占领数 ≥ required 即胜；双方同时达标或「全部被占但无人达标」
//!   （如 required=3 只有 2 个点）→ 占领多者胜，平手 → None（僵持继续）。
//! - KillCount 胜利：玩家（Blue 阵营）击杀达 target；Red 不通过击杀获胜。
//! - TimeLimit：到点后按据点归属定胜负（多者胜）；平局比玩家击杀（kills>0 → Victory(Blue)，
//!   否则 Defeat，避免无限僵持）；未到点 → None。
//! - 幂等：主会话收到 Victory/Defeat 后置位 `won_team`，此后 `evaluate` 恒返回 None，
//!   保证胜利事件只触发一次。

use crate::engine::ai::Team;

/// 占领据点：id 唯一标识（"A"/"B"/…），(x,z) 为水平位置（y 取地面高度，忽略），
/// `radius` 为占领判定半径，`capture_time` 为占领所需总秒数。
#[derive(Debug, Clone, PartialEq)]
pub struct CapturePoint {
    pub id: String,
    pub x: f32,
    pub z: f32,
    pub radius: f32,
    pub capture_time: f32,
    /// 当前归属：None=中立 / Some(Red) / Some(Blue)
    pub owner: Option<Team>,
    /// 占领进度 0.0..=1.0（向当前推进方增长；满 1.0 归属切换并归零）
    pub progress: f32,
}

impl CapturePoint {
    /// 新建中立据点（owner=None、progress=0.0）
    pub fn new(id: impl Into<String>, x: f32, z: f32, radius: f32, capture_time: f32) -> Self {
        Self {
            id: id.into(),
            x,
            z,
            radius,
            capture_time,
            owner: None,
            progress: 0.0,
        }
    }

    /// 水平距离（忽略 y）<= radius 视为在据点内（恰在半径上算在内）
    pub fn is_inside(&self, px: f32, pz: f32) -> bool {
        let dx = px - self.x;
        let dz = pz - self.z;
        dx * dx + dz * dz <= self.radius * self.radius
    }
}

/// 推进一个据点的归属状态（纯函数，每帧对每个据点调用一次）。
///
/// - `players_inside`：据点内玩家阵营列表（单人游戏通常只有单一阵营在）。
/// - `enemies_inside`：据点内是否有敌对阵营（NPC/敌方玩家）。
///
/// 规则（见模块文档）：
/// 1. owner 阵营玩家在场守点 → 进度维持（含敌对在场），返回 false；
/// 2. 否则若恰好单一玩家阵营且无敌对 → 进度 += dt/capture_time，满 1.0 归属切换并归零；
/// 3. 否则（无人/有敌对/多阵营争夺）→ 进度向 0 衰减（敌对 2×，无人 1×）。
///
/// 返回是否发生归属切换（供主会话触发事件/日志）。
pub fn update_point(
    point: &mut CapturePoint,
    dt: f32,
    players_inside: &[Team],
    enemies_inside: bool,
) -> bool {
    let inv = 1.0 / point.capture_time.max(f32::EPSILON);

    // 规则 1：owner 阵营玩家在场守点 → 保持占领（进度维持，占领后恒为 0）
    if let Some(owner) = point.owner {
        if players_inside.contains(&owner) {
            return false;
        }
    }

    // 确定推进方：恰好单一玩家阵营且无敌对（多阵营同点视为争夺，不推进）
    let capturer = if enemies_inside {
        None
    } else {
        match players_inside {
            [] => None,
            [single] => Some(*single),
            many => {
                if many.iter().all(|t| *t == many[0]) {
                    Some(many[0])
                } else {
                    None
                }
            }
        }
    };

    match capturer {
        Some(team) => {
            // 规则 2：向推进方增长，满 1.0 归属切换
            point.progress += dt * inv;
            if point.progress >= 1.0 {
                point.owner = Some(team);
                point.progress = 0.0;
                log::info!(
                    "objective: capture point {} now owned by {:?}",
                    point.id,
                    team
                );
                true
            } else {
                false
            }
        }
        None => {
            // 规则 3：衰减（敌对在场 2× 快，无人 1× 慢）
            let rate = if enemies_inside { 2.0 * inv } else { inv };
            point.progress = (point.progress - dt * rate).max(0.0);
            false
        }
    }
}

/// 全局胜负规则（主会话解析 TOML rule 后经 `from_toml` 构造）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GameRule {
    /// 某方占领 `required` 个据点即胜利
    CapturePoints { required: usize },
    /// 玩家（Blue 阵营）击杀达 `target` 即胜利
    KillCount { target: u32 },
    /// 限时：到点按据点归属/击杀数判定胜负
    TimeLimit { seconds: f64 },
    /// 防守波次：玩家守住 `waves` 波 NPC 进攻即胜利（波间有补给窗口）；
    /// 玩家死亡即失败（由 game.rs 波次循环驱动，evaluate 不直接判定）
    Survive { waves: u32 },
}

impl GameRule {
    /// 规则种类字符串："capture" / "kill" / "time" / "survive"（供日志与 TOML 校验）
    pub fn rule_kind(&self) -> &'static str {
        match self {
            GameRule::CapturePoints { .. } => "capture",
            GameRule::KillCount { .. } => "kill",
            GameRule::TimeLimit { .. } => "time",
            GameRule::Survive { .. } => "survive",
        }
    }
}

/// 每帧胜负判定结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinState {
    /// 未分胜负
    None,
    /// 玩家方（Blue）胜利
    Victory(Team),
    /// 敌方（Red）胜利
    Defeat,
}

/// 目标系统状态：规则 + 据点列表 + 累计击杀 + 已进行秒数
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveState {
    pub rule: GameRule,
    /// 据点列表（主会话按地图配置填充：new 后 push 或整体赋值 points）
    pub points: Vec<CapturePoint>,
    /// 玩家累计击杀（KillCount 与 TimeLimit 平局裁定用）
    pub kills: u32,
    /// 本关已进行秒数（TimeLimit 用，主会话每帧累加）
    pub elapsed: f64,
    /// 已判定胜利方（主会话在收到 Victory/Defeat 后置位；置位后 evaluate 返回 None，幂等）
    pub won_team: Option<Team>,
}

impl ObjectiveState {
    /// 新建状态：空据点列表、0 击杀、0 秒、未判定
    pub fn new(rule: GameRule) -> Self {
        Self {
            rule,
            points: Vec::new(),
            kills: 0,
            elapsed: 0.0,
            won_team: None,
        }
    }

    /// 每帧胜负判定（纯函数）。已置位 `won_team` → None（幂等，胜利事件只触发一次）。
    /// 具体规则见模块文档。
    pub fn evaluate(&self) -> WinState {
        if self.won_team.is_some() {
            return WinState::None;
        }
        match self.rule {
            GameRule::CapturePoints { required } => {
                let (blue, red) = count_owned(&self.points);
                let (blue_hit, red_hit) = (blue >= required, red >= required);
                match (blue_hit, red_hit) {
                    (true, false) => WinState::Victory(Team::Blue),
                    (false, true) => WinState::Defeat,
                    // 双方同时达标（如 required=1 各占 1）或全部被占但无人达标
                    // （如 required=3 只有 2 个点）→ 占领多者胜，平手 → None（僵持继续）
                    _ => {
                        let all_owned = self.points.iter().all(|p| p.owner.is_some());
                        if blue_hit && red_hit || all_owned {
                            if blue > red {
                                WinState::Victory(Team::Blue)
                            } else if red > blue {
                                WinState::Defeat
                            } else {
                                WinState::None
                            }
                        } else {
                            WinState::None
                        }
                    }
                }
            }
            GameRule::KillCount { target } => {
                if self.kills >= target {
                    WinState::Victory(Team::Blue) // 玩家阵营；Red 不通过击杀获胜
                } else {
                    WinState::None
                }
            }
            GameRule::TimeLimit { seconds } => {
                if self.elapsed < seconds {
                    return WinState::None;
                }
                // 到点：据点多者胜；平局比击杀（kills>0 → 玩家胜，否则判负避免无限僵持）
                let (blue, red) = count_owned(&self.points);
                if blue > red {
                    WinState::Victory(Team::Blue)
                } else if red > blue {
                    WinState::Defeat
                } else if self.kills > 0 {
                    WinState::Victory(Team::Blue)
                } else {
                    WinState::Defeat
                }
            }
            // 防守波次：胜负由 game.rs 波次循环驱动（守住全部波 → Victory；玩家死亡 → Defeat）。
            // evaluate 不直接判定（纯函数无法感知玩家血量/波次进度）；置位 won_team 后返回 None。
            GameRule::Survive { .. } => WinState::None,
        }
    }
}

/// 统计双方占领的据点数，返回 (blue, red)
fn count_owned(points: &[CapturePoint]) -> (usize, usize) {
    let blue = points.iter().filter(|p| p.owner == Some(Team::Blue)).count();
    let red = points.iter().filter(|p| p.owner == Some(Team::Red)).count();
    (blue, red)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(id: &str) -> CapturePoint {
        CapturePoint::new(id.to_string(), 10.0, 20.0, 5.0, 10.0)
    }

    #[test]
    fn point_new_sets_fields_and_is_inside() {
        let p = pt("A");
        assert_eq!(p.id, "A");
        assert_eq!(p.x, 10.0);
        assert_eq!(p.z, 20.0);
        assert_eq!(p.radius, 5.0);
        assert_eq!(p.capture_time, 10.0);
        assert_eq!(p.owner, None);
        assert_eq!(p.progress, 0.0);
        // 中心、半径内（3-4-5）、恰好在半径上 → true；半径外 → false
        assert!(p.is_inside(10.0, 20.0));
        assert!(p.is_inside(13.0, 22.0));
        assert!(p.is_inside(15.0, 20.0));
        assert!(!p.is_inside(15.1, 20.0));
    }

    #[test]
    fn update_point_progress_grows_toward_single_faction() {
        let mut p = pt("A");
        // Blue 单方进点、无敌人：进度 += dt/capture_time
        let switched = update_point(&mut p, 2.0, &[Team::Blue], false);
        assert!(!switched);
        assert_eq!(p.owner, None);
        assert!((p.progress - 0.2).abs() < 1e-6);
    }

    #[test]
    fn update_point_switches_owner_at_full() {
        let mut p = pt("A");
        update_point(&mut p, 9.0, &[Team::Blue], false);
        assert!((p.progress - 0.9).abs() < 1e-6);
        let switched = update_point(&mut p, 2.0, &[Team::Blue], false);
        assert!(switched);
        assert_eq!(p.owner, Some(Team::Blue));
        assert_eq!(p.progress, 0.0); // 满 1.0 切换后归零
    }

    #[test]
    fn update_point_decays_when_empty_or_contested() {
        // 无人：按 1× capture_time 衰减
        let mut p = pt("A");
        p.progress = 0.5;
        update_point(&mut p, 1.0, &[], false);
        assert!((p.progress - 0.4).abs() < 1e-6);
        // 有敌对：即使玩家在点内也不增长，按 2× 衰减
        let mut p2 = pt("A");
        p2.progress = 0.5;
        update_point(&mut p2, 1.0, &[Team::Blue], true);
        assert!((p2.progress - 0.3).abs() < 1e-6);
    }

    #[test]
    fn update_point_contested_decays_faster_than_empty() {
        // 敌对 2× vs 无人 1×：同起点同 dt，敌对侧衰减更快
        let mut a = pt("A");
        a.progress = 0.9;
        update_point(&mut a, 2.5, &[Team::Blue], true);
        let mut b = pt("B");
        b.progress = 0.9;
        update_point(&mut b, 2.5, &[], false);
        assert!((a.progress - 0.4).abs() < 1e-6);
        assert!((b.progress - 0.65).abs() < 1e-6);
        assert!(a.progress < b.progress);
    }

    #[test]
    fn update_point_owner_present_holds() {
        // owner 阵营玩家在场守点：进度维持（含敌对在场），不衰减不增长
        let mut p = pt("A");
        p.owner = Some(Team::Red);
        p.progress = 0.3;
        let switched = update_point(&mut p, 5.0, &[Team::Red], true);
        assert!(!switched);
        assert_eq!(p.owner, Some(Team::Red));
        assert!((p.progress - 0.3).abs() < 1e-6);
    }

    #[test]
    fn update_point_opponent_flips_owned_point() {
        // 敌对玩家（Red）进 Blue 的据点且 Blue 无人守 → 进度增长并翻转归属
        let mut p = pt("A");
        p.owner = Some(Team::Blue);
        let switched = update_point(&mut p, 10.5, &[Team::Red], false);
        assert!(switched);
        assert_eq!(p.owner, Some(Team::Red));
        assert_eq!(p.progress, 0.0);
    }

    #[test]
    fn evaluate_capture_required_reached_and_tie() {
        // 单方占领 required 个 → Victory/Defeat
        let mut s = ObjectiveState::new(GameRule::CapturePoints { required: 1 });
        s.points.push(CapturePoint::new("A", 0.0, 0.0, 5.0, 10.0));
        s.points[0].owner = Some(Team::Blue);
        assert_eq!(s.evaluate(), WinState::Victory(Team::Blue));

        let mut s = ObjectiveState::new(GameRule::CapturePoints { required: 1 });
        s.points.push(CapturePoint::new("A", 0.0, 0.0, 5.0, 10.0));
        s.points[0].owner = Some(Team::Red);
        assert_eq!(s.evaluate(), WinState::Defeat);

        // 双方各占 1 个（required=1）：同时达标 → 平手 → None
        let mut s = ObjectiveState::new(GameRule::CapturePoints { required: 1 });
        s.points.push(CapturePoint::new("A", 0.0, 0.0, 5.0, 10.0));
        s.points.push(CapturePoint::new("B", 0.0, 0.0, 5.0, 10.0));
        s.points[0].owner = Some(Team::Blue);
        s.points[1].owner = Some(Team::Red);
        assert_eq!(s.evaluate(), WinState::None);
    }

    #[test]
    fn evaluate_capture_unreachable_required_uses_majority() {
        // required=3 但只有 2 个据点：全部被占、无人达标 → 占领多者胜
        let mut s = ObjectiveState::new(GameRule::CapturePoints { required: 3 });
        s.points.push(CapturePoint::new("A", 0.0, 0.0, 5.0, 10.0));
        s.points.push(CapturePoint::new("B", 0.0, 0.0, 5.0, 10.0));
        s.points[0].owner = Some(Team::Blue);
        s.points[1].owner = Some(Team::Blue);
        assert_eq!(s.evaluate(), WinState::Victory(Team::Blue));

        // 1:1 平手 → None
        let mut s = ObjectiveState::new(GameRule::CapturePoints { required: 3 });
        s.points.push(CapturePoint::new("A", 0.0, 0.0, 5.0, 10.0));
        s.points.push(CapturePoint::new("B", 0.0, 0.0, 5.0, 10.0));
        s.points[0].owner = Some(Team::Blue);
        s.points[1].owner = Some(Team::Red);
        assert_eq!(s.evaluate(), WinState::None);

        // 存在中立（未全部被占）→ None
        let mut s = ObjectiveState::new(GameRule::CapturePoints { required: 3 });
        s.points.push(CapturePoint::new("A", 0.0, 0.0, 5.0, 10.0));
        s.points.push(CapturePoint::new("B", 0.0, 0.0, 5.0, 10.0));
        s.points[0].owner = Some(Team::Blue);
        assert_eq!(s.evaluate(), WinState::None);
    }

    #[test]
    fn evaluate_kill_count() {
        let s = ObjectiveState::new(GameRule::KillCount { target: 5 });
        assert_eq!(s.evaluate(), WinState::None);
        let mut s = ObjectiveState::new(GameRule::KillCount { target: 5 });
        s.kills = 5;
        assert_eq!(s.evaluate(), WinState::Victory(Team::Blue));
    }

    #[test]
    fn evaluate_time_limit() {
        let mut s = ObjectiveState::new(GameRule::TimeLimit { seconds: 60.0 });
        s.points.push(CapturePoint::new("A", 0.0, 0.0, 5.0, 10.0));
        s.points[0].owner = Some(Team::Blue);
        s.elapsed = 59.0;
        assert_eq!(s.evaluate(), WinState::None); // 未到时间
        s.elapsed = 60.0;
        assert_eq!(s.evaluate(), WinState::Victory(Team::Blue)); // Blue 据点更多
        s.points[0].owner = Some(Team::Red);
        assert_eq!(s.evaluate(), WinState::Defeat);
        // 平局（中立）→ 比击杀
        s.points[0].owner = None;
        s.kills = 1;
        assert_eq!(s.evaluate(), WinState::Victory(Team::Blue));
        s.kills = 0;
        assert_eq!(s.evaluate(), WinState::Defeat);
    }

    #[test]
    fn evaluate_idempotent_after_won_team_set() {
        // 已置位 won_team → 恒 None（幂等，胜利事件只触发一次）
        let mut s = ObjectiveState::new(GameRule::CapturePoints { required: 1 });
        s.points.push(CapturePoint::new("A", 0.0, 0.0, 5.0, 10.0));
        s.points[0].owner = Some(Team::Blue);
        s.won_team = Some(Team::Blue);
        assert_eq!(s.evaluate(), WinState::None);
    }

    #[test]
    fn rule_kind_matches() {
        assert_eq!(GameRule::CapturePoints { required: 2 }.rule_kind(), "capture");
        assert_eq!(GameRule::KillCount { target: 5 }.rule_kind(), "kill");
        assert_eq!(GameRule::TimeLimit { seconds: 60.0 }.rule_kind(), "time");
    }
}
