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

/// 寻路诊断计数（**只在 `RV3D_AI_DIAG=1` 时累加**，由 `game.rs` 每秒取走并打一行）。
///
/// 未结案 #25 的验收要"数 `find_path` 返回 `None` 的比例" —— 目标不可达正是单帧尖峰的来源
/// （会一路展开整张 128×128 网格）。没有这两个计数器，那条 lead 就只是句话。
static ASTAR_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ASTAR_FAILS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 失败三因分列（2026-09-25 补）：真机上"几乎 100% 失败"已实测到，必须知道是哪一种 ——
/// 起点/目标落在阻挡格是 O(1) 快速失败，连通域穷尽才是 #25 的尖峰来源。
static ASTAR_FAIL_START: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ASTAR_FAIL_GOAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ASTAR_FAIL_EXHAUSTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// "目标不可达 ⇒ 返回**部分路径**（走到最接近目标的可达格）"的次数（2026-09-25 加）。
/// 它**不是失败**：这是修掉"被围死就原地贴着墙"的那条兜底在生效。
static ASTAR_PARTIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 这一秒内 A* **展开的节点总数 / 单次最大展开数 / 堆入队总数**（2026-09-25 加，未结案 #25）。
///
/// 🔴 存在的理由：`ai_us` 的 41.6ms 尖峰既不是"起点阻挡"（O(1) 返回）也不是"连通域穷尽"
/// （真机 `aidiag: astar 1s` 实测**恒为 0**）⇒ 旧归因站不住。**先量再改**：没有这三个数，
/// 任何"加节点预算/缓存"的优化都是拿"我以为更快"换掉"AI 真的能找到路"。1 Hz 取走并清零。
static ASTAR_EXPANDED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ASTAR_EXPANDED_MAX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ASTAR_PUSHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 记一次寻路的工作量（关闭诊断时零成本：一个 `OnceLock` 读 + 提前返回）
fn note_astar_work(expanded: u64, pushed: u64) {
    if !diag_on() {
        return;
    }
    use std::sync::atomic::Ordering::Relaxed;
    ASTAR_EXPANDED.fetch_add(expanded, Relaxed);
    ASTAR_PUSHED.fetch_add(pushed, Relaxed);
    ASTAR_EXPANDED_MAX.fetch_max(expanded, Relaxed);
}

/// `RV3D_AI_DIAG` 开关（与 `game.rs::ai_diag()` 同一套取值：`1`/`on`/`true`）
fn diag_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("RV3D_AI_DIAG").is_ok_and(|v| v == "1" || v == "on" || v == "true")
    })
}

/// 取走并清零"这一秒的寻路调用数"（诊断用）
pub fn astar_calls_take() -> u64 {
    ASTAR_CALLS.swap(0, std::sync::atomic::Ordering::Relaxed)
}

/// 取走并清零"这一秒的寻路失败数"（诊断用）
pub fn astar_fails_take() -> u64 {
    ASTAR_FAILS.swap(0, std::sync::atomic::Ordering::Relaxed)
}

/// 取走并清零三种失败原因（诊断用）：返回 `(起点阻挡, 目标阻挡, 连通域穷尽)`
pub fn astar_fail_reasons_take() -> (u64, u64, u64) {
    (
        ASTAR_FAIL_START.swap(0, std::sync::atomic::Ordering::Relaxed),
        ASTAR_FAIL_GOAL.swap(0, std::sync::atomic::Ordering::Relaxed),
        ASTAR_FAIL_EXHAUSTED.swap(0, std::sync::atomic::Ordering::Relaxed),
    )
}

/// 取走并清零"部分路径"次数（诊断用）—— 目标不可达但给了"最接近目标的可达格"那条兜底
pub fn astar_partial_take() -> u64 {
    ASTAR_PARTIAL.swap(0, std::sync::atomic::Ordering::Relaxed)
}

/// 取走并清零 A* 的工作量（诊断用）：返回 `(这一秒展开节点总数, 单次最大展开数, 这一秒入队总数)`。
/// 未结案 #25 的判据就挂在这三个数上 —— 尖峰到底来自"某一只算爆了"（max 大）还是"调用太多"
/// （calls 大但每次很小），一眼可分。
pub fn astar_work_take() -> (u64, u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        ASTAR_EXPANDED.swap(0, Relaxed),
        ASTAR_EXPANDED_MAX.swap(0, Relaxed),
        ASTAR_PUSHED.swap(0, Relaxed),
    )
}

/// `from` 可通行就原样返回；否则**确定性扩环**找最近的可通行格（越界/超环返回 `None`）。
///
/// 🔴 2026-09-25 真机实测（iGPU 上的 survive 跑）：`aidiag: astar 1s 内 calls=108 fails=108
/// （起点阻挡=108 目标阻挡=0 连通域穷尽=0）` —— **每一只 NPC 都站在阻挡格里**，
/// 而 `find_path` 的第一条判据就是"起点必须可通行" ⇒ 它们**永远拿不到路径**，
/// 只能靠 `direct_goal` 直行，于是越顶越深地贴在墙里、`occluded=true`、玩家打不到
/// ⇒ **波次永远清不掉**（#17 的可见症状）。
///
/// 环内取"直线距离最小"的那个（与 `game.rs` 里原有的目标兜底螺旋同口径），
/// 保证结果**确定**（同一输入必得同一输出）。
pub fn passable_or_nearest(map: &GridMap, from: GridPos, max_ring: i32) -> Option<GridPos> {
    if map.is_passable(from) {
        return Some(from);
    }
    for r in 1..=max_ring {
        let mut best: Option<(i64, GridPos)> = None;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dy.abs() != r {
                    continue; // 只看这一环的边框
                }
                let gp = GridPos::new(from.x + dx, from.y + dy);
                if !map.is_passable(gp) {
                    continue;
                }
                let d = (dx * dx + dy * dy) as i64;
                if best.as_ref().map_or(true, |(bd, _)| d < *bd) {
                    best = Some((d, gp));
                }
            }
        }
        if let Some((_, gp)) = best {
            return Some(gp);
        }
    }
    None
}

