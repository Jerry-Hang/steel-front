//! AI 寻路与战术决策系统模块
//!
//! - 网格地图 A* 寻路（2D grid，可通行/阻挡）
//! - NPC 状态机：Idle → Patrol → Chase → Attack（含状态转换条件）
//! - 战术层：角色分工（突击/包抄/压制/掩体跃进）、战术决策（推进/侧翼/偷袭/撤退/站定）、
//!   多 AI 协同（同步冲锋、左右包抄分工）、躲避机动（锯齿推进/受击侧向弹开/火力威胁感知）
//! - 掩体点搜索（含遮挡掩体）/ 包抄目标点 / 偷袭绕背目标点 / 波次难度曲线
//! - 特殊波次：每 5 波 Boss 主怪、每 3 波援军补怪（wave_kind / boss_profile / wave_profile）
//!
//! 本模块仅依赖 std；`game.rs` 每帧按「感知填充 → 状态机 → 战术决策 → `advance_npc` 推进」接线。

#![allow(dead_code)]

use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// 阵营：普通波次模式 NPC 全部为 Red（目标=玩家）；压力模式 64v64 红蓝对抗，
/// NPC 以敌对阵营 NPC 为优先目标（玩家为兜底目标）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Team {
    Red,
    Blue,
}

impl Team {
    /// 敌对阵营
    pub const fn opposite(self) -> Team {
        match self {
            Team::Red => Team::Blue,
            Team::Blue => Team::Red,
        }
    }
}

/// 网格坐标（x 为列，y 为行）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridPos {
    pub x: i32,
    pub y: i32,
}

impl GridPos {
    /// 新建网格坐标
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// 曼哈顿距离（四方向移动的 A* 启发函数，可采纳）
    pub fn manhattan(self, other: Self) -> u32 {
        (self.x - other.x).unsigned_abs() + (self.y - other.y).unsigned_abs()
    }
}

/// 2D 网格地图：记录每格是否阻挡
#[derive(Debug, Clone)]
pub struct GridMap {
    width: usize,
    height: usize,
    blocked: Vec<bool>,
}

impl GridMap {
    /// 新建全可通行地图
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            blocked: vec![false; width * height],
        }
    }

    /// 地图宽度（列数）
    pub fn width(&self) -> usize {
        self.width
    }

    /// 地图高度（行数）
    pub fn height(&self) -> usize {
        self.height
    }

    /// 坐标是否在地图范围内
    pub fn in_bounds(&self, pos: GridPos) -> bool {
        pos.x >= 0 && pos.y >= 0 && (pos.x as usize) < self.width && (pos.y as usize) < self.height
    }

    /// 该格是否可通行（在地图内且未被阻挡）
    pub fn is_passable(&self, pos: GridPos) -> bool {
        self.in_bounds(pos) && !self.blocked[self.index(pos)]
    }

    /// 设置某格阻挡状态
    pub fn set_blocked(&mut self, pos: GridPos, blocked: bool) {
        if self.in_bounds(pos) {
            let i = self.index(pos);
            self.blocked[i] = blocked;
        }
    }

    /// 将某格标记为阻挡
    pub fn block(&mut self, pos: GridPos) {
        self.set_blocked(pos, true);
    }

    /// 清除某格阻挡
    pub fn clear(&mut self, pos: GridPos) {
        self.set_blocked(pos, false);
    }

    /// 行主序索引
    fn index(&self, pos: GridPos) -> usize {
        pos.y as usize * self.width + pos.x as usize
    }
}

/// 四方向邻居偏移（右、左、下、上）
const NEIGHBOR_OFFSETS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// A* 开放列表节点
#[derive(Debug, Clone, Copy)]
struct HeapNode {
    /// 估价 f = g + h
    f: u32,
    /// 起点到该格的实际代价
    g: u32,
    /// 行主序网格索引
    index: usize,
}

impl PartialEq for HeapNode {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f && self.index == other.index
    }
}

impl Eq for HeapNode {}

impl PartialOrd for HeapNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// BinaryHeap 是最大堆，反向比较实现 f 小者优先；
/// f 相同时 g 大者（更接近终点）优先，索引兜底保证确定性。
impl Ord for HeapNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f
            .cmp(&self.f)
            .then_with(|| self.g.cmp(&other.g))
            .then_with(|| other.index.cmp(&self.index))
    }
}

/// A* 寻路：求 `start` 到 `goal` 的最短四方向路径（含两端点）。
///
/// - 自动绕过阻挡格（阻挡格不可进入）
/// - 起点/终点越界或为阻挡格时返回 `None`
/// - 无可通行路径时返回 `None`
pub fn find_path(map: &GridMap, start: GridPos, goal: GridPos) -> Option<Vec<GridPos>> {
    if !map.is_passable(start) || !map.is_passable(goal) {
        return None;
    }
    if start == goal {
        return Some(vec![start]);
    }

    let width = map.width();
    let cell_count = width * map.height();
    let start_idx = map.index(start);
    let goal_idx = map.index(goal);

    let mut g_score = vec![u32::MAX; cell_count];
    let mut parent: Vec<Option<usize>> = vec![None; cell_count];
    let mut closed = vec![false; cell_count];
    g_score[start_idx] = 0;

    let mut open = BinaryHeap::new();
    open.push(HeapNode {
        f: start.manhattan(goal),
        g: 0,
        index: start_idx,
    });

    while let Some(node) = open.pop() {
        if closed[node.index] {
            continue;
        }
        closed[node.index] = true;

        if node.index == goal_idx {
            let mut path = Vec::new();
            let mut cur = Some(node.index);
            while let Some(i) = cur {
                path.push(GridPos::new((i % width) as i32, (i / width) as i32));
                cur = parent[i];
            }
            path.reverse();
            return Some(path);
        }

        let cur_pos = GridPos::new((node.index % width) as i32, (node.index / width) as i32);
        for (dx, dy) in NEIGHBOR_OFFSETS {
            let next = GridPos::new(cur_pos.x + dx, cur_pos.y + dy);
            if !map.is_passable(next) {
                continue;
            }
            let next_idx = map.index(next);
            if closed[next_idx] {
                continue;
            }
            let tentative_g = node.g + 1;
            if tentative_g >= g_score[next_idx] {
                continue;
            }
            g_score[next_idx] = tentative_g;
            parent[next_idx] = Some(node.index);
            open.push(HeapNode {
                f: tentative_g + next.manhattan(goal),
                g: tentative_g,
                index: next_idx,
            });
        }
    }

    None
}

/// NPC 状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpcState {
    /// 待机：无目标，原地警戒
    Idle,
    /// 巡逻：沿巡逻路线移动
    Patrol,
    /// 追击：发现敌人，向敌人移动
    Chase,
    /// 攻击：敌人在攻击距离内
    Attack,
}

