//! AI 寻路系统模块
//!
//! - 网格地图 A* 寻路（2D grid，可通行/阻挡）
//! - NPC 状态机：Idle → Patrol → Chase → Attack（含状态转换条件）
//! - 网格阻挡避障：A* 搜索自动绕过阻挡格
//! - 掩体点搜索 / 包抄目标点 / 波次难度曲线（供 Wave2 集成使用）
//!
//! 本模块仅依赖 std，暂未在 `main` 中接线（先独立提供实现与单元测试）。

#![allow(dead_code)]

use std::cmp::Ordering;
use std::collections::BinaryHeap;

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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NpcPerception {
    /// 视野内是否存在敌人
    pub enemy_visible: bool,
    /// 敌人是否在攻击距离内
    pub enemy_in_range: bool,
    /// 是否开始巡逻（有待巡逻路线）
    pub start_patrol: bool,
    /// 巡逻路线是否完成
    pub patrol_finished: bool,
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
}

/// 第 `n` 波的难度曲线（缩放与 `game.rs` 的 `spawn_wave` 保持一致）。
pub fn wave_profile(n: u32) -> WaveProfile {
    let nf = n as f32;
    WaveProfile {
        count: (4 + 2 * n).min(24),
        speed: (4.0 * (1.0 + 0.06 * (nf - 1.0))).min(8.0),
        hp: 100.0 + 20.0 * (nf - 1.0),
        attack_range: 12.0 + ((n / 2).min(4)) as f32,
        flank_chance: (0.1 + 0.08 * nf).min(0.6),
    }
}

/// 确定性伪随机判断某 NPC 是否本波执行包抄：
/// 由 `npc_id` 与 `wave` 生成 0..100 的伪随机数，与 `flank_chance` 比较。
pub fn should_flank(flank_chance: f32, npc_id: u32, wave: u32) -> bool {
    let r = ((npc_id as u64 * 7 + wave as u64 * 13) % 100) as f32 / 100.0;
    r < flank_chance
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