/// A* 的**可复用 scratch**（线程本地）：见 [`find_path`] 里的长注释（未结案 #25 的实测依据）。
///
/// 三份 O(格数) 缓冲不再每次调用分配/清零，而是靠 generation 戳判断"本次搜索是否访问过"。
/// 🔴 **必须用两张戳**（`seen` / `closed`）：只用一张的话，"先记起点的 g"与"出堆才算关闭"
/// 会打架 —— 起点会被自己的戳挡住、一个节点都展不开（2026-09-25 实测：`astar_straight_line`
/// 直接返回 `None`）。两张戳的语义与旧实现逐条等价：
///   - `seen[i] == gen`  ⇒ `g[i]` / `parent[i]` 有效；
///   - `closed[i] == gen` ⇒ 已出堆（旧 `closed: Vec<bool>`）。
struct AstarScratch {
    open: BinaryHeap<HeapNode>,
    /// 到该节点的已知最短距离（仅在 `seen[i] == gen` 时有效）
    g: Vec<u32>,
    /// 前驱（仅在 `seen[i] == gen` 时有效）
    parent: Vec<Option<usize>>,
    /// "有有效 g"的戳
    seen: Vec<u32>,
    /// "已出堆"的戳
    closed: Vec<u32>,
    /// 当前 generation（0 保留给"从未访问"）
    gen: u32,
}

impl AstarScratch {
    fn new() -> Self {
        Self {
            open: BinaryHeap::new(),
            g: Vec::new(),
            parent: Vec::new(),
            seen: Vec::new(),
            closed: Vec::new(),
            gen: 0,
        }
    }

    /// 开始一次搜索：必要时扩到 `cells` 项 / 回绕 generation（回绕时整体清零一次，极少发生）
    fn begin(&mut self, cells: usize) {
        if self.seen.len() < cells {
            self.g.resize(cells, 0);
            self.parent.resize(cells, None);
            self.seen.resize(cells, 0);
            self.closed.resize(cells, 0);
            self.gen = 0;
        }
        self.open.clear();
        self.gen = self.gen.wrapping_add(1);
        if self.gen == 0 {
            // u32 回绕（约 43 亿次搜索）：整体清零一次，避免"戳相等"误判
            for s in self.seen.iter_mut() {
                *s = 0;
            }
            for s in self.closed.iter_mut() {
                *s = 0;
            }
            self.gen = 1;
        }
    }
}

thread_local! {
    /// 每线程一份 scratch。AI 会走 `step_ai_parallel`（多线程），**不能**用全局 `Mutex`
    /// （那等于把并行 AI 串行化）；线程本地既免分配又不争用。
    static ASTAR_SCRATCH: std::cell::RefCell<AstarScratch> =
        std::cell::RefCell::new(AstarScratch::new());
}

