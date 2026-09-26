//! 连排班指挥体系（三三制）：营 → 连 → 排 → 班 → 战士。
//!
//! 编制（**按代码实测写**，判据 `organization_matches_the_documented_three_three_rule`）：
//! - 每 **4 人**切 1 个班（最后一班可以不足 4 人）、每 **3 班**成 1 排、每 **3 排**成 1 连；
//! - 「班长 / 排长 / 连长」都是**本级成员的最后一个**（不额外占编制，`leader` 只做标记）；
//! - 128 人 ⇒ **32 班 / 10 排 / 3 连**，且**连名单逐人等于全营**：末尾凑不满 3 个排的那一截
//!   **并入末位**（末排承接余班、末连承接余排）—— 军情是按连名单汇总的，漏掉一截就是漏一份兵力。
//!   判据 `every_soldier_is_carried_by_a_company`。⚠️ 不足 33 人（9 个班 = 3 个排）编不成连，
//!   此时 `Army::update` 直接跳过指挥层、逐人战术照常。
//!   ⚠️ 旧文案写的「营 = 3 连 + 营部 5 人」与代码不符：**没有单独的营部编制**。
//!
//! 指挥链：战士向班长汇报（状态/位置），班→排→连→营逐级汇总为「军情报告」；
//! 营司令官（AI 司令）每 0.5s 决策一次，判据就是 `Army::update` 里那三段：
//! - 连名单内存活 < **本营编制 × 0.55** 且 累计击杀 < 8 → 重组；否则比敌我重心到地图中心的距离：
//!   我方 < 0.8×敌 ⇒ 进攻；> 1.25×敌 ⇒ 防御；其余 ⇒ 钳形侧翼。
//!   （⚠️ 旧文案「伤亡>40% 转入防御」对不上代码：那个阈值判的是**重组**，防御看重心距离。）
//!   🔴 分母原来写死 `128.0` ⇒ `RV3D_STRESS_AI=64` 时"存活 64 人"恒 < 70.4 ⇒ 司令**永远重组**、
//!   两军永远不接火。现在取 `Army::roster_size()`（各班名单之和）。
//! 命令逐级下发为「班目标点」；未接敌的战士按班目标 + `FORMATION_OFFSETS` 槽位推进
//! （班长前中 4m、双翼侧后 4/3m、殿后 7m；**不随朝向旋转**，大战场上取确定性优先）；
//! 接敌后仍交由既有逐人战术（掩体/侧翼/偷袭/压制）。
//!
//! 同时提供：连长对班长投掷压制的指挥权加成（压制翻倍）、班长大致向敌方向
//! 投掷的信息（投掷逻辑在 game.rs，此处仅编制/目标/报告）。

use crate::engine::ai::{GridMap, Team};

/// 连队军情报告（自下而上汇总：营司令据此决策）
#[derive(Debug, Clone, Copy, Default)]
pub struct CompanyReport {
    pub strength: f32,
    pub centroid: [f32; 2],
    pub contact: bool,
}

/// 连队任务
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanyOrder {
    /// 前推至目标线（进攻）
    Assault,
    /// 占领线保持（防御）
    Hold,
    /// 向侧翼包抄（d=+1 左 / -1 右）
    Flank(i32),
    /// 退回支撑点重组
    Regroup,
}

impl CompanyOrder {
    pub fn label(&self) -> &'static str {
        match self {
            CompanyOrder::Assault => "进攻",
            CompanyOrder::Hold => "防御",
            CompanyOrder::Flank(_) => "侧翼包抄",
            CompanyOrder::Regroup => "重组",
        }
    }
}

/// 营级战役态势（AI 司令的决策依据）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleSituation {
    Offense,
    Defend,
    Pincer,
    Regroup,
}

pub struct Squad {
    pub id: usize,
    pub members: Vec<usize>,
    #[allow(dead_code)] // 建编成时已写入（`members.first()`），当前**无读取方** —— 连级只读 `Company::leader`；排/班级查询预留
    pub leader: Option<usize>,
    pub objective: [f32; 2],
    pub order: CompanyOrder,
}