/// 状态机感知输入（由 AI 感知层每帧填充）
///
/// 前 4 项为状态机转换条件；后 5 项为战术决策与躲避机动的输入。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NpcPerception {
    /// 视野内是否存在敌人
    pub enemy_visible: bool,
    /// 敌人是否在攻击距离内
    pub enemy_in_range: bool,
    /// 是否开始巡逻（有待巡逻路线）
    pub start_patrol: bool,
    /// 巡逻路线是否完成
    pub patrol_finished: bool,
    /// 玩家准星是否大致对准本 NPC（水平夹角 < 约 14°）
    pub player_aiming: bool,
    /// 玩家是否面朝本 NPC（水平夹角 < 90°；false 时包抄手可偷袭绕背）
    pub player_facing: bool,
    /// 本帧受击（血量较上一帧下降）
    pub took_hit: bool,
    /// 低血量（hp < 35% 上限）
    pub low_hp: bool,
    /// 有子弹正朝本 NPC 接近（火力威胁，供移动态躲避）
    pub under_fire: bool,
}

/// NPC 状态机（Idle → Patrol → Chase → Attack）
///
/// 转换条件：
/// - `Idle → Patrol`：`start_patrol`
/// - `Idle/Patrol → Chase`：发现敌人 `enemy_visible`
/// - `Chase → Attack`：发现敌人且在攻击距离内 `enemy_in_range`
/// - `Attack → Chase`：敌人仍在视野但脱离攻击距离
/// - `Patrol/Chase/Attack → Idle`：巡逻完成 / 丢失敌人
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcStateMachine {
    state: NpcState,
}

impl NpcStateMachine {
    /// 新建状态机，初始为 `Idle`
    pub fn new() -> Self {
        Self { state: NpcState::Idle }
    }

    /// 当前状态
    pub fn state(&self) -> NpcState {
        self.state
    }

    /// 根据感知输入推进状态机，返回转换后的状态
    pub fn update(&mut self, perception: NpcPerception) -> NpcState {
        self.state = match self.state {
            NpcState::Idle => {
                if perception.enemy_visible {
                    NpcState::Chase
                } else if perception.start_patrol {
                    NpcState::Patrol
                } else {
                    NpcState::Idle
                }
            }
            NpcState::Patrol => {
                if perception.enemy_visible {
                    NpcState::Chase
                } else if perception.patrol_finished {
                    NpcState::Idle
                } else {
                    NpcState::Patrol
                }
            }
            NpcState::Chase => {
                if !perception.enemy_visible {
                    NpcState::Idle
                } else if perception.enemy_in_range {
                    NpcState::Attack
                } else {
                    NpcState::Chase
                }
            }
            NpcState::Attack => {
                if !perception.enemy_visible {
                    NpcState::Idle
                } else if !perception.enemy_in_range {
                    NpcState::Chase
                } else {
                    NpcState::Attack
                }
            }
        };
        self.state
    }
}

impl Default for NpcStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

/// 掩体点：紧邻阻挡格的可通行格
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverPoint {
    /// 掩体格坐标
    pub pos: GridPos,
    /// 四邻域可通行格数量（越小越封闭，越适合当掩体）
    pub openness: u32,
    /// 与玩家的曼哈顿距离
    pub dist: u32,
}

/// 搜索掩体候选点：找出与阻挡格四邻域相邻的可通行格。
///
/// - `openness` 为该格四邻域可通行格数量（越小越封闭）
/// - 按 `(openness, dist)` 升序排序（同值按坐标兜底，保证确定性）
/// - 仅返回 `dist <= max_dist` 的候选
pub fn find_cover_points(grid: &GridMap, player: GridPos, max_dist: u32) -> Vec<CoverPoint> {
    let mut covers = Vec::new();
    for y in 0..grid.height() as i32 {
        for x in 0..grid.width() as i32 {
            let pos = GridPos::new(x, y);
            if !grid.is_passable(pos) {
                continue;
            }
            // 与阻挡格四邻域相邻才算掩体候选（越界不算阻挡格）
            let touches_blocked = NEIGHBOR_OFFSETS.iter().any(|&(dx, dy)| {
                let n = GridPos::new(x + dx, y + dy);
                grid.in_bounds(n) && !grid.is_passable(n)
            });
            if !touches_blocked {
                continue;
            }
            let openness = NEIGHBOR_OFFSETS
                .iter()
                .filter(|&&(dx, dy)| grid.is_passable(GridPos::new(x + dx, y + dy)))
                .count() as u32;
            let dist = player.manhattan(pos);
            if dist <= max_dist {
                covers.push(CoverPoint { pos, openness, dist });
            }
        }
    }
    covers.sort_by(|a, b| {
        a.openness
            .cmp(&b.openness)
            .then(a.dist.cmp(&b.dist))
            .then(a.pos.x.cmp(&b.pos.x))
            .then(a.pos.y.cmp(&b.pos.y))
    });
    covers
}

/// 计算包抄目标点：以 `player → target` 方向为基准，沿垂直轴向
/// `side`（+1/-1）方向偏移 `offset` 格，结果 clamp 到地图范围。
///
/// 四方向网格中垂直方向取主导轴的另一轴：水平主导走 y 轴，垂直主导走 x 轴。
pub fn flank_goal(grid: &GridMap, player: GridPos, target: GridPos, side: i32, offset: u32) -> GridPos {
    let dx = target.x - player.x;
    let dy = target.y - player.y;
    if dx == 0 && dy == 0 {
        // 方向退化（玩家与目标同格）：无包抄方向，返回玩家位置
        let max_x = grid.width() as i32 - 1;
        let max_y = grid.height() as i32 - 1;
        return GridPos::new(player.x.clamp(0, max_x), player.y.clamp(0, max_y));
    }
    let off = side * offset as i32;
    let raw = if dx.abs() >= dy.abs() {
        // 水平主导：垂直方向为 y 轴
        GridPos::new(player.x, player.y + off)
    } else {
        // 垂直主导：垂直方向为 x 轴
        GridPos::new(player.x + off, player.y)
    };
    let max_x = grid.width() as i32 - 1;
    let max_y = grid.height() as i32 - 1;
    GridPos::new(raw.x.clamp(0, max_x), raw.y.clamp(0, max_y))
}

/// 波次类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveKind {
    /// 常规波
    Normal,
    /// Boss 波：每 5 波，最后一只为主怪（高血量/慢速/高伤害）
    Boss,
    /// 援军波：每 3 波，波中途补怪 1..=2 只
    Reinforced,
}

/// Boss 主怪参数：体型/外观通过 max_hp 在渲染侧体现
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BossProfile {
    /// 主怪血量（远超同波小怪）
    pub hp: f32,
    /// 主怪移动速度（慢于同波小怪）
    pub speed: f32,
    /// 主怪攻击距离（略长，站定压力更大）
    pub attack_range: f32,
}

/// 单个波次的难度参数
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveProfile {
    /// 本波 NPC 数量
    pub count: u32,
    /// NPC 移动速度
    pub speed: f32,
    /// NPC 生命值
    pub hp: f32,
    /// NPC 攻击距离
    pub attack_range: f32,
    /// 本波包抄概率
    pub flank_chance: f32,
    /// 波次类型（常规 / Boss / 援军）
    pub kind: WaveKind,
    /// 本波实际出场总敌人数：
    /// - 常规/Boss 波 = count（Boss 波最后一只为主怪，替换常规小怪）
    /// - 援军波 = count + reinforcement_count（含波中途补怪）
    pub total_count: u32,
    /// 援军触发时间（距波开始的秒数；非援军波为 None）
    pub reinforcement_at: Option<f32>,
    /// 单次援军补怪数量（1..=2）
    pub reinforcement_count: u32,
    /// Boss 主怪参数（仅 Boss 波为 Some）
    pub boss: Option<BossProfile>,
    /// 攻击态 NPC 每秒伤害（Boss 波更高）
    pub dps: f32,
}