/// A* 寻路：求 `start` 到 `goal` 的最短四方向路径（含两端点）。
///
/// - 自动绕过阻挡格（阻挡格不可进入）
/// - 起点/终点越界或为阻挡格时返回 `None`
/// - 无可通行路径时返回 `None`
///
/// 诊断开关关闭时**零额外成本**（只是多一次 `is_none()` 判断）；实现见 [`find_path_inner`]。
pub fn find_path(map: &GridMap, start: GridPos, goal: GridPos) -> Option<Vec<GridPos>> {
    let diag = diag_on();
    if diag {
        ASTAR_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    let r = find_path_inner(map, start, goal);
    if diag && r.is_none() {
        ASTAR_FAILS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    r
}

/// [`find_path`] 的实现主体（与诊断计数分离）
fn find_path_inner(map: &GridMap, start: GridPos, goal: GridPos) -> Option<Vec<GridPos>> {
    // 🔴 2026-09-25 实测补：真机日志显示寻路**几乎 100% 失败**（`aidiag: astar … calls=104 fails=104`）。
    // 失败有三种完全不同的原因，代价也差三个量级，必须分开数：
    //   ① 起点就在阻挡格 / 越界 ⇒ O(1) 立即返回；
    //   ② 目标在阻挡格 / 越界 ⇒ 同样 O(1)（调用方会螺旋找附近可行格再试一次）；
    //   ③ 起点目标都合法但**被围死** ⇒ 展开可达的整片连通域（最贵，才是 #25 的尖峰来源）。
    // 只有①②时，那 100% 失败是"快速失败"，CPU 尖峰另有原因；③占多数则尖峰就是它。
    if !map.is_passable(start) {
        note_astar_fail(AstarFailReason::StartBlocked);
        return None;
    }
    if !map.is_passable(goal) {
        note_astar_fail(AstarFailReason::GoalBlocked);
        return None;
    }
    if start == goal {
        return Some(vec![start]);
    }

    let width = map.width();
    let cell_count = width * map.height();
    let start_idx = map.index(start);
    let goal_idx = map.index(goal);

    // 🔴 2026-09-25 改（未结案 #25，**实测数据支持的那一条**）：把三份 O(格数) 缓冲
    // （`g_score` / `parent` / `closed`，各 16384 项）从"每次调用分配 + 清零"改成
    // **线程本地 scratch + generation 戳**。
    //
    // 依据（独显 `perf_run.ps1 -Secs 30` 压力模式 27 个 1s 样本 + 新增的展开计数）：
    //   `ai_us` 中位 9253µs / 最大 17235µs，而**单次调用最大只展开 9 个节点**（p95 也是 9）
    //   ⇒ 贵的不是搜索本身，而是"每秒约 300–700 次调用 × 每次 ~400KB 的分配+清零"。
    //   ⇒ 正解是 scratch 复用（原 lead②），**不是**节点预算（原 lead①：单次 9 个节点，加预算白改）。
    //
    // 语义：`stamp[i] == gen` ⇒ 节点 i 在**本次**搜索里被访问过（旧的 `closed[i]`）；
    // `g[i]`/`parent[i]` 只在 `stamp[i] == gen` 时有效 ⇒ **不必清零**，`gen` 自增即"清空"。
    // `open` 堆每次 `clear()`（O(1)，只把长度置 0）。AI 是多线程步进的（`step_ai_parallel`），
    // 所以 scratch 必须是 **thread_local**，不能是全局 `Mutex`（那会把并行 AI 串行化）。
    ASTAR_SCRATCH.with(|s| {
        let s = &mut *s.borrow_mut();
        s.begin(cell_count);
        find_path_search(map, start, goal, start_idx, goal_idx, width, s)
    })
}

/// 一次 A* 搜索（scratch 由调用方提供，见 [`ASTAR_SCRATCH`] 的说明）
fn find_path_search(
    map: &GridMap,
    start: GridPos,
    goal: GridPos,
    start_idx: usize,
    goal_idx: usize,
    width: usize,
    s: &mut AstarScratch,
) -> Option<Vec<GridPos>> {
    let AstarScratch { open, g, parent, seen, closed, gen } = s;
    let gen = *gen;
    let max_steps = map.width() * map.height();
    g[start_idx] = 0;
    seen[start_idx] = gen;
    // 🔴 起点的 `parent` **必须显式清掉**：scratch 复用后它可能还留着上一次搜索的旧前驱，
    // 而 `reconstruct_path` 会顺着链一路走上去 —— 旧链与新链接上就是一个**环**，
    // 于是 `path.push` 无限增长直到 OOM（2026-09-25 实测：16 GiB 分配失败）。
    // 旧实现每次 `vec![None; cells]` 天然没这个问题，改成 scratch 后必须自己清。
    parent[start_idx] = None;

    open.push(HeapNode {
        f: start.manhattan(goal),
        g: 0,
        index: start_idx,
    });
    // 目标不可达时的**部分路径**兜底（2026-09-25 真机实测加）：
    // 记下搜索过程中"离目标最近"的那个可达节点；堆空时回退到它，而不是返回 `None`。
    // 依据：修掉"起点落在阻挡格"之后，实测失败原因 100% 变成 `连通域穷尽`（NPC 被墙围在
    // 另一个连通域里），返回 `None` 会让 NPC 退回直行贴墙、永远停在玩家看不见的地方
    // ⇒ 波次清不掉。给出"走到最接近目标的可达格"至少让它们走到屏障边（可见、可打），
    // 而且路径非空 ⇒ 不必每 1/3 秒重规划一次（CPU 也跟着降）。
    //
    // 🔴 2026-09-25 修（**非确定性**，由 `astar_goal_surrounded_returns_partial_path` 红测抓出）：
    // 原先按**曼哈顿** h 取最小，而曼哈顿 h 在斜向上一大片格子并列（本例 (1,3)/(2,2)/(3,1)
    // 都是 h=2）⇒ 谁先出堆全看二叉堆内部顺序 ⇒ **同样的输入、同样的实现，选点可以不一样**。
    // 现在改成"直线距离² 最小，再取格子序号最小"：与堆顺序无关、可复现，
    // 且与"离目标最近"的字面语义一致（(2,2) 的 d²=2 唯一最小）。
    let mut best_idx = start_idx;
    let mut best_d2 = i64::MAX;
    // 分项计时（未结案 #25 的 lead③）：本次调用**展开了多少节点**。
    // `ai_us` 41.6ms 的尖峰既不是"起点阻挡"（那是 O(1)）也不是"连通域穷尽"（实测恒为 0），
    // 所以在拿到"展开节点数"之前不该改算法 —— 先量，再决定是否加预算/缓存。
    let mut expanded = 0u64;
    let mut pushed = 0u64;

    while let Some(node) = open.pop() {
        if closed[node.index] == gen {
            continue; // 本次搜索已出堆（旧的 `closed[i]`）
        }
        closed[node.index] = gen;
        expanded += 1;

        if node.index == goal_idx {
            note_astar_work(expanded, pushed);
            return Some(reconstruct_path(parent, width, Some(node.index), max_steps));
        }

        let cur_pos = GridPos::new((node.index % width) as i32, (node.index / width) as i32);
        let (gx, gy) = ((cur_pos.x - goal.x) as i64, (cur_pos.y - goal.y) as i64);
        let d2 = gx * gx + gy * gy;
        if (d2, node.index) < (best_d2, best_idx) {
            best_d2 = d2;
            best_idx = node.index;
        }
        for (dx, dy) in NEIGHBOR_OFFSETS {
            let next = GridPos::new(cur_pos.x + dx, cur_pos.y + dy);
            if !map.is_passable(next) {
                continue;
            }
            let next_idx = map.index(next);
            let tentative_g = node.g + 1;
            // 未访问（seen != gen）⇒ 视为 g = ∞；否则比 g 值。**不清零**，见 scratch 注释。
            if seen[next_idx] == gen && tentative_g >= g[next_idx] {
                continue;
            }
            g[next_idx] = tentative_g;
            seen[next_idx] = gen;
            parent[next_idx] = Some(node.index);
            pushed += 1;
            open.push(HeapNode {
                f: tentative_g + next.manhattan(goal),
                g: tentative_g,
                index: next_idx,
            });
        }
    }

    // 堆空 = 起点所在连通域里没有目标
    note_astar_work(expanded, pushed);
    if best_idx != start_idx {
        if diag_on() {
            ASTAR_PARTIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        return Some(reconstruct_path(parent, width, Some(best_idx), max_steps));
    }
    note_astar_fail(AstarFailReason::Exhausted);
    None
}

/// 全图**最大连通域**的掩码（行主序）。
///
/// 🔴 2026-09-25 加（真机实测驱动）：压力模式的出生点在 150–198m 环上，而那一圈正好穿过城市
/// 街区 —— `push_out_of_obstacle` 只保证"落在可通行格"，**不保证落在主域里**。实测 4 只红方
/// 里 2 只落在 **9 格 / 2 格**的小口袋里（`tmp_stress_spawn_reachability` 探针），于是
/// `aidiag: astar` 里每秒 278 次调用**全部** `连通域穷尽` —— 两军各在各的院子里"隔空对射"。
/// 修法：出生选点先算"主域"，再把它拉进主域（见 `game.rs::nearest_in_component`）。
///
/// 成本：逐格扫描 + 对每个未访问的可通行格做一次 BFS，典型只有几个域 ⇒ O(格数)。
pub fn largest_component_mask(map: &GridMap) -> Vec<bool> {
    let w = map.width();
    let h = map.height();
    let mut done = vec![false; w * h];
    let mut best: Vec<bool> = vec![false; w * h];
    let mut best_n = 0usize;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let pos = GridPos::new(x, y);
            let i = y as usize * w + x as usize;
            if done[i] || !map.is_passable(pos) {
                continue;
            }
            let mask = reachable_mask(map, pos);
            let n = mask.iter().filter(|b| **b).count();
            for (k, m) in mask.iter().enumerate() {
                if *m {
                    done[k] = true;
                }
            }
            if n > best_n {
                best_n = n;
                best = mask;
            }
        }
    }
    best
}

/// 从 `from` 出发的 4 邻域**可达掩码**（行主序，`true` = 可通行且与 `from` 连通）。
///
/// 🔴 2026-09-25 加（#17 真根因）：`find_path` 找不到路只是**症状**，真正要回答的问题是
/// "这个出生点到底走不走得到玩家"。程序化城市地图实测：中央广场被一圈低矮装饰
/// （0.6m 台沿 / 0.2m 立柱）围住，导航网格按"障碍盒覆盖整格"判定 ⇒ 玩家所在域只有
/// **24 格**，而全图可通行 13165 格；NPC 出生在 40–80m 外 ⇒ 出生即在**另一个连通域**，
/// 永远走不到玩家 ⇒ `update_waves` 等不到 `npcs.is_empty()` ⇒ 波次永远清不掉。
///
/// 用于**出生选点校验**（`game.rs::spawn_npc_ring`）与地图连通性巡检（单测）。
/// 成本 O(格数)；由调用方决定缓存策略（每波一次足够）。
pub fn reachable_mask(map: &GridMap, from: GridPos) -> Vec<bool> {
    let w = map.width();
    let h = map.height();
    let mut seen = vec![false; w * h];
    if w == 0 || h == 0 || !map.is_passable(from) {
        return seen;
    }
    let mut queue = std::collections::VecDeque::new();
    seen[from.y as usize * w + from.x as usize] = true;
    queue.push_back(from);
    while let Some(p) = queue.pop_front() {
        for (dx, dy) in NEIGHBOR_OFFSETS {
            let next = GridPos::new(p.x + dx, p.y + dy);
            if !map.is_passable(next) {
                continue;
            }
            let i = next.y as usize * w + next.x as usize;
            if seen[i] {
                continue;
            }
            seen[i] = true;
            queue.push_back(next);
        }
    }
    seen
}

/// 由 `parent` 链反推出路径（含起点与终点，顺序 start→goal）
///
/// `max_steps` = 地图格数：**防御性上界**。scratch 复用后 `parent` 是复用的数组，
/// 一旦有陈旧前驱接成了环（见 `find_path_search` 里 `parent[start_idx] = None` 的注释），
/// 这里会无限 `push` 直到 OOM（实测 16 GiB 分配失败、进程直接 abort）。
/// 有上界时最坏退化成"一条长路径"，而不是把整台机器拖死。
fn reconstruct_path(
    parent: &[Option<usize>],
    width: usize,
    mut cur: Option<usize>,
    max_steps: usize,
) -> Vec<GridPos> {
    let mut path = Vec::new();
    while let Some(i) = cur {
        if path.len() >= max_steps {
            break;
        }
        path.push(GridPos::new((i % width) as i32, (i / width) as i32));
        cur = parent[i];
    }
    path.reverse();
    path
}

/// 寻路失败的原因（诊断用；只影响 `RV3D_AI_DIAG=1` 时的计数）
#[derive(Debug, Clone, Copy)]
enum AstarFailReason {
    /// 起点落在阻挡格/越界（O(1) 返回）
    StartBlocked,
    /// 目标落在阻挡格/越界（O(1) 返回）
    GoalBlocked,
    /// 两端都合法但连通域里找不到目标（展开整片连通域，最贵）
    Exhausted,
}

/// 记一次寻路失败（关闭诊断时**零成本**：一个 `OnceLock` 读）
fn note_astar_fail(reason: AstarFailReason) {
    if !diag_on() {
        return;
    }
    let counter = match reason {
        AstarFailReason::StartBlocked => &ASTAR_FAIL_START,
        AstarFailReason::GoalBlocked => &ASTAR_FAIL_GOAL,
        AstarFailReason::Exhausted => &ASTAR_FAIL_EXHAUSTED,
    };
    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
    /// 🔴 **目标已知**（2026-09-23 补，未结案 #17 的修法②）：与"看得见"**分两条通道**。
    ///
    /// 进攻方（波次/防守波里的敌军）从出生起就知道要打哪 —— 玩家就是它的任务目标，
    /// 不需要靠视距去"发现"。它只影响 `Idle/Patrol → Chase` 与 `Chase` 的**维持**：
    /// 目标已知的 NPC 会一路推进（A* 走位）去找人。
    ///
    /// ⚠️ **开火仍然只认 [`Self::enemy_visible`]**（视距 + 遮挡）：`Chase → Attack` 必须
    /// 同时满足"看得见"，否则就重演历史上的"隔墙掉血"（见 `game.rs` 里 `target_occluded`
    /// 那段注释）。这一条是"目标已知"与"敌人可见"的分工边界，别把 Attack 的入口放宽。
    ///
    /// 默认 `false` ⇒ 不填这一项的调用方（单测、压力模式、开始菜单游走）行为与旧版逐条一致。
    pub target_known: bool,
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
/// - `Idle/Patrol → Chase`：发现敌人 `enemy_visible`，**或目标已知 `target_known`**
/// - `Chase → Attack`：发现敌人且在攻击距离内（`enemy_visible && enemy_in_range`）
/// - `Attack → Chase`：敌人仍在视野但脱离攻击距离；**或目标已知但暂时看不见**（去重新找视线）
/// - `Patrol/Chase/Attack → Idle`：巡逻完成 / 丢失敌人**且目标未知**
///
/// 🔴 `target_known`（2026-09-23，未结案 #17 的修法②）是"知道要打谁"，
/// `enemy_visible` 是"现在看得见"。**只有后者能开火** —— 两个条件在 `Chase → Attack`
/// 处必须同时成立，否则就回到"隔着整栋楼输出"的老 bug。
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
    ///
    /// ⚠️ `target_known == false` 时，下面每个分支都与加入该字段之前**逐条等价**
    /// （旧行为由"目标未知"这一支完整保留）：单测、压力模式、开始菜单游走都不受影响。
    pub fn update(&mut self, perception: NpcPerception) -> NpcState {
        self.state = match self.state {
            NpcState::Idle => {
                if perception.enemy_visible || perception.target_known {
                    NpcState::Chase
                } else if perception.start_patrol {
                    NpcState::Patrol
                } else {
                    NpcState::Idle
                }
            }
            NpcState::Patrol => {
                if perception.enemy_visible || perception.target_known {
                    NpcState::Chase
                } else if perception.patrol_finished {
                    NpcState::Idle
                } else {
                    NpcState::Patrol
                }
            }
            NpcState::Chase => {
                // 看不见目标时：目标已知 ⇒ 继续追（推进找视线）；否则丢失敌人回 Idle
                if !perception.enemy_visible {
                    if perception.target_known {
                        NpcState::Chase
                    } else {
                        NpcState::Idle
                    }
                } else if perception.enemy_in_range {
                    // 🔴 开火入口：这里**必须**已经通过 `enemy_visible`（上个分支），
                    // `enemy_in_range` 单独成立不算数（隔墙不得进入 Attack）
                    NpcState::Attack
                } else {
                    NpcState::Chase
                }
            }
            NpcState::Attack => {
                if !perception.enemy_visible {
                    // 视线丢了：有目标情报就去重新找视线（Chase），没有就回 Idle
                    if perception.target_known {
                        NpcState::Chase
                    } else {
                        NpcState::Idle
                    }
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

/// 计算包抄目标点：把「玩家 → 本单位」的方向**旋转 90°**（`side=+1` 顺时针 / `-1` 逆时针），
/// 在距玩家 `offset` 格处取点，结果 clamp 到地图范围。
///
/// 🔴 2026-09-25 重写（修 #17 的最后一个死循环）。旧版按**主导轴**二选一：
/// `|dx| >= |dy|` → `(player.x, player.y + side*off)`，否则 → `(player.x + side*off, player.y)`。
/// 问题是这两支**互不相容**：`u=(1,0)` 时垂直方向取 `+y`，而 `u=(0,1)` 时取 `+x` ——
/// 同一个旋转方向在两个象限里给出相反的垂直向量。于是本单位走过 45° 分界线时，
/// 包抄点在两个相距 √8 格的点之间跳变，NPC 就在这两点间无限来回（真机日志：
/// `tac=Flank goal=(14,2) → (2,14) → (14,2) …`，永远进不了 `Attack`）。
/// 现在改用**一致的旋转**：单位方位连续变化 ⇒ 包抄点连续变化（判据见
/// `flank_goal_never_jumps_across_the_diagonal`，跳变版红、旋转版绿）。
///
/// `side` 只决定顺/逆时针（4 邻域网格下无法只靠"左上/右下"表达侧翼，需要符号），
/// 调用方用 id 奇偶定钳形；`ambush_goal` 则按玩家朝向在两个 side 里挑背后那个。
///
/// ⚠️ **偏移量必须小于交战距离**：包抄点 = 目的地，NPC 到了就不再靠近
/// （`path_index` 走完即原地重规划）。旧调用值 3 格 = 12m 恰好等于射程 ⇒
/// 包抄手永远停在射程外一步（见 `game.rs::FLANK_OFFSET` 注释与
/// `flank_and_ambush_goals_land_inside_engage_range`）。
pub fn flank_goal(grid: &GridMap, player: GridPos, target: GridPos, side: i32, offset: u32) -> GridPos {
    let dx = (target.x - player.x) as f32;
    let dy = (target.y - player.y) as f32;
    let len = (dx * dx + dy * dy).sqrt();
    let max_x = grid.width() as i32 - 1;
    let max_y = grid.height() as i32 - 1;
    if len < 1e-3 {
        // 方向退化（玩家与本单位同格）：已经贴脸，不再绕圈，返回玩家位置
        return GridPos::new(player.x.clamp(0, max_x), player.y.clamp(0, max_y));
    }
    let (ux, uy) = (dx / len, dy / len);
    // 顺时针 90°（+1）= (x,y) -> (-y, x)；逆时针（-1）取其反
    let (px, py) = if side >= 0 { (-uy, ux) } else { (uy, -ux) };
    let r = offset as f32;
    let raw = GridPos::new(
        player.x + (px * r).round() as i32,
        player.y + (py * r).round() as i32,
    );
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

    /// 包装层（诊断计数）必须与实现体**结果完全一致**：计数器绝不许改变寻路结果。
    /// 这条红了 = `find_path` 的包装改坏了（例如提前 return None 或吞掉路径）。
    #[test]
    fn find_path_wrapper_matches_inner() {
        let mut map = GridMap::new(9, 9);
        for y in 0..=4 {
            map.block(GridPos::new(4, y));
        }
        let cases = [
            (GridPos::new(0, 0), GridPos::new(8, 8)), // 需绕墙
            (GridPos::new(1, 1), GridPos::new(2, 2)), // 近距
            (GridPos::new(0, 0), GridPos::new(0, 0)), // 起终点相同
            (GridPos::new(4, 6), GridPos::new(8, 8)), // 起点在阻挡格上 → 双方都 None
            (GridPos::new(0, 0), GridPos::new(4, 2)), // 终点在阻挡格上 → 双方都 None
        ];
        for (s, g) in cases {
            assert_eq!(
                find_path(&map, s, g),
                find_path_inner(&map, s, g),
                "包装层与实现体结果必须一致: {s:?} -> {g:?}"
            );
        }
    }

    /// 🔴 **契约变更（2026-09-25，真机实测驱动）**：目标被围死时**不再返回 `None`**，
    /// 而是给出"走到最接近目标的可达格"的**部分路径**。
    ///
    /// 依据（核显 survive 实测）：旧行为下被墙围死的 NPC 拿不到路径 ⇒ 退回 `direct_goal` 直行
    /// ⇒ 顶着墙、`occluded=true`、玩家打不到 ⇒ **波次永远清不掉**。
    /// 这条测试原来断言 `None`（旧契约），改契约时它**必然红** —— 那是预期的。
    #[test]
    fn astar_goal_surrounded_returns_partial_path() {
        let mut map = GridMap::new(5, 5);
        for pos in [
            GridPos::new(2, 3),
            GridPos::new(4, 3),
            GridPos::new(3, 2),
            GridPos::new(3, 4),
        ] {
            map.block(pos);
        }
        let goal = GridPos::new(3, 3);
        let start = GridPos::new(0, 0);
        let path = find_path(&map, start, goal).expect("目标被围死也必须给部分路径");
        assert_eq!(path[0], start, "路径首格 = 起点");
        let last = *path.last().expect("部分路径不该为空");
        assert_ne!(last, goal, "目标格真的走不到");
        assert_eq!(
            last,
            GridPos::new(2, 2),
            "应停在离目标最近的可达格（h=2；四个邻居 h=1 但都被封）"
        );
        for w in path.windows(2) {
            let d = (w[1].x - w[0].x).abs() + (w[1].y - w[0].y).abs();
            assert_eq!(d, 1, "部分路径也必须四方向逐格相邻: {w:?}");
            assert!(map.is_passable(w[1]));
        }
    }

    /// 目标在**另一个连通域**（整列封死）时：部分路径应停在最贴近目标的墙边格。
    #[test]
    fn astar_partial_path_stops_at_the_barrier() {
        let mut map = GridMap::new(9, 9);
        for y in 0..9 {
            map.block(GridPos::new(4, y));
        }
        let start = GridPos::new(1, 4);
        let goal = GridPos::new(7, 4); // 右半域，不可达
        assert!(map.is_passable(start) && map.is_passable(goal), "两端本身要合法");
        let path = find_path(&map, start, goal).expect("不可达目标也要给部分路径");
        assert!(path.len() > 1, "应当真的走了一段: {path:?}");
        assert_eq!(*path.last().unwrap(), GridPos::new(3, 4), "停在墙左侧最近格");
    }

    /// 真·穷尽（起点自己就是孤岛）：仍然返回 `None`（这才是 `fails` 该计的那种）
    #[test]
    fn astar_returns_none_only_when_start_is_an_island() {
        let mut island = GridMap::new(5, 5);
        for y in 0..5 {
            for x in 0..5 {
                if (x, y) != (1, 1) {
                    island.block(GridPos::new(x, y));
                }
            }
        }
        assert!(island.is_passable(GridPos::new(1, 1)));
        assert_eq!(
            find_path(&island, GridPos::new(1, 1), GridPos::new(3, 3)),
            None,
            "四周全封 ⇒ 连一个可达的更近格都没有 ⇒ None"
        );
    }

    /// `passable_or_nearest`：可通行原样返回；不可通行时按**环由近及远**找最近的可通行格。
    /// 这条红了 = "NPC 站在阻挡格里就永远拿不到路径"（实测 108/108）会复发。
    #[test]
    fn passable_or_nearest_finds_deterministic_nearest() {
        let mut map = GridMap::new(9, 9);
        for y in 3..=5 {
            for x in 3..=5 {
                map.block(GridPos::new(x, y));
            }
        }
        // 可通行 ⇒ 原样返回
        assert_eq!(
            passable_or_nearest(&map, GridPos::new(0, 0), 8),
            Some(GridPos::new(0, 0))
        );
        // 被 3×3 封住的中心：环 1 全封，环 2 上直线距离最小的是边中点 (4,2)（d²=4）
        let p = GridPos::new(4, 4);
        assert!(!map.is_passable(p));
        let n = passable_or_nearest(&map, p, 8).expect("环 2 上应有可通行格");
        assert!(map.is_passable(n), "返回值必须可通行");
        assert_eq!(n, GridPos::new(4, 2), "确定性：环内取直线距离最小、顺序固定");
        // 半径不够 ⇒ None（调用方据此知道"救不回来"）
        assert_eq!(passable_or_nearest(&map, p, 1), None, "半径 1 全封");
        // 越界输入：要么 None，要么给一个真的可通行格（不能原样返回越界点）
        let oob = passable_or_nearest(&map, GridPos::new(-5, -5), 8);
        assert!(oob.is_none() || map.is_passable(oob.unwrap()));
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

    /// 🔴 未结案 #17 的修法②（2026-09-23）：目标**已知**但要靠推进去获得视线。
    /// 这条是"进攻方不该靠视距才知道要打哪"的最小判据。
    #[test]
    fn target_known_makes_npc_advance_instead_of_patrolling() {
        // Idle + 目标已知 → Chase（不需要 enemy_visible）
        let mut fsm = NpcStateMachine::new();
        let mut p = perception();
        p.target_known = true;
        assert_eq!(fsm.update(p), NpcState::Chase);

        // Chase + 目标已知 + 看不见（太远/被挡）→ 保持 Chase（继续推进），不回 Idle
        assert_eq!(fsm.update(p), NpcState::Chase);
        assert_eq!(fsm.update(p), NpcState::Chase);

        // Patrol + 目标已知 → Chase（巡逻中的也一样）
        let mut fsm = NpcStateMachine::new();
        let mut p = perception();
        p.start_patrol = true;
        assert_eq!(fsm.update(p), NpcState::Patrol);
        let mut p = perception();
        p.target_known = true;
        assert_eq!(fsm.update(p), NpcState::Chase);
    }

    /// 🔴 分工边界：`target_known` **绝不能**变成开火许可。
    /// 目标已知、距离够近、但看不见（被墙挡住）⇒ 只能 Chase，不能 Attack。
    /// 这条红了就等于"隔着整栋楼输出"那个老 bug 回来了。
    #[test]
    fn target_known_alone_never_enters_attack() {
        let mut fsm = NpcStateMachine::new();
        let mut p = perception();
        p.target_known = true;
        p.enemy_in_range = true; // 距离已在攻击范围内，但看不见
        assert_eq!(fsm.update(p), NpcState::Chase, "看不见就不许进 Attack");

        // 一旦看得见（且仍已知）⇒ 立刻进 Attack
        p.enemy_visible = true;
        assert_eq!(fsm.update(p), NpcState::Attack);

        // 视线又丢（例如目标躲到墙后）⇒ 回 Chase 去重新找视线（而不是回 Idle 忘记目标）
        p.enemy_visible = false;
        assert_eq!(fsm.update(p), NpcState::Chase);
    }

    /// 目标**未知**时（默认值）：与加入 `target_known` 字段之前逐条一致 ——
    /// 压力模式 / 开始菜单游走 / 旧单测都靠这条。
    #[test]
    fn target_unknown_keeps_legacy_transitions() {
        let mut fsm = NpcStateMachine::new();
        // Idle 不动
        assert_eq!(fsm.update(perception()), NpcState::Idle);
        // 看不见 ⇒ 从 Chase 掉回 Idle
        let mut p = perception();
        p.enemy_visible = true;
        assert_eq!(fsm.update(p), NpcState::Chase);
        assert_eq!(fsm.update(perception()), NpcState::Idle);
        // 看不见时即使距离够近也不进 Attack，也不维持 Chase
        let mut p = perception();
        p.enemy_in_range = true;
        assert_eq!(fsm.update(p), NpcState::Idle);
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

    /// 包抄点 = 「玩家 → 本单位」旋转 90°（一致旋转）+ 地图 clamp。
    ///
    /// ⚠️ 2026-09-25 随 `flank_goal` 重写更新：旧断言把**不一致**的"主导轴"写法固化了下来
    /// （水平主导取 `+y`、垂直主导取 `+x`）。改成一致旋转后，`u=(1,0)` 的两条断言不变，
    /// `u=(0,1)` 的两条**左右互换**（旋转方向必须自洽，这正是修掉跳变的前提）。
    #[test]
    fn flank_goal_rotates_and_clamps() {
        let map = GridMap::new(10, 10);
        // 本单位在玩家东侧（u=(1,0)）：顺时针 90° → +y
        assert_eq!(
            flank_goal(&map, GridPos::new(2, 2), GridPos::new(7, 2), 1, 2),
            GridPos::new(2, 4)
        );
        assert_eq!(
            flank_goal(&map, GridPos::new(2, 2), GridPos::new(7, 2), -1, 2),
            GridPos::new(2, 0)
        );
        // 本单位在玩家南侧（u=(0,1)）：同一个旋转给出 -x（与上一支构成自洽的 90° 旋转）
        assert_eq!(
            flank_goal(&map, GridPos::new(4, 4), GridPos::new(4, 9), 1, 2),
            GridPos::new(2, 4)
        );
        assert_eq!(
            flank_goal(&map, GridPos::new(4, 4), GridPos::new(4, 9), -1, 2),
            GridPos::new(6, 4)
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
            flank_goal(&map, GridPos::new(9, 9), GridPos::new(0, 9), -1, 5),
            GridPos::new(9, 9),
            "正方向越界应 clamp 到上界"
        );
        // 玩家与本单位同格：退化不再绕圈，返回玩家位置
        assert_eq!(
            flank_goal(&map, GridPos::new(3, 3), GridPos::new(3, 3), 1, 2),
            GridPos::new(3, 3)
        );
    }

    /// 🔴 包抄点必须**随本单位方位连续变化**（#17 最后一个死循环的红证）。
    ///
    /// 旧实现按「玩家→本单位」的**主导轴**二选一（`|dx|>=|dy|` → 沿 y 偏移，否则沿 x 偏移），
    /// 于是单位走过 45° 分界线时包抄点在两个相距 √8 格的点之间**跳变**。实机后果（2026-09-25
    /// `RV3D_AI_DIAG=1` 真机日志）：
    /// `tac=Flank goal=(14.0,2.0)` → `(2.0,14.0)` → `(14.0,2.0)` … 无限来回，
    /// NPC 在离玩家 15–22m 处走了一整个波次，永远进不了 `Attack`（射程 12m），
    /// 波次永远清不掉。
    ///
    /// 判据：单位绕玩家一圈（固定半径 8 格，逐度采样），相邻两次包抄点不得超过 2 格。
    /// 跳变版 = √8 ≈ 2.83 格（红）；连续旋转版 ≤ √2 ≈ 1.42 格（绿）。
    #[test]
    fn flank_goal_never_jumps_across_the_diagonal() {
        let grid = GridMap::new(128, 128);
        let player = GridPos::new(64, 64);
        let radius = 8.0f32;
        let mut prev: Option<GridPos> = None;
        for deg in 0..360 {
            let a = (deg as f32).to_radians();
            let npc = GridPos::new(
                player.x + (radius * a.cos()).round() as i32,
                player.y + (radius * a.sin()).round() as i32,
            );
            let g = flank_goal(&grid, player, npc, 1, 2);
            if let Some(p) = prev {
                let d = (((g.x - p.x).pow(2) + (g.y - p.y).pow(2)) as f32).sqrt();
                assert!(
                    d <= 2.0,
                    "包抄点在 {deg}° 处跳变：{p:?} -> {g:?}（相距 {d} 格）"
                );
            }
            prev = Some(g);
        }
    }

    /// 可达掩码：4 邻域连通、阻挡格排除、起点不可通行时**全 false 且不 panic**。
    /// 它是"出生点走不走得到玩家"的唯一判据（见 `reachable_mask` 的文档）。
    #[test]
    fn reachable_mask_matches_grid_connectivity() {
        let mut map = GridMap::new(5, 5);
        // 中间一整列封死 ⇒ 左侧可达、右侧不可达
        for y in 0..5 {
            map.block(GridPos::new(2, y));
        }
        let mask = reachable_mask(&map, GridPos::new(0, 0));
        let at = |x: i32, y: i32| mask[y as usize * 5 + x as usize];
        assert!(at(0, 0) && at(0, 4) && at(1, 4), "左侧连通域应全部可达");
        assert!(!at(2, 0), "阻挡格自身不可达");
        assert!(!at(3, 0) && !at(4, 0) && !at(4, 4), "被墙隔开的右侧不该可达");
        // 起点本身不可通行 ⇒ 全 false（不 panic、不越界）
        let none = reachable_mask(&map, GridPos::new(2, 2));
        assert_eq!(none.len(), 25);
        assert!(none.iter().all(|b| !b), "起点在墙里时不该有可达格");
        // 越界起点同样安全
        assert!(reachable_mask(&map, GridPos::new(-1, 0)).iter().all(|b| !b));
    }

    /// 🔴 **scratch 复用必须无状态泄漏**：同一张图连续问两次"同一对起点终点"必须给出**逐格相同**
    /// 的路径；中间插入一次别的搜索（成功 / 失败 / 部分路径各一次）也不能污染下一次的结果。
    /// 这是 generation 戳实现（不清零 `g`/`parent`/`stamp`）的**唯一**回归网：
    /// 漏掉 `stamp[next] = gen` 或误用 `g_score[i]` 未判戳，都会在这里红。
    #[test]
    fn astar_scratch_reuse_is_stateless() {
        let mut map = GridMap::new(12, 12);
        for y in 2..10 {
            map.block(GridPos::new(5, y)); // 一堵竖墙，右侧只能从上下绕
        }
        let a = GridPos::new(1, 5);
        let b = GridPos::new(10, 5);
        let first = find_path(&map, a, b).expect("绕行路径应存在");
        assert!(first.len() > 1);
        // 中间穿插：目标落在墙里（O(1) 失败）+ 部分路径查询 + 另一对端点
        assert!(
            find_path(&map, GridPos::new(0, 0), GridPos::new(5, 5)).is_none(),
            "(5,5) 在墙里 ⇒ 必须 O(1) 返回 None"
        );
        let _ = find_path(&map, GridPos::new(1, 2), GridPos::new(10, 2));
        let _ = find_path(&map, GridPos::new(0, 0), GridPos::new(11, 11));
        // 再问同一对：必须逐格相同
        let again = find_path(&map, a, b).expect("第二次也必须给出路径");
        assert_eq!(first, again, "scratch 复用后同一查询必须给出同一条路径");
        // 反向也要自洽（对称图里应等长）
        let back = find_path(&map, b, a).expect("反向路径应存在");
        assert_eq!(back.len(), first.len(), "四方向网格上正反路径长度应相同");
        // 起点==终点：返回单格路径（不碰 scratch 的边界分支）
        assert_eq!(find_path(&map, a, a), Some(vec![a]));
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
        // 与 game.rs spawn_wave 早期波次一致。写死期望值，不把生产公式重抄一遍——
        // 抄过来的话，公式被改坏时测试会跟着一起改，等于什么都没测到。
        assert_eq!(wave_profile(1).count, 6, "第 1 波 4+2·1=6 人");
        assert_eq!(wave_profile(1).speed, 4.0, "第 1 波是速度曲线基准点");
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
        // 常规波主曲线保持原样（供集成回归）。
        // 2026-09-08：这里原来写的是 `(4 + 2 * 4).min(24)`——把生产公式在测试里重抄一遍，
        // 而且 4+2·4=12 恒小于 24，`.min(24)` 那个分支一次都没被验证过（clippy 以
        // const_comparisons 报"因此无效"）。改成写死期望值，并补下面三条真正顶到上限的。
        assert_eq!(wave_profile(4).count, 12, "第 4 波 4+2·4=12 人");
        assert_eq!(wave_profile(10).count, 24, "第 10 波 4+2·10=24，正好触顶");
        assert_eq!(wave_profile(11).count, 24, "第 11 波算出 26，必须被截到 24");
        assert_eq!(wave_profile(30).count, 24, "再往后也不许超编");
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