pub struct Platoon {
    pub id: usize,
    pub members: Vec<usize>,
    #[allow(dead_code)] // 同 `Squad::leader`：写入过（`pm.last()`），当前无读取方；排级查询预留
    pub leader: Option<usize>,
    pub squads: Vec<usize>,
    pub objective: [f32; 2],
}

/// 外部（LLM 指挥官）下发的连长命令覆盖：任务 + 目标点
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CmdOverride {
    pub order: CompanyOrder,
    pub x: f32,
    pub z: f32,
}

pub struct Company {
    pub id: usize,
    pub members: Vec<usize>,
    pub leader: Option<usize>,
    #[allow(dead_code)]
    /// 所属排序号（排 id 列表；内容一直是排 id，2026-08-23 由 squads 更名避免歧义）。
    /// 建编成时写入；当前**无读取方** —— 保留给营/连级"下属有哪些排"的查询（与 `squads` 对称）。
    pub platoon_ids: Vec<usize>,
    pub objective: [f32; 2],
    pub order: CompanyOrder,
    pub report: CompanyReport,
}

pub struct Army {
    pub side: Team,
    pub companies: Vec<Company>,
    pub platoons: Vec<Platoon>,
    pub squads: Vec<Squad>,
    /// npc id → (班id, 班内序号 0=班长)
    pub soldier_slot: std::collections::HashMap<usize, (usize, usize)>,
    pub kills: u32,
    tick: f32,
    situation: BattleSituation,
    /// 敌营重心（由 game.rs 每 tick 写入，供司令参考）
    pub enemy_centroid: [f32; 2],
}

/// 班内阵型槽（相对班目标点的偏移，楔形：班长居中前，双战士后侧）
const FORMATION_OFFSETS: [[f32; 2]; 4] = [
    [0.0, -4.0],  // 班长：前中（箭头）
    [-4.0, 3.0],  // 左翼（品字左侧后）
    [4.0, 3.0],   // 右翼（品字右侧后）
    [0.0, 7.0],   // 殿后（中后，梯形队尾）
];

