//! Play / Edit 分离（引擎化 P6，见 `docs/ENGINE-PLAN.md`）。
//!
//! 编辑器要能"停住世界、改场景、再按播放看结果"。这件事的**全部难点是别让状态偷偷跑**：
//! 只要还有一条路径在编辑态推进玩法，改完的场景就会被下一次 tick 覆盖，而画面上看不出来
//! （本仓把这类问题叫"静默"）。所以这里把"这一帧要不要推进玩法"收敛成**一个纯函数**，
//! 主循环只问它一次，判据也只测它。
//!
//! 语义（与 Unity 的 Edit/Play 对齐）：
//! - **Edit**：玩法不推进（物理/AI/波次/武器都不动），场景与相机仍可编辑；
//! - **Play**：正常推进（就是今天的行为）；
//! - **暂停**（`Play` + `paused`）：推进但时间步为 0 —— 单帧步进调试用（本次不接线，
//!   先留语义与判据，接线等编辑器壳做按钮时再加，避免"死代码"）。

/// 编辑器/播放态。默认 **Play** —— 玩家直接跑游戏时行为和改造前一模一样。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 正常游玩（默认）。
    Play,
    /// 编辑器态：玩法不推进。
    Edit,
}

impl Mode {
    pub fn is_play(self) -> bool {
        matches!(self, Mode::Play)
    }

    /// 给上报/编辑器状态栏用的名字（机读）。
    pub fn name(self) -> &'static str {
        match self {
            Mode::Play => "play",
            Mode::Edit => "edit",
        }
    }
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Play
    }
}

/// 这一帧是否推进玩法，以及推进用的 dt。
///
/// **纯函数**：主循环把 `mode` / `paused` / 原始 dt 丢进来，拿到"要不要跑、跑多少"。
/// 之所以不把判断散在调用点：散开的判断会出现"物理跑了但 AI 没跑"这种半推进状态，
/// 那是编辑器里最难查的一类 bug（场景看起来一半活的）。
///
/// 编辑态返回 `dt = 0.0`（而不是让调用方跳过调用）：这样依赖"每帧被调用"的系统
/// （HUD 计时、上报）仍然按正常节奏走，只是游戏时间不再前进。
pub fn step_decision(mode: Mode, paused: bool, raw_dt: f32) -> (bool /*推进玩法*/, f32 /*dt*/) {
    if !mode.is_play() || paused {
        return (false, 0.0);
    }
    // 与主循环一致的夹取：负 dt 或超大 dt（切窗口回来）不许进物理。
    (true, raw_dt.clamp(0.0, MAX_DT))
}

/// 单帧 dt 上限（秒）。与 `app_update` 里的夹取同源；切窗口回来时 dt 可能是几秒，
/// 直接喂给物理会把玩家弹穿地图。
pub const MAX_DT: f32 = 0.1;

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔴 默认必须是 **Play**：玩家直接跑游戏时行为与改造前一致（转型不许改玩家体验）。
    #[test]
    fn mode_defaults_to_play() {
        assert_eq!(Mode::default(), Mode::Play);
        assert!(Mode::default().is_play());
    }

    /// 🔴 **编辑态 600 帧：玩法一步都不许推进**（目标里的验收口径）。
    ///
    /// 用一个计数器模拟"每帧推进一次"，跑 600 帧后必须仍是 0。
    /// 反向验证：把 `!mode.is_play()` 那个判断删掉（永远返回 true）⇒ 这条红。
    #[test]
    fn edit_mode_never_advances_the_world() {
        let mut world_time = 0.0f32;
        let mut steps = 0u32;
        for _ in 0..600 {
            let (advance, dt) = step_decision(Mode::Edit, false, 1.0 / 60.0);
            if advance {
                steps += 1;
                world_time += dt;
            }
        }
        assert_eq!(steps, 0, "编辑态不许推进玩法，实际推进了 {steps} 帧");
        assert_eq!(world_time, 0.0, "编辑态世界时间必须停在 0");
    }

    /// 播放态必须推进，且 dt 被夹在 [0, MAX_DT]（切窗口回来不许弹穿地图）。
    #[test]
    fn play_mode_advances_and_clamps_dt() {
        let (adv, dt) = step_decision(Mode::Play, false, 1.0 / 60.0);
        assert!(adv);
        assert!((dt - 1.0 / 60.0).abs() < 1e-9);
        let (adv, dt) = step_decision(Mode::Play, false, 5.0);
        assert!(adv);
        assert!((dt - MAX_DT).abs() < 1e-9, "超大 dt 必须被夹到 {MAX_DT}，实际 {dt}");
        let (adv, dt) = step_decision(Mode::Play, false, -1.0);
        assert!(adv);
        assert_eq!(dt, 0.0, "负 dt 必须夹成 0");
    }

    /// 暂停语义：推进标记为假（单帧步进留给编辑器按钮，本次不接线）。
    #[test]
    fn paused_play_mode_does_not_advance() {
        let (adv, dt) = step_decision(Mode::Play, true, 1.0 / 60.0);
        assert!(!adv);
        assert_eq!(dt, 0.0);
    }
}