/// 波次类型判定：每 5 波为 Boss 波，其余每 3 波为援军波，否则常规波。
pub fn wave_kind(n: u32) -> WaveKind {
    if n > 0 && n % 5 == 0 {
        WaveKind::Boss
    } else if n > 0 && n % 3 == 0 {
        WaveKind::Reinforced
    } else {
        WaveKind::Normal
    }
}

/// Boss 主怪参数曲线：血量每 5 波 +150，速度 3.2 起小幅爬升（仍慢于同波小怪），
/// 攻击距离固定取上限 16m（任何波次都不短于同波小怪，站定压力最大）
pub fn boss_profile(n: u32) -> BossProfile {
    let tier = (n / 5).max(1) as f32;
    BossProfile {
        hp: 300.0 + 150.0 * (tier - 1.0),
        speed: (3.2 + 0.1 * (tier - 1.0)).min(6.0),
        attack_range: 16.0,
    }
}

/// 援军参数：触发时间固定波开始后 1.5s，补怪 1..=2 只（按波次奇偶确定，确定性可测）
fn reinforcement_params(n: u32) -> (f32, u32) {
    (1.5, 1 + (n % 2))
}

/// 第 `n` 波的难度曲线（缩放与 `game.rs` 的 `spawn_wave` 保持一致）。
///
/// 速度曲线分段爬升（1..=5 慢速 / 6..=15 中速 / 15+ 高速，全程不回落、封顶 8.0）；
/// HP/数量/攻击距离/包抄沿用原曲线；每 5 波 Boss、每 3 波援军。
pub fn wave_profile(n: u32) -> WaveProfile {
    let nf = n as f32;
    let kind = wave_kind(n);
    // 速度分段：保证全程不回落且封顶 8.0
    let speed = (if n <= 5 {
        4.0 * (1.0 + 0.06 * (nf - 1.0))
    } else if n <= 15 {
        4.0 * (1.0 + 0.06 * 4.0 + 0.10 * (nf - 5.0))
    } else {
        4.0 * (1.0 + 0.06 * 4.0 + 0.10 * 10.0 + 0.05 * (nf - 15.0))
    })
    .min(8.0);
    let count = (4 + 2 * n).min(24);
    let (reinforcement_at, reinforcement_count) = if kind == WaveKind::Reinforced {
        let (at, count) = reinforcement_params(n);
        (Some(at), count)
    } else {
        (None, 0)
    };
    let boss = if kind == WaveKind::Boss {
        Some(boss_profile(n))
    } else {
        None
    };
    let total_count = count + reinforcement_count;
    WaveProfile {
        count,
        speed,
        hp: 100.0 + 20.0 * (nf - 1.0),
        attack_range: 12.0 + ((n / 2).min(4)) as f32,
        flank_chance: (0.22 + 0.08 * nf).min(0.6),
        kind,
        total_count,
        reinforcement_at,
        reinforcement_count,
        boss,
        dps: if kind == WaveKind::Boss { 12.0 } else { 5.0 },
    }
}

/// 确定性伪随机判断某 NPC 是否本波执行包抄：
/// 由 `npc_id` 与 `wave` 生成 0..100 的伪随机数，与 `flank_chance` 比较。
pub fn should_flank(flank_chance: f32, npc_id: u32, wave: u32) -> bool {
    let r = ((npc_id as u64 * 7 + wave as u64 * 13) % 100) as f32 / 100.0;
    r < flank_chance
}

/// 战术角色（多 AI 协同的分工基础，每波按 NPC id 与波次确定性分配）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalRole {
    /// 突击手：直线突进，接近时锯齿机动
    Rusher,
    /// 侧翼包抄：沿垂直轴向大偏移绕到玩家侧面
    Flanker,
    /// 压制手：推进到射程边缘站定压制
    Suppressor,
    /// 掩体跃进：逐掩体推进
    CoverCrawler,
}

/// 战术角色分配：由 `(spawn 槽位, wave)` 确定性哈希生成，同参数永远同角色。
///
/// 分配规则（按序取第一命中）：
/// - 哈希值落入 `flank_chance` 区间 → `Flanker`（沿用波次包抄概率）
/// - 第 2 波起，其余哈希为偶数 → `Suppressor`（约半数转为压制手）
/// - 第 3 波起，其余哈希能被 3 整除 → `CoverCrawler`
/// - 其余 → `Rusher`
pub fn role_for(slot: u32, wave: u32, flank_chance: f32) -> TacticalRole {
    let h = slot.wrapping_mul(31).wrapping_add(wave.wrapping_mul(17)) % 100;
    let flank_pct = (flank_chance.clamp(0.0, 1.0) * 100.0) as u32;
    if h < flank_pct {
        TacticalRole::Flanker
    } else if wave >= 2 && h % 2 == 0 {
        TacticalRole::Suppressor
    } else if wave >= 3 && h % 3 == 0 {
        TacticalRole::CoverCrawler
    } else {
        TacticalRole::Rusher
    }
}

/// 战斗战术（移动态行为选择；Attack 态统一站定开火，保证冒烟瞄准机制）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tactic {
    /// 直线突进（接近时锯齿躲避机动）
    Advance,
    /// 侧翼包抄（垂直轴向偏移绕到侧面）
    Flank,
    /// 偷袭绕背：玩家未面朝时绕大圈逼近；被发现后转侧翼
    Ambush,
    /// 压制：推进到射程边缘站定
    Suppress,
    /// 掩体间跃进
    CoverAdvance,
    /// 低血量撤向遮挡掩体
    Retreat,
    /// 站定开火（Attack 态）
    Hold,
    /// 掩体利用：接近射程边缘时先移动到障碍环带掩体，再从掩体推进开火。
    /// 由 game.rs 在 Chase 态按距离把 Advance/Suppress 覆盖为 CoverSeek
    /// （目标选择见 game.rs `pick_attack_cover`），冲锋时不做（冲锋 = 全队直突）。
    CoverSeek,
}

/// 战术决策：低血量且未进入射程 → 撤退；否则按角色 + 玩家是否面朝本 NPC。
///
/// 进入射程后状态机切到 `Attack`（站定开火），战术退居其次。
pub fn pick_tactic(role: TacticalRole, p: &NpcPerception) -> Tactic {
    if p.low_hp && !p.enemy_in_range {
        return Tactic::Retreat;
    }
    match role {
        TacticalRole::Rusher => Tactic::Advance,
        TacticalRole::Flanker => {
            if p.player_facing {
                Tactic::Flank
            } else {
                Tactic::Ambush
            }
        }
        TacticalRole::Suppressor => Tactic::Suppress,
        TacticalRole::CoverCrawler => Tactic::CoverAdvance,
    }
}