impl Army {
    /// 按出生顺序把 npc id 编成 3-3-3 营（id 连续 → 位置相邻，天然成组）
    pub fn build(side: Team, ids: &[usize]) -> Army {
        let mut squads: Vec<Squad> = Vec::new();
        let mut platoons: Vec<Platoon> = Vec::new();
        let mut companies: Vec<Company> = Vec::new();
        let mut soldier_slot = std::collections::HashMap::new();
        let mut idx = 0usize;
        let total = ids.len();
        // 排数与连数按三三制展开（人数不足时自然收尾）
        while idx < total {
            let squad_id = squads.len();
            let mut members = Vec::new();
            for k in 0..4 {
                if idx + k < total {
                    let id = ids[idx + k];
                    soldier_slot.insert(id, (squad_id, k));
                    members.push(id);
                }
            }
            idx += members.len();
            squads.push(Squad {
                id: squad_id,
                members: members.clone(),
                leader: members.first().copied(),
                objective: [0.0, 0.0],
                order: CompanyOrder::Assault,
            });
            if squads.len() % 3 == 0 {
                // 排编制完成：排长 = 本排最后一个班的班长（简单确定性指派）
                let p_id = platoons.len();
                let mut pm = Vec::new();
                for s in &squads[squads.len() - 3..] {
                    pm.extend(s.members.iter().copied());
                }
                platoons.push(Platoon {
                    id: p_id,
                    members: pm.clone(),
                    leader: pm.last().copied(),
                    squads: (squads.len() - 3..squads.len()).collect(),
                    objective: [0.0, 0.0],
                });
                if platoons.len() % 3 == 0 {
                    let c_id = companies.len();
                    let mut cm = Vec::new();
                    for p in &platoons[platoons.len() - 3..] {
                        cm.extend(p.members.iter().copied());
                    }
                    companies.push(Company {
                        id: c_id,
                        members: cm.clone(),
                        leader: cm.last().copied(),
                        platoon_ids: (platoons.len() - 3..platoons.len()).collect(),
                        objective: [0.0, 0.0],
                        order: CompanyOrder::Assault,
                        report: CompanyReport {
                            strength: 0.0,
                            centroid: [0.0, 0.0],
                            contact: false,
                        },
                    });
                }
            }
        }
        // 收尾：编制尾数并入末位（末排承接余班、末连承接余排）—— 2026-09-26 复查修。
        //
        // 上面的循环只在「刚好凑满 3 个」的那一刻建排/建连 ⇒ 末尾凑不满 3 个排的那一截会留在
        // 连名单之外，而军情 `CompanyReport` 是**按连的 `members` 汇总**的 ⇒ **128 人的营只上报
        // 108 人、64 人只上报 36 人**：那一截的兵力与伤亡对营司令完全隐形（行军目标另有兜底，
        // 所以"看着还在动"，缺陷不会自己暴露）。判据 = `every_soldier_is_carried_by_a_company`。
        //
        // ⚠️ 一个连都没编成（不足 33 人 = 9 个班 = 3 个排）时**保持原样**：`update` 的
        // `companies.is_empty()` 会跳过整个指挥层，不许凭空造连去改小规模战斗的行为。
        if !companies.is_empty() {
            // 1) 余班并入末排（末排承接余班）
            let last_p = platoons.len() - 1;
            for s in &squads[platoons.len() * 3..] {
                platoons[last_p].squads.push(s.id);
                platoons[last_p].members.extend(s.members.iter().copied());
            }
            // 2) 按 `platoon_company` 的同一口径重建连名单（末连承接余排）
            //    ⚠️ `Company::members` 是建连那一刻复制的**快照** ⇒ 只把余班补进末排而不重建连名单，
            //    「只差 1 人凑不满 3 个排」的营仍然漏报（实测 n=37 依旧红：连 0 只认 36 人）。
            let owner: Vec<usize> = platoons
                .iter()
                .map(|p| platoon_company(&companies, p.id))
                .collect();
            for c in companies.iter_mut() {
                c.platoon_ids.clear();
                c.members.clear();
            }
            for (i, p) in platoons.iter().enumerate() {
                companies[owner[i]].platoon_ids.push(p.id);
                companies[owner[i]].members.extend(p.members.iter().copied());
            }
        }
        Army {
            side,
            kills: 0,
            companies,
            platoons,
            squads,
            soldier_slot,
            tick: 0.0,
            situation: BattleSituation::Offense,
            enemy_centroid: [0.0, 0.0],
        }
    }

    /// 该 NPC 所在班

    /// 该 NPC 是否班长
    pub fn is_leader(&self, npc_id: usize) -> bool {
        self.soldier_slot
            .get(&npc_id)
            .map(|(_s, k)| *k == 0)
            .unwrap_or(false)
    }

    /// 全营存活数

