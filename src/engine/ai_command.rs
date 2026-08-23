//! 连排班指挥体系（三三制）：营 → 连 → 排 → 班 → 战士。
//!
//! 编制（每营 128 人，精确三三制）：
//! - 班 = 3 战士 + 班长（4 人）；排 = 3 班 + 排长（13）；连 = 3 排 + 连长/副官（41）；
//! - 营 = 3 连 + 营部 5 人（128）。红蓝各一营（128v128 战场）。
//!
//! 指挥链：战士向班长汇报（状态/位置），班→排→连→营逐级汇总为「军情报告」；
//! 营司令官（AI 司令）按战场形势（前线推进度/兵力/伤亡）每 0.5s 做出战役决策：
//! - 进攻：全线前推至接触线；- 防御：占领线保持，伤亡>40% 时转入；
//! - 侧翼：连队向敌阵侧后迂回包抄（钳形）；- 重组：班退回连部方向整补。
//! 命令逐级下发为「班目标点」，未接敌的战士按班目标编队推进（班长居中、
//! 左右战士侧后 3m 楔形）；接敌后仍交由既有逐人战术（掩体/侧翼/偷袭/压制）。
//!
//! 同时提供：连长对班长投掷压制的指挥权加成（压制翻倍）、班长大致向敌方向
//! 投掷的信息（投掷逻辑在 game.rs，此处仅编制/目标/报告）。

use crate::engine::ai::{GridMap, Team};

/// 连队军情报告（自下而上汇总：营司令据此决策）
#[derive(Debug, Clone, Copy)]
pub struct CompanyReport {
    pub strength: f32,
    pub centroid: [f32; 2],
    pub contact: bool,
    #[allow(dead_code)]
    pub kills: u32,
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

/// 班目标点 + 班内阵型槽（班长居中，战士侧后楔形）
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct SquadOrder {
    pub objective: [f32; 2],
    pub order: CompanyOrder,
}

pub struct Squad {
    pub id: usize,
    pub members: Vec<usize>,
    #[allow(dead_code)]
    pub leader: Option<usize>,
    pub objective: [f32; 2],
    pub order: CompanyOrder,
}

pub struct Platoon {
    pub id: usize,
    pub members: Vec<usize>,
    #[allow(dead_code)]
    pub leader: Option<usize>,
    pub squads: Vec<usize>,
    pub objective: [f32; 2],
}

pub struct Company {
    pub id: usize,
    pub members: Vec<usize>,
    pub leader: Option<usize>,
    #[allow(dead_code)]
    pub squads: Vec<usize>,
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
    enemy_centroid: [f32; 2],
}

/// 班内阵型槽（相对班目标点的偏移，楔形：班长居中前，双战士后侧）
const FORMATION_OFFSETS: [[f32; 2]; 4] = [
    [0.0, 0.0],   // 班长：居中
    [-3.0, 3.0],  // 左翼
    [3.0, 3.0],   // 右翼
    [0.0, 5.0],   // 殿后
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
                        squads: (platoons.len() - 3..platoons.len()).collect(),
                        objective: [0.0, 0.0],
                        order: CompanyOrder::Assault,
                        report: CompanyReport {
                            strength: 0.0,
                            centroid: [0.0, 0.0],
                            contact: false,
                            kills: 0,
                        },
                    });
                }
            }
        }
        Army {
            side,
            companies,
            platoons,
            squads,
            soldier_slot,
            kills: 0,
            tick: 0.0,
            situation: BattleSituation::Offense,
            enemy_centroid: [0.0, 0.0],
        }
    }

    /// 该 NPC 所在班
    #[allow(dead_code)]
    pub fn squad_of(&self, npc_id: usize) -> Option<usize> {
        self.soldier_slot.get(&npc_id).map(|(s, _)| *s)
    }

    /// 该 NPC 是否班长
    pub fn is_leader(&self, npc_id: usize) -> bool {
        self.soldier_slot
            .get(&npc_id)
            .map(|(_s, k)| *k == 0)
            .unwrap_or(false)
    }

    /// 全营存活数
    #[allow(dead_code)]
    pub fn strength(&self, npcs: &[crate::engine::game::Npc], side: Team) -> f32 {
        npcs.iter()
            .filter(|n| n.team == side)
            .count() as f32
    }

    /// 指挥节拍：每 0.5s 重新评估军情并下发命令（由 game.rs 在 update_ai 内调用）
    pub fn update(
        &mut self,
        npcs: &[crate::engine::game::Npc],
        _grid: &GridMap,
        dt: f32,
        kills: u32,
        enemy_centroid: [f32; 2],
    ) {
        self.tick += dt;
        self.kills = kills;
        self.enemy_centroid = enemy_centroid;
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
                kills: 0,
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
        // 我方重心比敌方更靠近地图中心 → 我方压上；反之敌方前推 → 防御；伤亡>40% → 重组
        let total = 128.0f32;
        self.situation = if own_advance < total * 0.55 && self.kills < 8 {
            BattleSituation::Regroup
        } else if d_self < d_enemy * 0.8 {
            BattleSituation::Offense
        } else if d_self > d_enemy * 1.25 {
            BattleSituation::Defend
        } else {
            BattleSituation::Pincer
        };
        // 目标线：敌我重心连线中点附近（营命令基准）
        let mid = [
            (my_c[0] + self.enemy_centroid[0]) * 0.5,
            (my_c[1] + self.enemy_centroid[1]) * 0.5,
        ];
        // 3) 逐连下发：命令 + 目标点（侧翼命令取垂直方向偏移）
        let company_count = self.companies.len();
        for (ci, c) in self.companies.iter_mut().enumerate() {
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
            c.objective = [
                (mid[0] - en[0] / el * keep * back).max(-280.0).min(280.0) + px * flank_off,
                (mid[1] - en[1] / el * keep * back).max(-280.0).min(280.0) + pz * flank_off,
            ];
            c.report = company_reports.get(ci).copied().unwrap_or(CompanyReport {
                strength: 0.0,
                centroid: [0.0, 0.0],
                contact: false,
                kills: 0,
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
    pub fn squad_waypoint(&self, npc_id: usize, npc_pos: [f32; 3]) -> Option<[f32; 2]> {
        let (squad_id, slot) = self.soldier_slot.get(&npc_id)?;
        let squad = &self.squads.get(*squad_id)?;
        if matches!(squad.order, CompanyOrder::Regroup) {
            return Some(squad.objective);
        }
        let off = FORMATION_OFFSETS[*slot % FORMATION_OFFSETS.len()];
        // 以班目标点为锚，班内槽位偏移（不旋转——大战场上可接受并保持确定）
        let mut wp = [squad.objective[0] + off[0], squad.objective[1] + off[1]];
        let _ = npc_pos;
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

fn platoon_of_squad(platoons: &[Platoon], squad_id: usize) -> usize {
    for (pi, p) in platoons.iter().enumerate() {
        if p.squads.contains(&squad_id) {
            return pi;
        }
    }
    0
}