/// 同步冲锋判定（带滞回，防边界震荡）：未激活时 Chase/Attack ≥50% 且 ≥2 只 → 触发；
/// 已激活后需 ≥60% 保持，低于 60% 才取消。
pub fn should_charge(attacking: u32, total: u32, active: bool) -> bool {
    if total == 0 {
        return false;
    }
    if active {
        attacking * 10 >= total * 6
    } else {
        attacking >= 2 && attacking * 10 >= total * 5
    }
}

/// 角度差归一到 `[-π, π]`（camera.yaw 无界累加，跨 ±π 比较前必须归一）。
pub fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = a - b;
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}

/// 目标方位角（弧度，与 camera.yaw 同约定：yaw=0 看向 -Z，逆时针为正）。
pub fn yaw_to_target(from_x: f32, from_z: f32, to_x: f32, to_z: f32) -> f32 {
    // 写成 (from - to) 避免 -0.0 参与 atan2（atan2(-0.0, -1.0) = -π 而非 π）
    (from_x - to_x).atan2(from_z - to_z)
}

/// 锯齿躲避横向偏移：时间驱动正弦，相位按 id 确定性错开，结果在 `[-amplitude, amplitude]`。
pub fn zigzag_offset(time: f32, id: u32, amplitude: f32) -> f32 {
    amplitude * (time * 2.2 + id as f32 * 1.7).sin()
}

/// 遮挡掩体搜索：仅返回「邻接阻挡格位于 NPC 与玩家之间」的掩体候选
/// （即玩家视线/弹道会被该阻挡格挡住），排序与 `find_cover_points` 一致。
pub fn find_cover_shielding(
    grid: &GridMap,
    npc: GridPos,
    player: GridPos,
    max_dist: u32,
) -> Vec<CoverPoint> {
    if npc == player {
        return Vec::new();
    }
    let px = (player.x - npc.x) as f32;
    let pz = (player.y - npc.y) as f32;
    let player_len = (px * px + pz * pz).sqrt();
    let mut covers = find_cover_points(grid, npc, max_dist);
    covers.retain(|c| {
        NEIGHBOR_OFFSETS.iter().any(|&(dx, dy)| {
            let n = GridPos::new(c.pos.x + dx, c.pos.y + dy);
            if !grid.in_bounds(n) || grid.is_passable(n) {
                return false;
            }
            let bx = (n.x - npc.x) as f32;
            let bz = (n.y - npc.y) as f32;
            let block_len = (bx * bx + bz * bz).sqrt();
            // 阻挡格在 npc → player 方向（点积 > 0）
            (bx * px + bz * pz) / (block_len * player_len) > 0.0
        })
    });
    // 撤退掩体排序：封闭性优先（openness 升序），同封闭性取离 NPC 更远者（dist 降序），
    // 避免取到"最开放"的暴露点；调用方取 `.first()`。
    covers.sort_by(|a, b| {
        a.openness
            .cmp(&b.openness)
            .then(b.dist.cmp(&a.dist))
            .then(a.pos.x.cmp(&b.pos.x))
            .then(a.pos.y.cmp(&b.pos.y))
    });
    covers
}