    /// 指挥节拍：每 0.5s 重新评估军情并下发命令（由 game.rs 在 update_ai 内调用）
    pub fn update(
        &mut self,
        npcs: &[crate::engine::game::Npc],
        _grid: &GridMap,
        dt: f32,
        kills: u32,
        enemy_centroid: [f32; 2],
        llm: Option<&[CmdOverride]>,
    ) {
        self.tick += dt;
        self.kills = kills;
        self.enemy_centroid = enemy_centroid;
        // 不足 33 人（9 个班 = 3 个排）连未编成：跳过指挥层（逐人战术照常）
        if self.companies.is_empty() {
            return;
        }
        if self.tick < 0.5 {
            return;
        }
        self.tick = 0.0;

        // 1) 逐连自下而上汇总报告（战士 → 班 → 排 → 连）
        let alive: Vec<(usize, [f32; 3])> = npcs
            .iter()
            .filter(|n| n.team == self.side)
            .map(|n| (n.id, n.position))
            .collect();
        let mut company_reports = Vec::with_capacity(self.companies.len());
        for c in &self.companies {
            let mut sum = [0.0f32, 0.0f32];
            let mut n = 0usize;
            let mut contact = false;
            for (id, pos) in &alive {
                if c.leader == Some(*id) || c.members.iter().any(|m| m == id) {
                    sum[0] += pos[0];
                    sum[1] += pos[2];
                    n += 1;
                    if matches!(npc_state(npcs, *id), StateKind::Combat) {
                        contact = true;
                    }
                }
            }
            let centroid = if n > 0 {
                [sum[0] / n as f32, sum[1] / n as f32]
            } else {
                [0.0, 0.0]
            };
            company_reports.push(CompanyReport {
                strength: n as f32,
                centroid,
                contact,
            });
        }

        // 2) 营司令决策：按本轮汇总选择态势
        let own_advance = company_reports.iter().map(|r| r.strength).sum::<f32>();
        let my_c = self
            .companies
            .first()
            .map(|_| company_reports.iter().map(|r| r.centroid).fold([0.0, 0.0], |a, b| [a[0] + b[0], a[1] + b[1]]))
            .unwrap_or([0.0, 0.0]);
        let my_c = [my_c[0] / self.companies.len().max(1) as f32, my_c[1] / self.companies.len().max(1) as f32];
        let d_self = (my_c[0] * my_c[0] + my_c[1] * my_c[1]).sqrt();
        let d_enemy = (self.enemy_centroid[0] * self.enemy_centroid[0]
            + self.enemy_centroid[1] * self.enemy_centroid[1])
            .sqrt();
        // 我方重心比敌方更靠近地图中心 → 我方压上；反之敌方前推 → 防御；伤亡 >45% → 重组
        self.situation = decide_situation(
            own_advance,
            self.roster_size() as f32,
            self.kills,
            d_self,
            d_enemy,
        );
        // 目标线：敌我重心连线中点附近（营命令基准）
        let mid = [
            (my_c[0] + self.enemy_centroid[0]) * 0.5,
            (my_c[1] + self.enemy_centroid[1]) * 0.5,
        ];
        // 3) 逐连下发：命令 + 目标点（侧翼命令取垂直方向偏移）
        //    LLM 指挥官覆盖：数量匹配时直接采用外部命令（目标点越界由 llm_cmd 校验过）
        let company_count = self.companies.len();
        let llm_ok = llm.map(|o| o.len() == company_count).unwrap_or(false);
        for (ci, c) in self.companies.iter_mut().enumerate() {
            if llm_ok {
                let o = &llm.unwrap()[ci];
                c.order = o.order;
                c.objective = [o.x.clamp(-280.0, 280.0), o.z.clamp(-280.0, 280.0)];
                c.report = company_reports.get(ci).copied().unwrap_or(CompanyReport {
                    strength: 0.0,
                    centroid: [0.0, 0.0],
                    contact: false,
                });
                continue;
            }
            let order = match self.situation {
                BattleSituation::Offense => CompanyOrder::Assault,
                BattleSituation::Defend => CompanyOrder::Hold,
                BattleSituation::Regroup => CompanyOrder::Regroup,
                BattleSituation::Pincer => {
                    // 双连钳形：左连左翼、右连右翼（id 奇偶定左右）；其余连正面牵制
                    if company_count >= 2 && ci < 2 {
                        CompanyOrder::Flank(if ci % 2 == 0 { 1 } else { -1 })
                    } else {
                        CompanyOrder::Assault
                    }
                }
            };
            c.order = order;
            // 侧翼点：目标线沿垂直方向外推 40m
            let dx = self.enemy_centroid[0] - my_c[0];
            let dz = self.enemy_centroid[1] - my_c[1];
            let dl = (dx * dx + dz * dz).sqrt().max(1.0);
            let (px, pz) = (-dz / dl, dx / dl); // 垂直
            let flank_off = match order {
                CompanyOrder::Flank(s) => 40.0 * s as f32,
                _ => 0.0,
            };
            // 距敌保持 25m（防御/重组保持更远）
            let keep = match order {
                CompanyOrder::Regroup => 90.0,
                CompanyOrder::Hold => 60.0,
                _ => 25.0,
            };
            let en = self.enemy_centroid;
            let el = (en[0] * en[0] + en[1] * en[1]).sqrt().max(1.0);
            let back = match order {
                CompanyOrder::Regroup | CompanyOrder::Hold => -1.0,
                _ => 1.0,
            };
            // 战位铺开（2026-08-26 防排队站一排）：正面连也沿垂直方向横向散开——
            // 连索引依序 -55/0/+55 米（左中右战位），侧翼连再外推；三连目标线不再汇成一条线
            let spread = if company_count >= 3 {
                (ci as f32 - (company_count as f32 - 1.0) / 2.0) * 55.0
            } else {
                0.0
            };
            c.objective = [
                (mid[0] - en[0] / el * keep * back).max(-280.0).min(280.0) + px * flank_off
                    + px * spread,
                (mid[1] - en[1] / el * keep * back).max(-280.0).min(280.0) + pz * flank_off
                    + pz * spread,
            ];
            c.report = company_reports.get(ci).copied().unwrap_or(CompanyReport {
                strength: 0.0,
                centroid: [0.0, 0.0],
                contact: false,
            });
        }
        // 4) 逐排 → 逐班：目标点 = 连目标点 + 班槽位偏移；刷新每个 soldier 的队列目标
        for p in &mut self.platoons {
            let c = &self.companies[platoon_company(&self.companies, p.id)];
            p.objective = c.objective;
        }
        for s in &mut self.squads {
            let p = &self.platoons[platoon_of_squad(&self.platoons, s.id)];
            let c = &self.companies[platoon_company(&self.companies, p.id)];
            s.objective = p.objective;
            s.order = c.order;
        }
    }

    /// 战士的排位目标点（未接敌时编队推进用；接敌后由 game.rs 既有战术接管）
    pub fn squad_waypoint(&self, npc_id: usize) -> Option<[f32; 2]> {
        let (squad_id, slot) = self.soldier_slot.get(&npc_id)?;
        let squad = &self.squads.get(*squad_id)?;
        if matches!(squad.order, CompanyOrder::Regroup) {
            return Some(squad.objective);
        }
        let off = FORMATION_OFFSETS[*slot % FORMATION_OFFSETS.len()];
        // 以班目标点为锚，班内槽位偏移（不旋转——大战场上可接受并保持确定）
        let mut wp = [squad.objective[0] + off[0], squad.objective[1] + off[1]];
        wp[0] = wp[0].clamp(-270.0, 270.0);
        wp[1] = wp[1].clamp(-270.0, 270.0);
        Some(wp)
    }

    /// 军情摘要（观察日志，10s 一条）
    pub fn summary(&self) -> String {
        let cs: Vec<String> = self
            .companies
            .iter()
            .map(|c| {
                format!(
                    "连{}[{} 强度{} 位({:.0},{:.0}) {}]",
                    c.id,
                    c.order.label(),
                    c.report.strength as i32,
                    c.report.centroid[0],
                    c.report.centroid[1],
                    if c.report.contact { "接敌" } else { "未接敌" }
                )
            })
            .collect();
        format!(
            "营[态势{:?} 击杀{}] {}",
            self.situation,
            self.kills,
            cs.join(" ")
        )
    }
}