/// 偷袭绕背目标点：从两个侧翼偏移中选「玩家朝向得分更低」的一侧
/// （得分 = 玩家朝向与 玩家→目标 方向的单位点积，越小越在玩家背后）。
///
/// `player_yaw` 用 camera.yaw（yaw=0 → 朝向 -Z）；两侧得分相同时取 +1 侧，保证确定性。
pub fn ambush_goal(
    grid: &GridMap,
    player_g: GridPos,
    npc_g: GridPos,
    player_yaw: f32,
    offset: u32,
) -> GridPos {
    let a = flank_goal(grid, player_g, npc_g, 1, offset);
    let b = flank_goal(grid, player_g, npc_g, -1, offset);
    let fwd_x = -player_yaw.sin();
    let fwd_z = -player_yaw.cos();
    let score = |g: GridPos| -> f32 {
        let dx = (g.x - player_g.x) as f32;
        let dz = (g.y - player_g.y) as f32;
        let len = (dx * dx + dz * dz).sqrt().max(1e-4);
        (fwd_x * dx + fwd_z * dz) / len
    };
    // 容差平局裁决：两侧得分差 < 1e-6 视为平分（f32 三角函数噪声不翻转选择），平分取 +1 侧
    if score(b) < score(a) - 1e-6 { b } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_opposite_flips_side() {
        assert_eq!(Team::Red.opposite(), Team::Blue);
        assert_eq!(Team::Blue.opposite(), Team::Red);
        assert_eq!(Team::Red.opposite().opposite(), Team::Red);
    }

    /// 校验路径合法性：端点正确、全程可通行、相邻格四方向相邻
    fn assert_path_valid(map: &GridMap, start: GridPos, goal: GridPos, path: &[GridPos]) {
        assert_eq!(path.first(), Some(&start), "路径必须以起点开始");
        assert_eq!(path.last(), Some(&goal), "路径必须以终点结束");
        for &pos in path {
            assert!(map.is_passable(pos), "路径经过了阻挡格: {:?}", pos);
        }
        for w in path.windows(2) {
            assert_eq!(
                w[0].manhattan(w[1]),
                1,
                "相邻路径点必须四方向相邻: {:?} -> {:?}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn astar_straight_line() {
        let map = GridMap::new(6, 6);
        let start = GridPos::new(1, 1);
        let goal = GridPos::new(4, 4);
        let path = find_path(&map, start, goal).expect("空旷地图应有路径");
        assert_path_valid(&map, start, goal, &path);
        assert_eq!(path.len(), 7, "曼哈顿距离 6，路径格数应为 7");
    }

    #[test]
    fn astar_around_obstacle_wall() {
        let mut map = GridMap::new(7, 7);
        for y in 0..=2 {
            map.block(GridPos::new(3, y));
        }
        let start = GridPos::new(1, 1);
        let goal = GridPos::new(5, 1);
        let path = find_path(&map, start, goal).expect("墙未封死，应有绕行路径");
        assert_path_valid(&map, start, goal, &path);
        assert!(path.len() > 5, "直线 5 格被墙挡住，必须绕行: {:?}", path);
    }

    #[test]
    fn astar_around_block() {
        let mut map = GridMap::new(7, 7);
        for pos in [
            GridPos::new(2, 1),
            GridPos::new(2, 2),
            GridPos::new(2, 3),
            GridPos::new(1, 2),
            GridPos::new(3, 2),
        ] {
            map.block(pos);
        }
        let start = GridPos::new(0, 3);
        let goal = GridPos::new(6, 3);
        let path = find_path(&map, start, goal).expect("方块未封死边界，应有绕行路径");
        assert_path_valid(&map, start, goal, &path);
        assert!(path.len() > 7, "直线 7 格被方块挡住，必须绕行: {:?}", path);
    }

    #[test]
    fn astar_no_path_when_goal_surrounded() {
        let mut map = GridMap::new(5, 5);
        for pos in [
            GridPos::new(2, 3),
            GridPos::new(4, 3),
            GridPos::new(3, 2),
            GridPos::new(3, 4),
        ] {
            map.block(pos);
        }
        assert_eq!(
            find_path(&map, GridPos::new(0, 0), GridPos::new(3, 3)),
            None
        );
    }

    #[test]
    fn astar_blocked_or_out_of_bounds_endpoints() {
        let mut map = GridMap::new(4, 4);
        map.block(GridPos::new(1, 1));
        assert_eq!(
            find_path(&map, GridPos::new(1, 1), GridPos::new(2, 2)),
            None,
            "起点为阻挡格"
        );
        assert_eq!(
            find_path(&map, GridPos::new(0, 0), GridPos::new(1, 1)),
            None,
            "终点为阻挡格"
        );
        assert_eq!(
            find_path(&map, GridPos::new(9, 9), GridPos::new(0, 0)),
            None,
            "起点越界"
        );
        assert_eq!(
            find_path(&map, GridPos::new(0, 0), GridPos::new(9, 9)),
            None,
            "终点越界"
        );
    }

    #[test]
    fn astar_same_start_and_goal() {
        let map = GridMap::new(3, 3);
        let pos = GridPos::new(1, 2);
        assert_eq!(find_path(&map, pos, pos), Some(vec![pos]));
    }

    #[test]
    fn grid_map_helpers() {
        let mut map = GridMap::new(3, 4);
        assert_eq!(map.width(), 3);
        assert_eq!(map.height(), 4);
        assert!(map.is_passable(GridPos::new(0, 0)));
        assert!(!map.is_passable(GridPos::new(3, 0)), "越界不可通行");
        assert!(!map.is_passable(GridPos::new(0, -1)), "负坐标不可通行");

        map.block(GridPos::new(1, 2));
        assert!(!map.is_passable(GridPos::new(1, 2)));
        map.clear(GridPos::new(1, 2));
        assert!(map.is_passable(GridPos::new(1, 2)));
    }

    fn perception() -> NpcPerception {
        NpcPerception::default()
    }

    #[test]
    fn state_machine_full_cycle() {
        let mut fsm = NpcStateMachine::new();
        assert_eq!(fsm.state(), NpcState::Idle);

        assert_eq!(fsm.update(perception()), NpcState::Idle);

        let mut p = perception();
        p.start_patrol = true;
        assert_eq!(fsm.update(p), NpcState::Patrol);

        let mut p = perception();
        p.enemy_visible = true;
        assert_eq!(fsm.update(p), NpcState::Chase);

        let mut p = perception();
        p.enemy_visible = true;
        p.enemy_in_range = true;
        assert_eq!(fsm.update(p), NpcState::Attack);

        let mut p = perception();
        p.enemy_visible = true;
        assert_eq!(fsm.update(p), NpcState::Chase);

        assert_eq!(fsm.update(perception()), NpcState::Idle);
    }

    #[test]
    fn state_machine_direct_transitions() {
        let mut fsm = NpcStateMachine::new();

        let mut p = perception();
        p.enemy_visible = true;
        assert_eq!(fsm.update(p), NpcState::Chase);

        let mut p = perception();
        p.enemy_visible = true;
        p.enemy_in_range = true;
        assert_eq!(fsm.update(p), NpcState::Attack);

        assert_eq!(fsm.update(perception()), NpcState::Idle);

        let mut p = perception();
        p.start_patrol = true;
        assert_eq!(fsm.update(p), NpcState::Patrol);

        let mut p = perception();
        p.patrol_finished = true;
        assert_eq!(fsm.update(p), NpcState::Idle);
    }

    #[test]
    fn state_machine_keeps_state_on_no_trigger() {
        let mut fsm = NpcStateMachine::new();

        let mut p = perception();
        p.start_patrol = true;
        fsm.update(p);
        assert_eq!(fsm.update(perception()), NpcState::Patrol);

        let mut p = perception();
        p.enemy_visible = true;
        fsm.update(p);
        assert_eq!(fsm.update(p), NpcState::Chase);

        let mut p = perception();
        p.enemy_visible = true;
        p.enemy_in_range = true;
        fsm.update(p);
        assert_eq!(fsm.update(p), NpcState::Attack);
    }

    #[test]
    fn cover_points_near_blocked_cluster() {
        let mut map = GridMap::new(7, 7);
        // 中央 3x3 阻挡簇
        for y in 2..=4 {
            for x in 2..=4 {
                map.block(GridPos::new(x, y));
            }
        }
        let player = GridPos::new(0, 0);
        let covers = find_cover_points(&map, player, 99);

        assert!(!covers.is_empty(), "阻挡簇周围应有掩体候选");
        for c in &covers {
            assert!(map.is_passable(c.pos), "掩体必须可通行: {:?}", c.pos);
            assert_eq!(c.dist, player.manhattan(c.pos));
            // 必须与阻挡格四邻域相邻
            let touches_blocked = NEIGHBOR_OFFSETS.iter().any(|&(dx, dy)| {
                let n = GridPos::new(c.pos.x + dx, c.pos.y + dy);
                map.in_bounds(n) && !map.is_passable(n)
            });
            assert!(touches_blocked, "掩体必须紧邻阻挡格: {:?}", c.pos);
            // 紧邻阻挡格，四邻域最多 3 个可通行
            assert!(c.openness <= 3, "紧邻阻挡格时 openness 应 <= 3: {:?}", c.pos);
        }
        // (openness, dist) 升序
        for w in covers.windows(2) {
            let (a, b) = (w[0], w[1]);
            assert!(
                a.openness < b.openness || (a.openness == b.openness && a.dist <= b.dist),
                "排序必须 (openness, dist) 升序: {:?} vs {:?}",
                a,
                b
            );
        }
        // 具体候选：簇角点 (2,2) 的可通行邻格必须出现
        let mut found: Vec<GridPos> = covers
            .iter()
            .map(|c| c.pos)
            .filter(|&p| p.manhattan(GridPos::new(2, 2)) == 1 && map.is_passable(p))
            .collect();
        found.sort_by(|a, b| a.x.cmp(&b.x).then(a.y.cmp(&b.y)));
        assert_eq!(
            found.len(),
            2,
            "簇角点可通行邻格应成为候选: {:?}",
            found
        );
        assert_eq!(found, vec![GridPos::new(1, 2), GridPos::new(2, 1)]);
    }

    #[test]
    fn cover_points_dist_and_openness() {
        let mut map = GridMap::new(5, 5);
        map.block(GridPos::new(2, 2));
        let covers = find_cover_points(&map, GridPos::new(0, 0), 3);
        // (1,2)/(2,1) 距离 3 在内；(2,3)/(3,2) 距离 5 被 max_dist 过滤
        let mut positions: Vec<GridPos> = covers.iter().map(|c| c.pos).collect();
        positions.sort_by(|a, b| a.x.cmp(&b.x).then(a.y.cmp(&b.y)));
        assert_eq!(positions, vec![GridPos::new(1, 2), GridPos::new(2, 1)]);

        // 边角阻挡格：紧邻格 openness 更小
        let mut corner = GridMap::new(5, 5);
        corner.block(GridPos::new(0, 0));
        let corner_covers = find_cover_points(&corner, GridPos::new(0, 0), 99);
        let c = corner_covers
            .iter()
            .find(|c| c.pos == GridPos::new(1, 0))
            .expect("(1,0) 应紧邻角落阻挡格");
        // 邻格只有 (0,0) 阻挡与 (1,0) 越界上侧，(2,0)/(1,1) 可通行
        assert_eq!(c.openness, 2);
        // openness 最小者排最前（角落阻挡格旁的候选 openness 小）
        assert_eq!(corner_covers[0].openness, 2);
    }

    #[test]
    fn flank_goal_vertical_offset_and_clamp() {
        let map = GridMap::new(10, 10);
        // 水平主导（player→target 沿 x 轴）：垂直方向为 y 轴
        assert_eq!(
            flank_goal(&map, GridPos::new(2, 2), GridPos::new(7, 2), 1, 2),
            GridPos::new(2, 4)
        );
        assert_eq!(
            flank_goal(&map, GridPos::new(2, 2), GridPos::new(7, 2), -1, 2),
            GridPos::new(2, 0)
        );
        // 垂直主导（player→target 沿 y 轴）：垂直方向为 x 轴
        assert_eq!(
            flank_goal(&map, GridPos::new(4, 4), GridPos::new(4, 9), 1, 2),
            GridPos::new(6, 4)
        );
        assert_eq!(
            flank_goal(&map, GridPos::new(4, 4), GridPos::new(4, 9), -1, 2),
            GridPos::new(2, 4)
        );
        // clamp 到地图范围
        assert_eq!(
            flank_goal(&map, GridPos::new(0, 0), GridPos::new(5, 0), -1, 3),
            GridPos::new(0, 0),
            "负方向越界应 clamp 到 0"
        );
        assert_eq!(
            flank_goal(&map, GridPos::new(0, 0), GridPos::new(5, 0), 1, 3),
            GridPos::new(0, 3)
        );
        assert_eq!(
            flank_goal(&map, GridPos::new(9, 9), GridPos::new(0, 9), 1, 5),
            GridPos::new(9, 9),
            "正方向越界应 clamp 到上界"
        );
        // 玩家与目标同格：退化为玩家位置
        assert_eq!(
            flank_goal(&map, GridPos::new(3, 3), GridPos::new(3, 3), 1, 2),
            GridPos::new(3, 3)
        );
    }

    #[test]
    fn wave_profile_monotonic() {
        let mut prev = wave_profile(1);
        assert_eq!(prev.count, 6);
        assert_eq!(prev.speed, 4.0);
        assert_eq!(prev.hp, 100.0);
        assert_eq!(prev.attack_range, 12.0);
        for n in 2..=40 {
            let cur = wave_profile(n);
            assert!(cur.count >= prev.count, "count 应不降: wave {} -> {}", prev.count, cur.count);
            assert!(cur.speed >= prev.speed, "speed 应不降: wave {} -> {}", prev.speed, cur.speed);
            assert!(cur.hp >= prev.hp, "hp 应不降: wave {} -> {}", prev.hp, cur.hp);
            assert!(
                cur.attack_range >= prev.attack_range,
                "attack_range 应不降: wave {} -> {}",
                prev.attack_range,
                cur.attack_range
            );
            assert!(
                cur.flank_chance >= prev.flank_chance,
                "flank_chance 应不降: wave {} -> {}",
                prev.flank_chance,
                cur.flank_chance
            );
            prev = cur;
        }
        // 封顶边界
        let late = wave_profile(100);
        assert_eq!(late.count, 24);
        assert_eq!(late.speed, 8.0);
        assert_eq!(late.attack_range, 16.0);
        assert_eq!(late.flank_chance, 0.6);
        assert_eq!(late.hp, 100.0 + 20.0 * 99.0);
        // 攻击距离台阶
        assert_eq!(wave_profile(2).attack_range, 13.0);
        assert_eq!(wave_profile(8).attack_range, 16.0);
        assert_eq!(wave_profile(9).attack_range, 16.0);
        // 与 game.rs spawn_wave 早期波次一致
        assert_eq!(wave_profile(1).count, (4 + 2 * 1).min(24));
        assert_eq!(
            wave_profile(1).speed,
            (4.0f32 * (1.0 + 0.06 * (1.0f32 - 1.0))).min(8.0)
        );
    }

    #[test]
    fn should_flank_deterministic_and_boundaries() {
        // 同参数同结果
        for &(chance, id, wave) in &[
            (0.2, 1u32, 1u32),
            (0.5, 7, 3),
            (0.0, 0, 0),
            (1.0, 999, 42),
        ] {
            assert_eq!(should_flank(chance, id, wave), should_flank(chance, id, wave));
        }
        // 阈值边界：r 恰等于 flank_chance 时不触发（严格小于）
        let (id, wave) = (1u32, 1u32);
        let r = ((id as u64 * 7 + wave as u64 * 13) % 100) as f32 / 100.0;
        assert_eq!(r, 0.2);
        assert!(!should_flank(r, id, wave), "r == chance 不应包抄");
        assert!(should_flank(r + 0.01, id, wave), "r < chance 应包抄");
        // 极端阈值
        assert!(!should_flank(0.0, 0, 0), "概率 0 永不包抄");
        assert!(should_flank(1.0, 0, 0), "概率 1 必包抄");
        assert!(should_flank(1.0, u32::MAX, u32::MAX), "大 id/wave 不溢出且必包抄");
    }

    /// 波次类型判定：每 5 波 Boss、其余每 3 波援军、其余常规
    #[test]
    fn wave_kind_classification() {
        assert_eq!(wave_kind(1), WaveKind::Normal);
        assert_eq!(wave_kind(2), WaveKind::Normal);
        assert_eq!(wave_kind(3), WaveKind::Reinforced);
        assert_eq!(wave_kind(4), WaveKind::Normal);
        assert_eq!(wave_kind(5), WaveKind::Boss);
        assert_eq!(wave_kind(6), WaveKind::Reinforced);
        assert_eq!(wave_kind(10), WaveKind::Boss);
        assert_eq!(wave_kind(15), WaveKind::Boss);
        assert_eq!(wave_kind(0), WaveKind::Normal, "n=0 防御性归为常规");
    }

    /// Boss 波参数：主怪高血量/慢速/攻击距离略长，血量随波次递增
    #[test]
    fn boss_wave_profile_params() {
        let p5 = wave_profile(5);
        assert_eq!(p5.kind, WaveKind::Boss);
        let b5 = p5.boss.expect("Boss 波应有主怪参数");
        assert_eq!(b5.hp, 300.0);
        assert!(b5.hp > p5.hp, "主怪血量应远超同波小怪: {} vs {}", b5.hp, p5.hp);
        assert!(b5.speed < p5.speed, "主怪应慢于同波小怪: {} vs {}", b5.speed, p5.speed);
        assert!(b5.attack_range >= p5.attack_range, "主怪攻击距离不短于小怪");
        let b10 = wave_profile(10).boss.expect("第 10 波应有主怪参数");
        assert!(b10.hp > b5.hp, "主怪血量应随波次递增");
        assert!(b10.speed >= b5.speed, "主怪速度不应回落");
        // 主怪参数确定性
        assert_eq!(boss_profile(5), boss_profile(5));
    }

    /// 援军波参数：触发时间固定 1.5s、补怪 1..=2、total_count 与补怪数自洽
    #[test]
    fn reinforcement_wave_params() {
        let p3 = wave_profile(3);
        assert_eq!(p3.kind, WaveKind::Reinforced);
        assert_eq!(p3.reinforcement_at, Some(1.5));
        assert!((1..=2).contains(&p3.reinforcement_count), "补怪数应为 1..=2");
        assert_eq!(p3.total_count, p3.count + p3.reinforcement_count);
        let p6 = wave_profile(6);
        assert_eq!(p6.kind, WaveKind::Reinforced);
        assert_eq!(p6.total_count, p6.count + p6.reinforcement_count);
        // 常规/Boss 波无援军
        assert_eq!(wave_profile(1).reinforcement_at, None);
        assert_eq!(wave_profile(1).total_count, wave_profile(1).count);
        assert_eq!(wave_profile(5).reinforcement_at, None);
        assert_eq!(wave_profile(5).total_count, wave_profile(5).count);
    }

    /// 难度曲线关键阈值：速度分段与封顶、Boss 波 dps 更高、常规波主曲线不变
    #[test]
    fn wave_profile_thresholds_locked() {
        assert_eq!(wave_profile(1).speed, 4.0);
        assert!((wave_profile(5).speed - 4.96).abs() < 1e-4, "第 5 波速度 4.96");
        assert!((wave_profile(6).speed - 5.36).abs() < 1e-4, "第 6 波进入中速段");
        assert_eq!(wave_profile(15).speed, 8.0, "第 15 波封顶");
        assert_eq!(wave_profile(16).speed, 8.0);
        assert_eq!(wave_profile(100).speed, 8.0);
        assert_eq!(wave_profile(1).dps, 5.0);
        assert_eq!(wave_profile(3).dps, 5.0);
        assert_eq!(wave_profile(5).dps, 12.0, "Boss 波 dps 更高");
        // 常规波主曲线保持原样（供集成回归）
        assert_eq!(wave_profile(4).count, (4 + 2 * 4).min(24));
        assert_eq!(wave_profile(4).hp, 100.0 + 20.0 * 3.0);
    }

    /// 角色分配：确定性、波次门槛、概率边界
    #[test]
    fn role_for_deterministic_and_wave_gates() {
        for &(id, wave, chance) in &[
            (1u32, 1u32, 0.2f32),
            (7, 3, 0.35),
            (0, 0, 0.0),
            (999, 42, 1.0),
        ] {
            assert_eq!(
                role_for(id, wave, chance),
                role_for(id, wave, chance),
                "同参数必须同角色"
            );
        }
        // 第 1 波只允许 Flanker/Rusher（无压制/掩体跃进）
        for id in 0..200u32 {
            let r = role_for(id, 1, 0.2);
            assert!(
                r == TacticalRole::Flanker || r == TacticalRole::Rusher,
                "第 1 波不应出现高级角色: id={} role={:?}",
                id,
                r
            );
        }
        // 第 2 波出现压制手、第 3 波出现掩体跃进
        let has_role = |wave: u32, want: TacticalRole| {
            (0..64u32).any(|id| role_for(id, wave, 0.2) == want)
        };
        assert!(has_role(1, TacticalRole::Rusher), "第 1 波应有突击手");
        assert!(has_role(2, TacticalRole::Suppressor), "第 2 波应有压制手");
        assert!(!has_role(1, TacticalRole::Suppressor), "第 1 波无压制手");
        assert!(has_role(3, TacticalRole::CoverCrawler), "第 3 波应有掩体跃进");
        assert!(!has_role(2, TacticalRole::CoverCrawler), "第 2 波无掩体跃进");
        // 概率边界：0 无包抄、1 全包抄
        for id in 0..100u32 {
            assert_ne!(role_for(id, 1, 0.0), TacticalRole::Flanker, "概率 0 不包抄");
            assert_eq!(role_for(id, 1, 1.0), TacticalRole::Flanker, "概率 1 全包抄");
        }
    }

    /// 战术决策：低血量撤退优先；角色与玩家面朝决定侧翼/偷袭
    #[test]
    fn pick_tactic_respects_hp_role_and_facing() {
        let mut p = NpcPerception {
            low_hp: true,
            ..NpcPerception::default()
        };
        for role in [
            TacticalRole::Rusher,
            TacticalRole::Flanker,
            TacticalRole::Suppressor,
            TacticalRole::CoverCrawler,
        ] {
            assert_eq!(pick_tactic(role, &p), Tactic::Retreat, "低血量未进射程应撤退");
        }
        p.low_hp = false;
        p.enemy_in_range = true;
        for role in [
            TacticalRole::Rusher,
            TacticalRole::Flanker,
            TacticalRole::Suppressor,
            TacticalRole::CoverCrawler,
        ] {
            assert_ne!(pick_tactic(role, &p), Tactic::Retreat, "已进射程不应撤退");
        }
        p.enemy_in_range = false;
        assert_eq!(pick_tactic(TacticalRole::Rusher, &p), Tactic::Advance);
        assert_eq!(pick_tactic(TacticalRole::Suppressor, &p), Tactic::Suppress);
        assert_eq!(pick_tactic(TacticalRole::CoverCrawler, &p), Tactic::CoverAdvance);
        // 包抄手：被面朝 → 侧翼；未面朝 → 偷袭绕背
        p.player_facing = true;
        assert_eq!(pick_tactic(TacticalRole::Flanker, &p), Tactic::Flank);
        p.player_facing = false;
        assert_eq!(pick_tactic(TacticalRole::Flanker, &p), Tactic::Ambush);
    }

    /// 同步冲锋：开启 ≥50%（且 ≥2 只），激活后 <60% 才关闭（滞回）
    #[test]
    fn charge_thresholds() {
        // 未激活：≥50% 且 ≥2 只触发
        assert!(!should_charge(0, 0, false), "空场不冲锋");
        assert!(!should_charge(0, 3, false));
        assert!(!should_charge(1, 3, false), "未过半不冲锋");
        assert!(!should_charge(1, 2, false), "单只不冲锋");
        assert!(should_charge(2, 3, false), "2/3 过半冲锋");
        assert!(should_charge(2, 2, false));
        assert!(should_charge(4, 8, false), "恰好过半也冲锋");
        assert!(!should_charge(3, 8, false));
        // 滞回：激活后需 ≥60% 保持，低于 60% 取消
        assert!(should_charge(6, 10, true), "激活后 60% 保持");
        assert!(should_charge(7, 10, true));
        assert!(should_charge(8, 10, true));
        assert!(!should_charge(5, 10, true), "50% 已低于关闭阈值");
        assert!(!should_charge(4, 10, true));
        assert!(!should_charge(0, 0, true), "空场即使已激活也关闭");
        assert!(!should_charge(0, 3, true));
    }

    /// 角度差归一：跨 ±π 正确折叠
    #[test]
    fn angle_diff_wraps_across_pi() {
        assert!((angle_diff(0.0, 0.0)).abs() < 1e-6);
        assert!((angle_diff(std::f32::consts::PI, -std::f32::consts::PI)).abs() < 1e-6);
        assert!((angle_diff(3.5, 0.0) - (3.5 - std::f32::consts::TAU)).abs() < 1e-6);
        assert!((angle_diff(-3.5, 0.0) - (-3.5 + std::f32::consts::TAU)).abs() < 1e-6);
        assert!((angle_diff(100.0, 100.0)).abs() < 1e-6, "无界 yaw 同值差为 0");
    }

    /// 目标方位角约定：yaw=0 看向 -Z；+X 东 → -π/2；+Z 南 → π
    #[test]
    fn yaw_to_target_matches_camera_convention() {
        let y = |tx: f32, tz: f32| yaw_to_target(0.0, 0.0, tx, tz);
        assert!((y(0.0, -1.0)).abs() < 1e-6, "-Z 方向 yaw=0");
        assert!((y(1.0, 0.0) - (-std::f32::consts::FRAC_PI_2)).abs() < 1e-6, "+X 东 yaw=-π/2");
        assert!((y(0.0, 1.0) - std::f32::consts::PI).abs() < 1e-6, "+Z 南 yaw=π");
        assert!((y(-1.0, 0.0) - std::f32::consts::FRAC_PI_2).abs() < 1e-6, "-X 西 yaw=π/2");
        // 与 forward(-sin, -cos) 自洽：对准目标时 yaw 应指向该目标
        let yaw = y(5.0, 3.0);
        let fwd = (-yaw.sin(), -yaw.cos());
        let to = (5.0f32, 3.0f32);
        let len = (to.0 * to.0 + to.1 * to.1).sqrt();
        assert!(
            (fwd.0 - to.0 / len).abs() < 1e-4 && (fwd.1 - to.1 / len).abs() < 1e-4,
            "yaw 对准目标后 forward 应指向目标"
        );
    }

    /// 锯齿偏移：范围、对称、确定性
    #[test]
    fn zigzag_offset_bounded_and_deterministic() {
        for t in [0.0f32, 0.37, 12.9] {
            for id in [0u32, 1, 7] {
                for amp in [0.0f32, 2.0, 3.0] {
                    let o = zigzag_offset(t, id, amp);
                    assert!(o >= -amp - 1e-5 && o <= amp + 1e-5, "偏移应受限: {o}");
                    assert_eq!(zigzag_offset(t, id, amp), o, "同参数同结果");
                }
            }
        }
        assert_eq!(zigzag_offset(1.0, 3, 0.0), 0.0, "零幅度恒为 0");
        assert!(
            (zigzag_offset(1.0, 0, 2.0) * 2.0 - zigzag_offset(1.0, 0, 4.0)).abs() < 1e-6,
            "幅度线性缩放"
        );
        assert!(
            (zigzag_offset(0.0, 0, 2.0) + zigzag_offset(std::f32::consts::PI / 2.2, 0, 2.0))
                .abs()
                < 1e-5,
            "半周期反对称（相位 0）"
        );
    }

    /// 遮挡掩体：只保留「阻挡格位于 NPC 与玩家之间」的候选
    #[test]
    fn cover_shielding_requires_blocked_between() {
        let mut map = GridMap::new(9, 9);
        map.block(GridPos::new(4, 4)); // 中央单格阻挡
        let npc = GridPos::new(6, 4); // NPC 在东侧
        let player = GridPos::new(2, 4); // 玩家在西侧
        let shielded = find_cover_shielding(&map, npc, player, 99);
        assert!(!shielded.is_empty(), "遮挡掩体应有候选");
        for c in &shielded {
            assert!(map.is_passable(c.pos));
            // 存在邻接阻挡格且在 npc→player 半平面
            let ok = NEIGHBOR_OFFSETS.iter().any(|&(dx, dy)| {
                let n = GridPos::new(c.pos.x + dx, c.pos.y + dy);
                if !map.in_bounds(n) || map.is_passable(n) {
                    return false;
                }
                let bx = (n.x - npc.x) as f32;
                let bz = (n.y - npc.y) as f32;
                let px = (player.x - npc.x) as f32;
                let pz = (player.y - npc.y) as f32;
                bx * px + bz * pz > 0.0
            });
            assert!(ok, "非遮挡掩体不应返回: {:?}", c.pos);
        }
        // 玩家与 NPC 同格：无遮挡掩体（防御性）
        assert!(find_cover_shielding(&map, npc, npc, 99).is_empty());
        // 掩体在阻挡格背后（远离玩家一侧）应被排除：NPC 东侧有另一阻挡格时，
        // 其西侧邻格对玩家而言是"背后"而非遮挡
        let mut map2 = GridMap::new(9, 9);
        map2.block(GridPos::new(7, 4));
        let npc2 = GridPos::new(6, 4);
        let player2 = GridPos::new(2, 4);
        let shielded2 = find_cover_shielding(&map2, npc2, player2, 99);
        let behind = GridPos::new(8, 4); // 阻挡格 (7,4) 的东侧邻格（远离玩家）
        assert!(
            !shielded2.iter().any(|c| c.pos == behind),
            "阻挡格背后邻格不应算遮挡掩体"
        );
        // 撤退排序：openness 升序，同 openness 时 dist 降序（更远者优先）
        let shielded3 = find_cover_shielding(&map, npc, player, 99);
        for w in shielded3.windows(2) {
            let (a, b) = (w[0], w[1]);
            assert!(
                a.openness < b.openness
                    || (a.openness == b.openness && a.dist >= b.dist),
                "撤退掩体排序必须 (openness 升序, dist 降序): {:?} vs {:?}",
                a,
                b
            );
        }
    }

    /// 偷袭目标点：优先选玩家朝向背后一侧；相同时确定性取 +1 侧
    #[test]
    fn ambush_goal_prefers_behind_player() {
        let grid = GridMap::new(11, 11);
        let player = GridPos::new(5, 5);
        let npc = GridPos::new(7, 5); // NPC 在玩家东侧 → 侧翼候选点为南北两侧
        // yaw=0 → 玩家朝 -Z（北），南侧(+1)偏移点位于玩家背后 → 应取该点
        let goal = ambush_goal(&grid, player, npc, 0.0, 3);
        assert_eq!(goal, flank_goal(&grid, player, npc, 1, 3), "应取南侧(+1)点");
        let fwd = (0.0f32, -1.0f32); // 玩家朝向（-Z）
        let dx = (goal.x - player.x) as f32;
        let dz = (goal.y - player.y) as f32;
        let len = (dx * dx + dz * dz).sqrt();
        assert!(
            (fwd.0 * dx + fwd.1 * dz) / len < 0.0,
            "目标点应位于玩家背向半平面"
        );
        // 反向：yaw=π → 玩家朝 +Z（南），北侧(-1)点应在背后
        let goal2 = ambush_goal(&grid, player, npc, std::f32::consts::PI, 3);
        assert_eq!(goal2, flank_goal(&grid, player, npc, -1, 3), "应取北侧(-1)点");
        // 确定性：同参数同结果
        assert_eq!(
            ambush_goal(&grid, player, npc, 0.0, 3),
            goal
        );
        // 纯垂直平分时（两侧得分相同）确定性取 +1 侧
        let g0 = ambush_goal(&grid, player, npc, 0.0, 3);
        assert_eq!(
            ambush_goal(&grid, player, npc, std::f32::consts::FRAC_PI_2, 3),
            g0,
            "两侧平分时确定性取 +1 侧"
        );
    }
}