fn npc_state(npcs: &[crate::engine::game::Npc], id: usize) -> StateKind {
    match npcs.iter().find(|n| n.id == id).map(|n| n.state_machine.state()) {
        Some(crate::engine::ai::NpcState::Attack) | Some(crate::engine::ai::NpcState::Chase) => {
            StateKind::Combat
        }
        Some(_) => StateKind::Patrol,
        None => StateKind::Dead,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateKind {
    Combat,
    Patrol,
    Dead,
}

fn platoon_company(companies: &[Company], platoon_id: usize) -> usize {
    // 每连容纳 3 排（三三制）：排序整除得连序；末连承接余排
    (platoon_id / 3).min(companies.len().saturating_sub(1))
}

impl Army {
    /// 该排归哪个连（与 `platoon_company` 同一个真源）。调用方只有判据测试 ——
    /// `update` 里那条路要同时可变借用 `companies`，只能直调同名自由函数。
    #[allow(dead_code)] // 使用者全在 `#[cfg(test)]` ⇒ 非 test 构建看不见（2026-09-26：`cargo build` 报过 never used，`cargo test` 不报）
    pub fn company_of_platoon(&self, platoon_id: usize) -> usize {
        platoon_company(&self.companies, platoon_id)
    }

    /// 该班归哪个排（余数班归**末排**，与「末连承接余排」同一约定）。同上的测试入口。
    #[allow(dead_code)] // 同 `company_of_platoon`：只在判据测试里被调用
    pub fn platoon_of_squad(&self, squad_id: usize) -> usize {
        platoon_of_squad(&self.platoons, squad_id)
    }

    /// 全营编制人数 = 各班名单之和（军情伤亡比例的**分母**）。
    ///
    /// 🔴 这个数以前在 `update` 里被写死成 `128.0`：`RV3D_STRESS_AI=64` 时"存活 64 人"恒
    /// `< 128×0.55 = 70.4` ⇒ **司令永远只会重组**（`Regroup` 保持 90m 距离、不再压上），
    /// 小规模压力模式的两军于是永远打不起来。判据 = `regroup_threshold_follows_the_roster`。
    pub fn roster_size(&self) -> usize {
        self.squads.iter().map(|s| s.members.len()).sum()
    }
}

/// 营司令的态势判定（纯函数：阈值可逐档验证，不必造 NPC）。
///
/// 阈值口径：**连名单内存活 < 本营编制 × 0.55 且累计击杀 < 8 → 重组**；否则比敌我重心到
/// 地图中心的距离（我方 < 0.8×敌 ⇒ 进攻；> 1.25×敌 ⇒ 防御；其余 ⇒ 钳形侧翼）。
fn decide_situation(
    alive: f32,
    roster: f32,
    kills: u32,
    d_self: f32,
    d_enemy: f32,
) -> BattleSituation {
    if roster > 0.0 && alive < roster * 0.55 && kills < 8 {
        BattleSituation::Regroup
    } else if d_self < d_enemy * 0.8 {
        BattleSituation::Offense
    } else if d_self > d_enemy * 1.25 {
        BattleSituation::Defend
    } else {
        BattleSituation::Pincer
    }
}

fn platoon_of_squad(platoons: &[Platoon], squad_id: usize) -> usize {
    for (pi, p) in platoons.iter().enumerate() {
        if p.squads.contains(&squad_id) {
            return pi;
        }
    }
    // 🔴 2026-09-26 复查修：这里以前回落 **0（首排）** —— 而 `platoon_company` 的余数约定是
    // 「归末位」（`.min(len-1)`）。两处不一致的后果：128 人的营有 2 个班（8 人）落在这个兜底上，
    // 它们会被指到**首排** ⇒ 跟着最左侧的连（`spread` 里 ci=0 那条，横向 −55m）跑，而不是
    // 跟着编号相邻、出生点也相邻的末排。判据 = `organization_tail_follows_the_last_platoon`。
    platoons.len().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn army_of(n: usize) -> Army {
        let ids: Vec<usize> = (0..n).collect();
        Army::build(Team::Red, &ids)
    }

    /// 判据：**余数一律归末位**（末连承接余排、末排承接余班）——两处兜底必须是同一个约定。
    ///
    /// 128 人的营实测编成 = **3 连 / 10 排 / 32 班**：末连（连 2）承接第 10 排、末排（排 9）
    /// 承接末尾 2 个班 ⇒ **全营 128 人逐人都在连名单里**。2026-09-26 前的写法只认「刚好凑满
    /// 3 个」，尾数（20 人）落在连名单外 ⇒ 军情漏报（判据见 `every_soldier_is_carried_by_a_company`）。
    #[test]
    fn organization_tail_follows_the_last_platoon() {
        let a = army_of(128);
        assert_eq!(a.companies.len(), 3, "128 人 = 3 个连（末连吃尾数）");
        assert_eq!(a.platoons.len(), 10, "每 3 个班成 1 排 ⇒ 30 个班 10 排，余 2 班");
        assert_eq!(a.squads.len(), 32, "每班 4 人 ⇒ 128/4");

        let in_companies: usize = a.companies.iter().map(|c| c.members.len()).sum();
        assert_eq!(in_companies, 128, "连名单必须覆盖全营，漏一个就是漏一份兵力");
        assert_eq!(
            a.companies[2].platoon_ids,
            vec![6, 7, 8, 9],
            "末连 = 3 个满排 + 第 10 排"
        );
        assert_eq!(
            a.platoons[9].squads,
            vec![27, 28, 29, 30, 31],
            "末排 = 第 30~32 个班（后两个是余数班）"
        );

        // 末连承接余排（第 10 排 ⇒ 连 2）
        assert_eq!(a.company_of_platoon(9), 2, "第 10 排（下标 9）归末连");
        assert_eq!(a.company_of_platoon(10), 2, "同理");
        // 末排承接余班（第 32 个班 ⇒ 排 9）—— 这就是上一轮修的那处
        assert_eq!(
            a.platoon_of_squad(31),
            9,
            "余数班必须跟着编号相邻的末排，而不是首排"
        );
        assert_eq!(a.platoon_of_squad(0), 0, "正常班仍按所属排走");
        assert_eq!(a.platoon_of_squad(29), 9, "第 30 个班属于第 10 排");
    }

    /// 判据：编成必须**逐字**符合文档写的那条规则（每班 ≤4 人、每 3 班成排、每 3 排成连），
    /// 且任何人数下兜底都不下溢 —— 用「模型」算期望值比写死几个数字更能防走样：
    /// 128 人 ⇒ 32 班 / 10 排 / 3 连；33 人才刚够 9 个班 ⇒ 3 排 ⇒ 1 连（**门槛是 33 不是 36**，
    /// 因为最后一个班可以是 1 人的残班）。
    #[test]
    fn organization_matches_the_documented_three_three_rule() {
        for n in 0..=200usize {
            let a = army_of(n);
            let squads = n.div_ceil(4);
            let platoons = squads / 3;
            let companies = platoons / 3;
            assert_eq!(a.squads.len(), squads, "n={n}：每班 ≤4 人");
            assert_eq!(a.platoons.len(), platoons, "n={n}：每 3 班成 1 排");
            assert_eq!(a.companies.len(), companies, "n={n}：每 3 排成 1 连");
            assert_eq!(a.soldier_slot.len(), n, "每个士兵都要在编制表里，n={n}");
            assert_eq!(a.platoon_of_squad(0), 0, "没有排时兜底返回 0，不许下溢，n={n}");
            assert_eq!(a.company_of_platoon(0), 0, "没有连时兜底返回 0，不许下溢，n={n}");
        }
    }

    /// 判据：**每个士兵都必须进一个连**，且排/班/连三级名单逐人等于 `0..n`（不重不漏）。
    ///
    /// 🔴 2026-09-26 实测的红：连只在「排数刚好 % 3 == 0」那一刻成立 ⇒ 末尾凑不满 3 个排的那一截
    /// 落在连名单之外。而军情 `CompanyReport` 是**按连的 `members` 汇总**的 ⇒
    /// **128 人的营只上报 108 人、64 人只上报 36 人**，司令看到的是"编制外用不存在的部队"，
    /// 尾数那一截（含他们的伤亡）对它完全隐形。判据就是这条名单相等。
    #[test]
    fn every_soldier_is_carried_by_a_company() {
        let all = |n: usize| (0..n).collect::<Vec<usize>>();
        for n in 1..=200usize {
            let a = army_of(n);
            if a.companies.is_empty() {
                // 不足 33 人（9 个班）编不成连 ⇒ `update` 跳过指挥层，逐人战术照常。
                assert!(n <= 32, "n={n}：33 人起（9 个班 = 3 个排）就必须编出连");
                continue;
            }
            let mut carried: Vec<usize> = a
                .companies
                .iter()
                .flat_map(|c| c.members.iter().copied())
                .collect();
            carried.sort_unstable();
            assert_eq!(
                carried,
                all(n),
                "n={n}：连名单漏人或重复 ⇒ 军情的强度/重心把这些人当空气"
            );
            for (level, got) in [
                (
                    "排",
                    a.platoons
                        .iter()
                        .flat_map(|p| p.members.iter().copied())
                        .collect::<Vec<usize>>(),
                ),
                (
                    "班",
                    a.squads
                        .iter()
                        .flat_map(|s| s.members.iter().copied())
                        .collect::<Vec<usize>>(),
                ),
            ] {
                let mut got = got;
                got.sort_unstable();
                assert_eq!(got, all(n), "n={n}：{level}名单漏人");
            }
            // 索引安全：`update` 里是直接下标，越界即 panic
            for s in 0..a.squads.len() {
                assert!(
                    a.platoon_of_squad(s) < a.platoons.len(),
                    "n={n}：班 {s} 的排下标越界（update 会 panic）"
                );
            }
            for p in 0..a.platoons.len() {
                assert!(
                    a.company_of_platoon(p) < a.companies.len(),
                    "n={n}：排 {p} 的连下标越界"
                );
            }
        }
    }

    /// 判据：重组阈值必须按**本营实际编制**算，不许写死 128。
    ///
    /// 写死 128 的后果（2026-09-26 修）：`RV3D_STRESS_AI=64` 时"存活 64 人"恒
    /// `< 128×0.55 = 70.4` ⇒ 司令**永远只会重组**（保持 90m 距离、不再压上）⇒ 小规模压力模式
    /// 的两军永远不接火。
    #[test]
    fn regroup_threshold_follows_the_roster() {
        // 128 人的营：重组线 = 70.4 人（刚好跨过阈值的两档都给）
        assert_eq!(
            decide_situation(70.0, 128.0, 0, 10.0, 10.0),
            BattleSituation::Regroup
        );
        assert_eq!(
            decide_situation(71.0, 128.0, 0, 10.0, 10.0),
            BattleSituation::Pincer
        );
        // 64 人的营：重组线 = 35.2 人 —— 写死 128 时这两档都恒为 Regroup
        assert_eq!(
            decide_situation(36.0, 64.0, 0, 10.0, 10.0),
            BattleSituation::Pincer
        );
        assert_eq!(
            decide_situation(35.0, 64.0, 0, 10.0, 10.0),
            BattleSituation::Regroup
        );
        // 击杀 ≥ 8 之后不再重组（原有口径未变）
        assert_eq!(
            decide_situation(10.0, 128.0, 8, 10.0, 10.0),
            BattleSituation::Pincer
        );
        // 重心判据两侧都取"刚好跨过"的输入（教训 42）
        assert_eq!(
            decide_situation(128.0, 128.0, 0, 7.9, 10.0),
            BattleSituation::Offense
        );
        assert_eq!(
            decide_situation(128.0, 128.0, 0, 12.6, 10.0),
            BattleSituation::Defend
        );
        assert_eq!(
            decide_situation(128.0, 128.0, 0, 10.0, 10.0),
            BattleSituation::Pincer
        );
    }

    /// 判据：`roster_size()` = 军情比例的分母，必须等于全营实际人数，且与连名单闭合。
    #[test]
    fn roster_size_covers_every_built_soldier() {
        for n in [1usize, 32, 33, 37, 64, 100, 128, 129, 200] {
            let a = army_of(n);
            assert_eq!(a.roster_size(), n, "n={n}：编制人数必须等于实际人数");
            if !a.companies.is_empty() {
                assert_eq!(
                    a.companies
                        .iter()
                        .map(|c| c.members.len())
                        .sum::<usize>(),
                    n,
                    "n={n}：连名单合计必须闭合到全营（尾数不许落在编制外）"
                );
            }
        }
    }
}
