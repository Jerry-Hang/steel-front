//! 游戏运行时中枢
//!
//! 把 weapons / ai / physics / ui / audio / network 模块接进主循环：
//! - 每帧 `update(dt, camera, fire)` 推进物理、武器、AI、音频、网络
//! - 渲染前由 main.rs 取 HUD quad 列表与光照 uniform
//!
//! 本文件只做模块间编排与少量胶水逻辑，具体算法仍留在各模块内。

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::audio::{AudioListener, AudioPlayer, AudioSource, Channel, SilentSink, SfxBank, SfxKind};
use crate::net::{
    Client, NetworkMessage, NetInput, NpcSnapshot, PlayerState, Server, CLIENT_TIMEOUT,
    SERVER_TIMEOUT,
};
use super::camera::{Camera, CameraMode};
use super::ai::{
    ambush_goal, angle_diff, find_cover_points, find_cover_shielding, find_path, flank_goal,
    pick_tactic, role_for, should_charge, wave_profile, yaw_to_target, zigzag_offset, GridMap,
    GridPos, NpcPerception, NpcState, NpcStateMachine, TacticalRole, Tactic, Team, WaveKind,
};
use super::physics::{self, Body, CollisionEvent, CollisionListener, PlayerBody, Vec3 as Pv};
use super::renderer::terrain_height_at;
use super::window::{WINDOW_HEIGHT, WINDOW_WIDTH};
use super::weapons::{Firearm, Grenade, Projectile, ProjectileWeapon, WeaponRack, GRENADE_FUSE_MAX, GRENADE_FUSE_MIN, GRENADE_SPEED};
use crate::ui::HudState;

/// 阵营显示名（击杀提示用）
fn team_name(t: Team) -> &'static str {
    match t {
        Team::Red => "RED",
        Team::Blue => "BLUE",
    }
}

/// AI 网格覆盖范围：128×128 格 × 4m = ±256m（与实例场/地形同域）
const GRID_CELL: f32 = 4.0;
const GRID_SIZE: usize = 128;
const GRID_HALF: f32 = GRID_CELL * (GRID_SIZE as f32) * 0.5;
/// NPC 数量
const NPC_COUNT: usize = 8;
/// NPC 发现玩家距离（米）
const NPC_SIGHT: f32 = 60.0;
/// 波间倒计时（秒）
const WAVE_INTERMISSION: f32 = 3.0;
/// 击杀得分
const KILL_SCORE: u64 = 10;
/// 清波奖励分
const WAVE_CLEAR_BONUS: u64 = 25;
/// 爆炸参数：过期投射物/命中障碍触发的 AoE（半径 8m，中心 60 伤害，衰减按冲击波压力）。
/// M1 单发 25 伤害 → 爆心一发约 2.4 倍伤害，边缘递减；推挤速度见 KNOCKBACK_SPEED。
const EXPLOSION_RADIUS: f32 = 8.0;
const EXPLOSION_DAMAGE: f32 = 60.0;
const EXPLOSION_LIFETIME: f32 = 0.35;
/// 冲击波击退速度（爆心处，m/s；指数衰减率 -12/s，约 0.25s 内衰减到 5%）
const KNOCKBACK_SPEED: f32 = 14.0;
const KNOCKBACK_DECAY: f32 = 12.0;
/// 玩家震屏：冲击半径（m）与强度（世界位移米数，随剩余时间线性衰减）
const SHAKE_RADIUS: f32 = 14.0;
const SHAKE_STRENGTH: f32 = 0.35;
const SHAKE_DURATION: f32 = 0.3;
/// 玩家移动速度（米/秒，第一人称 WASD）
const PLAYER_SPEED: f32 = 6.0;
/// NPC 就近掩体搜索半径（网格格数）
const COVER_MAX_DIST: u32 = 10;
/// 压力模式掩体搜索半径（网格格数）：NPC 战场开阔（150m 外出生），
/// 沿目标方向找遮挡掩体需覆盖障碍环带（58-130m）——40m 不够，放宽到 35 格 = 140m。
const STRESS_COVER_MAX_DIST: u32 = 35;
/// 手榴弹爆炸半径（米）与 AoE 伤害（近距可秒标准 NPC 100HP）
const GRENADE_EXPLOSION_RADIUS: f32 = 8.0;
const GRENADE_EXPLOSION_DAMAGE: f32 = 120.0;
/// 爆炸对障碍的伤害系数（障碍血量大，爆炸冲击按比例折算；半径内线性衰减）。
/// 1.0 = 手榴弹 120 伤爆心可摧毁 150HP 障碍（掩体可炸毁，符合"爆炸可摧毁/破坏掩体"）
const EXPLOSION_OBSTACLE_FACTOR: f32 = 1.0;
/// 玩家自伤伤害：距离衰减系数 + 封顶值（玩家 100HP，手榴弹 120 伤 × 0.35 ≈ 42 最大自伤，
/// 不会秒杀自己；NPC 爆炸对玩家同样生效——爆炸中心偏移保证实际伤害通常更低）
const SELF_DAMAGE_FACTOR: f32 = 0.35;
const SELF_DAMAGE_CAP: f32 = 45.0;
/// 掩体利用触发距离（米）：Chase 态距目标 ≤ 攻击距离 + 该值时先寻障碍环带掩体
const COVER_SEEK_RANGE: f32 = 20.0;
/// 玩家准星对准判定角（弧度，≈14°）
const AIM_ANGLE: f32 = 0.25;
/// 低血量撤退阈值（hp 占比）
const LOW_HP_RATIO: f32 = 0.35;
/// 火力威胁感知半径（米）：子弹水平距离小于该值且朝 NPC 飞来
const THREAT_RADIUS: f32 = 10.0;
/// 躲避触发距离（米）
const DODGE_TRIGGER_DIST: f32 = 30.0;
/// 受击躲避持续（秒）
const DODGE_HIT_TIME: f32 = 0.5;
/// 火力威胁躲避持续（秒）
const DODGE_THREAT_TIME: f32 = 0.35;
/// 两次躲避最小间隔（秒）
const DODGE_COOLDOWN: f32 = 2.0;
/// 并行 AI 更新阈值：NPC 数 ≥ 该值走亲和线程池分块并行（ai_pool）
/// （普通波次远小于此，保持单线程串行 → 冒烟行为不变）
const PARALLEL_AI_MIN: usize = 32;

/// 远组降频周期（第 3 步）：无感知、非追击/攻击、非受击/被瞄准的远 NPC
/// 每 `AI_FAR_DECIMATE` 帧步进一次（确定性按 npc.id 分帧），其余帧冻结省 CPU。
const AI_FAR_DECIMATE: u32 = 4;
/// 压力模式出生环半径（米）：超出障碍环带 58–130m，两军对垒区干净
const STRESS_SPAWN_RADIUS: f32 = 150.0;
/// 压力模式视野半径（米）：全场可见（512m 场地），保证 64v64 出生后立即交火
const STRESS_SIGHT: f32 = 512.0;
/// 锯齿机动触发距离（米）
const ZIGZAG_DIST: f32 = 40.0;
/// 常规锯齿幅度（米）
const ZIGZAG_AMP: f32 = 1.5;
/// 被瞄准/火力威胁时的锯齿幅度（米）
const ZIGZAG_AMP_HIGH: f32 = 2.5;
/// 侧翼包抄偏移（格，12m）
const FLANK_OFFSET: u32 = 3;
/// 偷袭绕背偏移（格，20m）
const AMBUSH_OFFSET: u32 = 5;
/// 脚步声音效限频间隔（秒）
const FOOTSTEP_INTERVAL: f32 = 0.5;
/// 每关波次数：清完 WAVES_PER_LEVEL 波升关，难度按累计有效波次递进（跨关不回落）
const WAVES_PER_LEVEL: u32 = 3;
/// 程序化障碍环带内半径（米）。
///
/// 必须 > NPC 最大攻击距离(16) + 掩体搜索半径(40) = 56：否则攻击态 NPC 会就近跑去掩体，
/// 不再原地站定（冒烟依赖 `npc: #id stand` 日志瞄准点射）；同时保证玩家出生点附近弹道无阻挡。
const MAP_RING_INNER: f32 = 58.0;
/// 程序化障碍环带外半径（米）：第 1 关（冒烟基准）障碍簇最远落点；其余关卡按主题轮换。
/// 与 MAP_RING_INNER 一起界定"障碍环带"，掩体利用评估（pick_attack_cover）按此过滤。
const MAP_RING_OUTER: f32 = 130.0;
/// 障碍簇数量基数：实际簇数 = MAP_CLUSTERS + seed % 5（6..=10）
const MAP_CLUSTERS: u32 = 6;
/// 障碍盒高度（米）
const MAP_BLOCK_HEIGHT: f32 = 2.4;

/// 网络环回演示（仅 RV3D_NET=1|demo 启用）：同进程 Server + Client
struct NetworkDemo {
    server: Server,
    client: Client,
    seq: u32,
    last_log: f32,
}

/// 游戏主状态机（开始菜单 → 游戏中 → 死亡/胜利/失败结算）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    /// 开始菜单：任意键开始
    StartMenu,
    /// 加载关卡数据（RV3D_MAP/RV3D_MAPS 关卡系统；同步加载，瞬间完成）
    LoadingMap,
    /// 游戏中：波次战斗
    Playing,
    /// 死亡结算：R 重开
    GameOver,
    /// 关卡胜利结算（获胜方）：R 重开本关 / N 下一关
    Victory(crate::engine::ai::Team),
    /// 关卡失败结算（时间到/规则失败）：R 重开本关
    Defeat,
}

/// 单个 NPC：A* 路径 + 状态机 + 世界位置（y 采样地形高度）
pub struct Npc {
    /// 全局 id（也用于确定性巡逻相位）
    pub id: usize,
    /// 世界坐标（x, y, z），y 每帧采样地形高度
    pub position: [f32; 3],
    /// 移动速度（米/秒）
    pub speed: f32,
    /// 攻击距离（米）
    pub attack_range: f32,
    /// 巡逻基准点（x, z）
    pub home: [f32; 2],
    /// 状态机
    pub state_machine: NpcStateMachine,
    /// 本帧感知输入
    pub perception: NpcPerception,
    /// 当前 A* 路径（网格坐标）
    pub path: Vec<GridPos>,
    /// 路径游标
    path_index: usize,
    /// 当前血量
    pub hp: f32,
    /// 血量上限（出生时设定，随波次递进）
    pub max_hp: f32,
    /// 战术角色（每波确定性分配，见 ai::role_for）
    pub role: TacticalRole,
    /// 当前战术（移动态行为，每帧由 pick_tactic 决策）
    pub tactic: Tactic,
    /// 受击/火力威胁后的侧向躲避剩余时间（秒）
    dodge_timer: f32,
    /// 两次躲避的最小间隔倒计时（秒）
    hit_cooldown: f32,
    /// 上一帧血量（受击检测）
    last_hp: f32,
    /// 阵营（普通波次全为 Red；压力模式红蓝对抗）
    pub team: Team,
    /// 朝向角（绕 Y 轴旋转，约定 atan2(dx, dz)，渲染士兵模型用）
    pub facing: f32,
    /// 对目标开火累计时间（压力模式 NPC 互射，每满 1 秒结算一次 dps）
    fire_accum: f32,
    /// 爆炸冲击波推挤速度（世界坐标 x/z 分量，m/s；advance_npc 每帧指数衰减）
    pub knockback: [f32; 2],
    /// 投掷手榴弹冷却（秒，>0 递减；=0 时低概率投掷，见 update_ai npc_throw_grenades）
    pub grenade_timer: f32,
}

/// AI 分层调度优先级（线程优化第 1 步，2026-08-11）：
/// Near = 与玩家/敌对目标实时交互或距离近（延迟敏感，走 P 核 / CCD0 簇，每帧步进）；
/// Far = 距离远且当前无交互（延迟不敏感，走 E 核 / CCD1，可降频）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AiTier {
    Near,
    Far,
}

/// 分层阈值参数（可配置；接入双池调度时由主循环/配置注入）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AiTierParams {
    /// 近档半径（米）：到目标距离 ≤ 该值 → Near
    pub near_radius: f32,
}

impl Default for AiTierParams {
    fn default() -> Self {
        Self { near_radius: 100.0 }
    }
}

/// 纯函数：单个 NPC 分层判定。
/// `dist_sq` 为到目标（玩家或敌对 NPC）的距离平方；`interacting` 表示当前与目标存在
/// 实时交互（攻击态 / 感知到敌人 / 受击 / 被瞄准等）。交互中一律 Near（每帧步进），
/// 否则按距离阈值划分。
pub fn classify_ai_tier(dist_sq: f32, interacting: bool, params: &AiTierParams) -> AiTier {
    if interacting || dist_sq <= params.near_radius * params.near_radius {
        AiTier::Near
    } else {
        AiTier::Far
    }
}

/// 就地稳定分区：Near 在前、Far 在后，返回 Near 段长度；组内保持原相对顺序。
/// 各 NPC 独立读写（AiStepCtx 只读），重排不改变步进语义。泛型便于纯逻辑单测。
pub fn partition_ai_tiers<T>(items: &mut [T], tier_of: impl Fn(&T) -> AiTier) -> usize {
    items.sort_by_key(|it| tier_of(it));
    items.iter().filter(|it| tier_of(it) == AiTier::Near).count()
}

/// 分层判定：NPC 是否与玩家实时交互。
/// 普通模式目标恒为玩家（追击/攻击/感知/被瞄准/受击/被子弹威胁均算交互）；
/// 压力模式远处红蓝互射不算（玩家无敌旁观），仅玩家直接作用（瞄准/命中/子弹威胁）
/// 才算交互——互射 NPC 归远组（CCD1/E 核），不挤占玩家所在簇。
fn ai_tier_of(npc: &Npc, player: &glam::Vec3, stress: bool, params: &AiTierParams) -> AiTier {
    let dx = npc.position[0] - player.x;
    let dz = npc.position[2] - player.z;
    let dist_sq = dx * dx + dz * dz;
    let attacking_player = matches!(
        npc.state_machine.state(),
        NpcState::Chase | NpcState::Attack
    );
    let interacting = if stress {
        npc.perception.player_aiming || npc.perception.took_hit || npc.perception.under_fire
    } else {
        attacking_player
            || npc.perception.enemy_visible
            || npc.perception.player_aiming
            || npc.perception.took_hit
            || npc.perception.under_fire
    };
    classify_ai_tier(dist_sq, interacting, params)
}

/// 远组降频判定（第 3 步）：无感知、非追击/攻击、非受击/被瞄准的远 NPC
/// 每 `AI_FAR_DECIMATE` 帧步进一次（确定性按 npc.id 分帧，`frame % N == id % N`
/// 的帧才步进）；交互中 NPC 恒每帧（红线：攻击态/接火必须每帧）。
fn should_decimate_far(npc: &Npc, frame: u32) -> bool {
    if npc.perception.enemy_visible
        || npc.perception.took_hit
        || npc.perception.under_fire
        || npc.perception.player_aiming
        || matches!(npc.state_machine.state(), NpcState::Chase | NpcState::Attack)
    {
        return false;
    }
    frame % AI_FAR_DECIMATE != (npc.id as u32) % AI_FAR_DECIMATE
}

/// 爆炸实体：冲击波 AoE 伤害 + 径向击退（生成时一次性结算），
/// 存活期内由 main.rs 生成膨胀淡出的闪光 marker（复用主 pipeline）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Explosion {
    /// 爆心（世界坐标）
    pub center: [f32; 3],
    /// 冲击波半径（米）
    pub radius: f32,
    /// 爆心最大伤害（沿冲击波压力衰减）
    pub max_damage: f32,
    /// 已存在时间（秒，驱动闪光膨胀淡出）
    pub age: f32,
    /// 视觉持续时间（秒，age 达到后实体移除）
    pub lifetime: f32,
}

/// 世界坐标 → 网格坐标（±256 → 0..128）
pub fn world_to_grid(x: f32, z: f32) -> GridPos {
    let gx = ((x + GRID_HALF) / GRID_CELL).floor() as i32;
    let gz = ((z + GRID_HALF) / GRID_CELL).floor() as i32;
    GridPos::new(gx.clamp(0, GRID_SIZE as i32 - 1), gz.clamp(0, GRID_SIZE as i32 - 1))
}

/// 网格坐标 → 世界坐标（格中心）
pub fn grid_to_world(g: GridPos) -> (f32, f32) {
    let x = (g.x as f32 + 0.5) * GRID_CELL - GRID_HALF;
    let z = (g.y as f32 + 0.5) * GRID_CELL - GRID_HALF;
    (x, z)
}

/// 障碍种类：决定摆放形态与尺寸（渲染侧 marker 颜色由 main.rs 按 kind 映射）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObstacleKind {
    /// 墙体：1..=3 个中等盒子沿切线并排成墙（第一关基准形态）
    Wall,
    /// 大块：1..=2 个大尺寸独立方块（密度高、体型大）
    Block,
    /// 路障：2..=4 个长条薄墙（单簇更长、更稀疏）
    Barrier,
}

/// 障碍基础血量：按种类区分（墙 150 / 大块 300 / 路障 100）。
/// M1 步枪单发 25 伤害 → 6/12/4 发击穿；hp 归 0 即摧毁，从碰撞/阻挡/渲染中移除。
fn obstacle_max_hp(kind: ObstacleKind) -> f32 {
    match kind {
        ObstacleKind::Wall => 150.0,
        ObstacleKind::Block => 300.0,
        ObstacleKind::Barrier => 100.0,
    }
}

/// 程序化地图上的静态障碍盒（AABB：世界坐标中心 + x/z 半尺寸，贴地高度 MAP_BLOCK_HEIGHT）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapObstacle {
    pub x: f32,
    pub z: f32,
    pub half_w: f32,
    pub half_d: f32,
    /// 障碍种类（第一关全部为 Wall；主题轮换见 theme_for_level）
    pub kind: ObstacleKind,
    /// 血量上限（按种类，见 obstacle_max_hp）
    pub max_hp: f32,
    /// 当前血量（归 0 → 摧毁：从物理刚体/AI 网格/渲染 marker 中移除）
    pub hp: f32,
}

/// 程序化关卡布局：确定性（种子 = 关卡号），障碍全部位于中央安全环带之外
#[derive(Debug, Clone, Default)]
pub struct LevelMap {
    pub obstacles: Vec<MapObstacle>,
}

/// 任务目标：本关（普通波次）/本轮（压力模式）需歼灭的敌人数。
/// 达成 → 一次性胜利日志 + HUD 横幅；不阻断波次生成与补员逻辑。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissionObjective {
    /// 需歼灭的敌人总数（0 = 未启用）
    pub target: u32,
    /// 已歼灭数（封顶在 target）
    pub eliminated: u32,
    /// 是否已完成（达成时置位，只触发一次横幅/日志）
    pub done: bool,
}

impl MissionObjective {
    /// 新建任务目标
    pub fn new(target: u32) -> Self {
        Self {
            target,
            eliminated: 0,
            done: false,
        }
    }

    /// 登记 `kills` 名敌人被歼灭；返回本次调用是否首次达成目标
    pub fn progress(&mut self, kills: u32) -> bool {
        if self.done {
            return false;
        }
        self.eliminated = (self.eliminated + kills).min(self.target);
        if self.target > 0 && self.eliminated >= self.target {
            self.done = true;
            true
        } else {
            false
        }
    }
}

/// 确定性 LCG（与 audio.rs 同款常数，零第三方依赖）：同一种子恒同布局，可测试
fn map_lcg_next(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *state
}

/// [0,1) 确定性随机数（取 LCG 高 24 位，避免低位周期过短）
fn map_lcg_unit(state: &mut u32) -> f32 {
    (map_lcg_next(state) >> 8) as f32 / (1u32 << 24) as f32
}

/// 把障碍盒径向推出安全环（若侵入 `ring_inner` 以内则沿原方向外推），并 clamp 到地图范围
fn push_out_of_safe_ring(ob: &mut MapObstacle, ring_inner: f32) {
    let d = (ob.x * ob.x + ob.z * ob.z).sqrt();
    if d < ring_inner && d > 1e-4 {
        let k = ring_inner / d;
        ob.x *= k;
        ob.z *= k;
    }
    ob.x = ob.x.clamp(-240.0, 240.0);
    ob.z = ob.z.clamp(-240.0, 240.0);
}

/// 关卡主题：安全环半径 / 障碍密度 / 种类按关卡轮换（第一关固定为冒烟基准）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapTheme {
    /// 安全环内半径（米）：环内保持无障碍（NPC 站定 + 玩家出生点弹道无阻挡）
    pub ring_inner: f32,
    /// 障碍环带外半径（米）
    pub ring_outer: f32,
    /// 障碍簇数量基数：实际簇数 = base + (seed 派生值) % var
    pub clusters_base: u32,
    /// 障碍簇数量浮动幅度
    pub clusters_var: u32,
    /// 障碍种类（决定盒子尺寸与簇形态）
    pub kind: ObstacleKind,
}

/// 关卡主题轮换：每 3 关一个周期。
///
/// 第 1 关 = 冒烟基准主题（58m 安全环 + 现状墙体布局），见 MAP_RING_INNER 注释；
/// 第 2/3 关依次切换大块/路障主题，安全环半径与密度同步变化（安全环都不低于 58m，
/// 保证任意关卡攻击态 NPC 站定与出生点弹道规则一致）。
pub fn theme_for_level(level: u32) -> MapTheme {
    match (level.saturating_sub(1)) % 3 {
        0 => MapTheme {
            ring_inner: MAP_RING_INNER,
            ring_outer: MAP_RING_OUTER,
            clusters_base: MAP_CLUSTERS,
            clusters_var: 5,
            kind: ObstacleKind::Wall,
        },
        1 => MapTheme {
            ring_inner: 64.0,
            ring_outer: 125.0,
            clusters_base: 8,
            clusters_var: 4,
            kind: ObstacleKind::Block,
        },
        _ => MapTheme {
            ring_inner: 60.0,
            ring_outer: 120.0,
            clusters_base: 9,
            clusters_var: 4,
            kind: ObstacleKind::Barrier,
        },
    }
}

/// 障碍种类对应的摆放风格：簇内盒数范围 + 盒子半尺寸范围 + 盒间间隙
#[derive(Debug, Clone, Copy)]
struct KindStyle {
    min_boxes: u32,
    max_boxes: u32,
    min_w: f32,
    max_w: f32,
    min_d: f32,
    max_d: f32,
    gap: f32,
}

/// 每种障碍的摆放风格（Wall = 第一关基准参数，勿改动）
fn kind_style(kind: ObstacleKind) -> KindStyle {
    match kind {
        ObstacleKind::Wall => KindStyle {
            min_boxes: 1,
            max_boxes: 3,
            min_w: 1.2,
            max_w: 3.2,
            min_d: 0.7,
            max_d: 1.7,
            gap: 0.6,
        },
        ObstacleKind::Block => KindStyle {
            min_boxes: 1,
            max_boxes: 2,
            min_w: 2.0,
            max_w: 4.0,
            min_d: 2.0,
            max_d: 4.0,
            gap: 1.0,
        },
        ObstacleKind::Barrier => KindStyle {
            min_boxes: 2,
            max_boxes: 4,
            min_w: 3.0,
            max_w: 5.0,
            min_d: 0.5,
            max_d: 0.8,
            gap: 0.5,
        },
    }
}

/// 程序化关卡地图：以 seed（= 关卡号）按主题生成障碍簇，分布在安全环带内。
///
/// 布局规则（注释即约定，勿随意改动）：
/// 1. 中央 theme.ring_inner 内刻意留空：攻击态 NPC 需原地站定（冒烟机制），
///    且玩家出生点/近距离战斗弹道不受阻挡（见 MAP_RING_INNER 注释）。
/// 2. 每簇沿切线方向并排 1..=3 个盒子（带间隙），形成可绕行的掩体墙。
/// 3. 盒子两两不重叠（最多重试 8 次，仍冲突则跳过），确定性可测。
pub fn generate_level_map(seed: u32) -> LevelMap {
    generate_level_map_with_theme(seed, theme_for_level(seed))
}

/// 以指定主题生成关卡布局：主题轮换测试 / 外部定制布局用。
///
/// Wall 主题的参数与 RNG 消耗顺序与旧版 generate_level_map 完全一致，
/// 保证第 1 关布局不因主题化而改变。
pub fn generate_level_map_with_theme(seed: u32, theme: MapTheme) -> LevelMap {
    let mut state = seed.wrapping_mul(0x9E37_79B9) ^ 0x5EED_1234;
    let clusters = theme.clusters_base + (state.wrapping_add(seed) % theme.clusters_var);
    let style = kind_style(theme.kind);
    let mut obstacles: Vec<MapObstacle> = Vec::new();
    let tau = std::f32::consts::TAU;
    let hp = obstacle_max_hp(theme.kind);
    for _ in 0..clusters {
        // 每簇 style 范围内个盒子，沿切线方向并排成墙/块
        let n_boxes = style.min_boxes as usize
            + (map_lcg_unit(&mut state) * (style.max_boxes - style.min_boxes + 1) as f32) as usize;
        let angle = map_lcg_unit(&mut state) * tau;
        let dir = (angle.cos(), angle.sin());
        let half_w = style.min_w + map_lcg_unit(&mut state) * (style.max_w - style.min_w);
        let half_d = style.min_d + map_lcg_unit(&mut state) * (style.max_d - style.min_d);
        let gap = style.gap;
        let span = n_boxes as f32 * (half_w * 2.0 + gap);
        // 簇中心到原点距离：内缘留出半墙余量，避免墙体侵入安全环
        let min_dist = theme.ring_inner + span * 0.5 + 1.0;
        let dist = min_dist + map_lcg_unit(&mut state) * (theme.ring_outer - min_dist).max(1.0);
        let cx = dir.0 * dist;
        let cz = dir.1 * dist;
        let tx = -dir.1; // 切线方向（垂直径向）
        let tz = dir.0;
        for i in 0..n_boxes {
            let off = (i as f32 - (n_boxes as f32 - 1.0) * 0.5) * (half_w * 2.0 + gap);
            let mut ob = MapObstacle {
                x: cx + tx * off,
                z: cz + tz * off,
                half_w,
                half_d,
                kind: theme.kind,
                max_hp: hp,
                hp,
            };
            // 冲突检测 + 抖动重试：先径向推出安全环，再与已有障碍查重叠；
            // 重叠则小幅随机移位（最多 8 次），保证放置后同时满足安全环与非重叠约束
            let mut placed = false;
            for _ in 0..8 {
                push_out_of_safe_ring(&mut ob, theme.ring_inner);
                let mut overlap = false;
                for o in &obstacles {
                    if (ob.x - o.x).abs() < ob.half_w + o.half_w
                        && (ob.z - o.z).abs() < ob.half_d + o.half_d
                    {
                        overlap = true;
                        break;
                    }
                }
                if !overlap {
                    placed = true;
                    break;
                }
                ob.x += (map_lcg_unit(&mut state) - 0.5) * 6.0;
                ob.z += (map_lcg_unit(&mut state) - 0.5) * 6.0;
            }
            if placed {
                obstacles.push(ob);
            }
        }
    }
    LevelMap { obstacles }
}

/// 碰撞事件缓冲：监听者写入，Game 每帧 drain 取走
struct EventBuffer(Arc<Mutex<Vec<CollisionEvent>>>);

impl CollisionListener for EventBuffer {
    fn on_collision(&mut self, event: &CollisionEvent) {
        if let Ok(mut buf) = self.0.lock() {
            buf.push(*event);
        }
    }
}

/// 游戏运行时状态（随接线进度逐步扩展）
pub struct Game {
    /// 物理世界：重力积分、地面响应、刚体间碰撞
    pub world: physics::World,
    /// 本帧产生的碰撞事件（drain 后清空）
    collisions: Vec<CollisionEvent>,
    /// 碰撞事件累计数（供 UI/日志）
    total_collisions: u64,
    /// 累计运行时间（秒）
    pub time: f32,
    /// 最近一帧 dt
    pub last_dt: f32,
    /// 碰撞事件缓冲（与监听者共享）
    event_buf: Arc<Mutex<Vec<CollisionEvent>>>,
    /// 上次碰撞日志时间（限频用）
    last_event_log_time: f32,
    /// 武器架：多把弹匣武器 + 切换计时（M1 Rifle + Thompson SMG，数字键 1/2 或滚轮切换）
    weapons: WeaponRack,
    /// 手榴弹库存（默认 2，上限 2；G 投掷，补给键补充）
    grenades: u32,
    /// 手榴弹上限
    grenades_max: u32,
    /// 在场投掷物（抛物线 + 引信计时）
    grenades_vec: Vec<Grenade>,
    /// 待施加到相机的后坐力（pitch/yaw 弧度，main.rs 每帧 drain 取走）
    pending_kick: (f32, f32),
    /// 第一人称玩家身体（WASD 移动 + 与演示刚体碰撞，y 每帧贴地形）
    player_body: PlayerBody,
    /// 移动输入标志（main.rs 转发 WASD；仅 Playing + FPS 生效）
    move_forward: bool,
    move_backward: bool,
    move_left: bool,
    move_right: bool,
    /// 脚步声音效限频计时
    footstep_timer: f32,
    /// 程序化合成音效库（命中/换弹/提示；枪声/脚步/环境风走 DspSynth）
    sfx: SfxBank,
    /// 在场投射物
    projectiles: Vec<Projectile>,
    /// 在场爆炸实体（AoE 结算后保留短暂生命周期供闪光渲染）
    pub explosions: Vec<Explosion>,
    /// 爆炸震屏剩余时间（秒，>0 时相机叠加抖动偏移）
    shake_timer: f32,
    /// 爆炸震屏强度（世界位移米数，随剩余时间线性衰减）
    shake_strength: f32,
    /// 开火冷却剩余时间（秒）
    fire_cooldown: f32,
    /// 发射次数（累计）
    shots: u64,
    /// 命中次数（累计，供 UI/日志）
    hits: u64,
    /// AI 导航网格（128×128，4m/格）
    grid: GridMap,
    /// NPC 列表
    pub npcs: Vec<Npc>,
    /// 上次 AI 统计日志时间（限频）
    ai_log_time: f32,
    /// 同步冲锋滞回状态（开启后需 <60% 才取消）
    charge_active: bool,
    /// NPC 数量缩放（RV3D_NPC_SCALE，默认 1.0；压测多人对战压力场景用）
    npc_scale: f32,
    /// 压力模式（RV3D_STRESS_AI）：红蓝各 `stress_sides` 名 NPC 大战场对抗
    stress: bool,
    /// 压力模式每边 NPC 数量（默认 64 → 64v64）
    stress_sides: usize,
    /// 压力模式对抗轮次（一方团灭补员后 +1）
    stress_round: u32,
    /// 并行 AI 更新开关（RV3D_AI_PARALLEL=off 关闭，可串行 A/B 对比）
    ai_parallel: bool,
    /// 性能探针：本帧各阶段耗时（µs，1Hz 日志输出，定位 CPU 侧瓶颈）
    stage_physics_us: u64,
    stage_ai_us: u64,
    stage_audio_us: u64,
    stage_net_us: u64,
    /// 冲击波/爆炸 SIMD 实测开关（RV3D_EXPLOSION_SIM=1；默认关，不影响主玩法）
    explosion_sim: bool,
    /// 冲击波压力场采样点（64×64 覆盖 512m 场地，惰性初始化）
    shock_points: Vec<[f32; 3]>,
    /// 指令集加速比基准采样点（256×256=65536，覆盖 512m 场地，惰性初始化）
    bench_points: Vec<[f32; 3]>,
    /// 冲击波压力输出（每帧覆盖）
    shock_out: Vec<f32>,
    /// 本帧冲击波压力场耗时（µs，simd: 日志用）
    stage_explosion_us: u64,
    /// 上次 simd: 加速比日志时间
    last_explosion_log: f32,
    /// 当前选路路径名（simd: 日志用）
    explosion_path: &'static str,
    /// 上次对玩家造成伤害的时间（攻击态 NPC 每秒扣血）
    last_damage_time: f32,
    /// HUD 状态（每帧喂 fps/血量，渲染前取 quad 列表）
    pub hud: HudState,
    /// fps 统计：时间窗内帧数
    frames: u64,
    /// 全局帧号（永不回绕清零；远组降频确定性分帧用）
    frame_no: u32,
    /// fps 统计时间窗起点
    fps_window_start: Instant,
    /// 音频播放器（SilentSink：rodio 未安装，样本被丢弃，但混音/衰减链路真实运行）
    audio: AudioPlayer<SilentSink>,
    /// 音频采样率
    audio_sample_rate: u32,
    /// 网络环回演示（默认关闭，RV3D_NET=1 启用）
    net_demo: Option<NetworkDemo>,
    /// 服务器模式（RV3D_NET=server）：权威模拟 + 每 tick 广播快照
    net_server: Option<Server>,
    /// 客户端模式（RV3D_NET=client）：每 tick 上报输入 + 快照插值缓冲
    net_client: Option<Client>,
    /// 客户端输入序号（每 tick +1，服务端据此去重/排序）
    net_input_seq: u32,
    /// 服务端快照序号（每 tick +1）
    net_snap_seq: u32,
    /// 客户端模式：main.rs 转发的本帧开火意图（随 Input 上报）
    net_fire_pending: bool,
    /// 服务器模式：最近一次客户端输入视角（yaw, pitch），main.rs 应用到相机
    net_look: Option<(f32, f32)>,
    /// 网络状态日志限频（1 秒一条）
    last_net_log: f32,
    /// 游戏主状态（commit a 先提供枚举与查询；e 接入完整状态机）
    game_state: GameState,
    /// 当前波次（Game::new 预置的 8 个 NPC 即第 1 波）
    wave: u32,
    /// 当前关卡（1 起；每关 WAVES_PER_LEVEL 波，清完升关并重新生成地图）
    level: u32,
    /// 当前关卡的程序化布局（种子 = level，供物理刚体 / AI 网格 / 渲染 marker 使用）
    map: LevelMap,
    /// 波间倒计时（清空后 3 秒刷下一波）
    wave_timer: f32,
    /// 击杀累计得分
    score: u64,
    /// 下一个 NPC 全局 id（出生用，保证巡逻相位唯一）
    next_npc_id: u32,
    /// 当前波开始时间（秒；援军波计时基准，spawn_wave 时重置）
    wave_started_at: f32,
    /// 本波援军是否已触发（每波重置，防止重复补怪）
    reinforcement_done: bool,
    /// 上次状态日志时间（1 秒一条 game: wave=...）
    last_status_log: f32,
    /// 任务目标（本关/本轮歼灭数；达成 → 胜利横幅/日志）
    objective: MissionObjective,
    /// 关卡系统地图管理器（RV3D_MAP/RV3D_MAPS 环境变量启用；None = 程序化地图，默认行为）
    map_mgr: Option<crate::engine::map::MapManager>,
    /// 当前地图文件路径（F5 热重载用；关卡系统启用时 Some）
    map_path: Option<String>,
    /// 关卡列表（RV3D_MAPS=index.toml 时加载；N 键按序进入下一关）
    level_list: Vec<String>,
    /// 当前关卡在 level_list 中的索引（0 起）
    level_idx: usize,
    /// 目标系统（占领据点/胜负规则；关卡系统启用时 Some）
    obj_state: Option<crate::engine::objective::ObjectiveState>,
}

/// 单帧 AI 步进上下文（全部为共享只读数据，供串行/并行两种 runner 复用）
struct AiStepCtx<'a> {
    player: &'a glam::Vec3,
    player_yaw: f32,
    charge: bool,
    under_fire: &'a [bool],
    /// 压力模式预选目标：(索引, 位置快照, 目标朝向 facing)；None = 目标为玩家
    targets: &'a [Option<(usize, [f32; 3], f32)>],
    grid: &'a GridMap,
    time: f32,
    dt: f32,
    stress: bool,
    /// 全局帧号（远组降频按 id 分帧用）
    frame: u32,
    /// 远组降频开关（压力模式开启；普通模式关闭保持行为不变）
    decimate_far: bool,
    /// 当前关卡障碍环带（theme.ring_inner/ring_outer，掩体利用评估用）
    ring_inner: f32,
    ring_outer: f32,
    /// 当前关卡存活障碍列表（掩体利用评估用；摧毁后的障碍已移除）
    obstacles: &'a [MapObstacle],
}

/// 压力模式目标预选：每 NPC 找视野内最近的敌对阵营 NPC（纯读，O(n²)）。
/// 返回 (目标索引, 目标位置快照, 目标朝向 facing)；None = 目标为玩家（兜底）。同距取索引小者，确定性。
/// facing 用于「目标是否面朝本 NPC」判定（总指挥指令单 #1 阶段二：让 NPC-vs-NPC 触发包抄/偷袭）。
fn pick_stress_targets(npcs: &[Npc], sight: f32) -> Vec<Option<(usize, [f32; 3], f32)>> {
    let mut out = Vec::with_capacity(npcs.len());
    for npc in npcs {
        let mut best: Option<(usize, f32)> = None;
        for (j, other) in npcs.iter().enumerate() {
            if other.team == npc.team || other.hp <= 0.0 {
                continue;
            }
            let dx = other.position[0] - npc.position[0];
            let dz = other.position[2] - npc.position[2];
            let d2 = dx * dx + dz * dz;
            if d2 >= sight * sight {
                continue;
            }
            if best.map_or(true, |(_, bd)| d2 < bd) {
                best = Some((j, d2));
            }
        }
        out.push(best.map(|(j, _)| (j, npcs[j].position, npcs[j].facing)));
    }
    out
}

impl Game {
    /// 创建游戏中枢：初始化物理演示场景
    pub fn new() -> Self {
        let mut world = physics::World::new();
        world.gravity = 9.8;
        // 地形中央 60×60 区域已压平到 y=0（renderer.rs flatten_mask）；障碍刚体由关卡布局生成
        world.ground_y = 0.0;
        let event_buf = Arc::new(Mutex::new(Vec::new()));
        world.add_listener(Box::new(EventBuffer(event_buf.clone())));
        // NPC 出生点：集中在中心 ±40m（相机默认在原点附近，可触发 Chase）
        let spawns = [
            (-30.0, -20.0),
            (-20.0, 15.0),
            (-10.0, -35.0),
            (0.0, 25.0),
            (15.0, -15.0),
            (25.0, 20.0),
            (35.0, -30.0),
            (40.0, 10.0),
        ];
        let mut npcs = Vec::with_capacity(NPC_COUNT);
        for (id, (x, z)) in spawns.iter().enumerate().take(NPC_COUNT) {
            npcs.push(Npc {
                id,
                position: [*x, terrain_height_at(*x, *z), *z],
                speed: 4.0,
                attack_range: 12.0,
                home: [*x, *z],
                state_machine: NpcStateMachine::new(),
                perception: NpcPerception::default(),
                path: Vec::new(),
                path_index: 0,
                hp: 100.0,
                max_hp: 100.0,
                role: TacticalRole::Rusher,
                tactic: Tactic::Advance,
                dodge_timer: 0.0,
                hit_cooldown: 0.0,
                last_hp: 100.0,
                team: Team::Red,
                facing: 0.0,
                fire_accum: 0.0,
                knockback: [0.0, 0.0],
                grenade_timer: 0.0,
            });
        }
        let mut game = Self {
            world,
            collisions: Vec::new(),
            total_collisions: 0,
            time: 0.0,
            last_dt: 0.0,
            event_buf,
            last_event_log_time: 0.0,
            wave_started_at: 0.0,
            reinforcement_done: false,
            weapons: WeaponRack::new(
                vec![
                    (
                        "M1 Rifle".to_string(),
                        Firearm::new(
                            ProjectileWeapon::new("M1 Rifle", 25.0, 3.0, 200.0, 60.0, 5.0),
                            30,
                            120,
                            2.0,
                            0.006,
                            0.003,
                        ),
                    ),
                    (
                        "Thompson SMG".to_string(),
                        crate::engine::weapons::thompson_smg_firearm(),
                    ),
                ],
                0.6,
            ),
            grenades: 2,
            grenades_max: 2,
            grenades_vec: Vec::new(),
            pending_kick: (0.0, 0.0),
            player_body: PlayerBody::new(Pv::new(0.0, 0.0, 0.0), 0.5, 1.6),
            move_forward: false,
            move_backward: false,
            move_left: false,
            move_right: false,
            footstep_timer: 0.0,
            sfx: SfxBank::new(48_000),
            projectiles: Vec::new(),
            explosions: Vec::new(),
            shake_timer: 0.0,
            shake_strength: 0.0,
            fire_cooldown: 0.0,
            shots: 0,
            hits: 0,
            grid: GridMap::new(GRID_SIZE, GRID_SIZE),
            npcs,
            ai_log_time: 0.0,
            charge_active: false,
            npc_scale: {
                let v = std::env::var("RV3D_NPC_SCALE")
                    .ok()
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                v.max(0.5)
            },
            stress: std::env::var("RV3D_STRESS_AI").is_ok(),
            stress_sides: std::env::var("RV3D_STRESS_AI")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|&n| n >= 4)
                .unwrap_or(64),
            stress_round: 0,
            ai_parallel: std::env::var("RV3D_AI_PARALLEL")
                .map(|v| v != "off")
                .unwrap_or(true),
            stage_physics_us: 0,
            stage_ai_us: 0,
            stage_audio_us: 0,
            stage_net_us: 0,
            explosion_sim: std::env::var("RV3D_EXPLOSION_SIM")
                .is_ok_and(|v| v == "1" || v == "true"),
            shock_points: Vec::new(),
            bench_points: Vec::new(),
            shock_out: Vec::new(),
            stage_explosion_us: 0,
            last_explosion_log: 0.0,
            explosion_path: "scalar",
            last_damage_time: 0.0,
            hud: HudState::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
            frames: 0,
            frame_no: 0,
            fps_window_start: Instant::now(),
            audio_sample_rate: 48_000,
            level: 1,
            map: LevelMap::default(),
            audio: {
                let mut player = AudioPlayer::new(SilentSink::new(48_000, 2));
                player.mixer_mut().set_master(0.8);
                player.mixer_mut().set_channel_volume(Channel::Sfx, 1.0);
                // 环境风：DSP 慢速调制噪声（SilentSink：输出被丢弃，但合成/混音链路真实运行）
                player
                    .synth_mut()
                    .set_ambient(glam::Vec3::new(0.0, 2.0, 0.0), 0.35);
                player
            },
            net_demo: {
                let enabled = std::env::var("RV3D_NET")
                    .map(|v| v == "1" || v == "demo")
                    .unwrap_or(false);
                if !enabled {
                    None
                } else {
                    match Self::init_network_demo() {
                        Ok(demo) => {
                            log::info!("net: loopback demo enabled (in-process server+client)");
                            Some(demo)
                        }
                        Err(e) => {
                            log::warn!("net: 环回演示初始化失败，已禁用: {}", e);
                            None
                        }
                    }
                }
            },
            net_server: None,
            net_client: None,
            net_input_seq: 0,
            net_snap_seq: 0,
            net_fire_pending: false,
            net_look: None,
            last_net_log: 0.0,
            game_state: GameState::StartMenu,
            wave: 1,
            wave_timer: 0.0,
            score: 0,
            next_npc_id: NPC_COUNT as u32,
            last_status_log: 0.0,
            objective: MissionObjective::new(0),
            map_mgr: None,
            map_path: None,
            level_list: Vec::new(),
            level_idx: 0,
            obj_state: None,
        };
        // 关卡系统初始化：RV3D_MAP=<单关 toml> 或 RV3D_MAPS=<index.toml 关卡列表>
        // 启用时替换程序化地图；未设置 → None（程序化地图，默认行为零回归）
        game.init_map_system();
        // 初始关卡布局（level 1，种子 = 1）：物理刚体 + AI 网格 + 玩家安全区复位
        game.apply_level(1);
        game
    }

    /// 当前游戏状态（供 main.rs 控制光标捕获等输入行为）
    pub fn state(&self) -> GameState {
        self.game_state
    }

    /// 初始化关卡系统（Game::new 调用一次）：
    /// - `RV3D_MAP=<assets/maps/xxx.toml>`：加载单张地图（关卡列表为空）
    /// - `RV3D_MAPS=<assets/maps/index.toml>`：加载关卡列表，进入第一关
    /// 两者皆未设置 → 保持 None（程序化地图 + StartMenu，默认行为与测试基线零回归）。
    /// 加载失败仅告警并回退程序化地图（不 panic、不中断启动）。
    fn init_map_system(&mut self) {
        let single = std::env::var("RV3D_MAP").ok().filter(|p| !p.is_empty());
        let index = std::env::var("RV3D_MAPS").ok().filter(|p| !p.is_empty());
        let (path, list) = if let Some(p) = single {
            (p, Vec::new())
        } else if let Some(idx) = index {
            match crate::engine::map::load_map_list(&idx) {
                Ok(list) if !list.is_empty() => (list[0].clone(), list),
                Ok(_) => {
                    log::warn!("map: 关卡列表 {} 为空，回退程序化地图", idx);
                    return;
                }
                Err(e) => {
                    log::warn!("map: 关卡列表加载失败 {}: {}，回退程序化地图", idx, e);
                    return;
                }
            }
        } else {
            return;
        };
        match crate::engine::map::MapManager::load(&path) {
            Ok(mgr) => {
                use crate::engine::objective::GameRule;
                let rule = match mgr.data().rule.kind.as_str() {
                    "kill" => GameRule::KillCount {
                        target: mgr.data().rule.target,
                    },
                    "time" => GameRule::TimeLimit {
                        seconds: mgr.data().rule.seconds,
                    },
                    "survive" => GameRule::Survive {
                        waves: mgr.data().rule.waves.max(1),
                    },
                    _ => GameRule::CapturePoints {
                        required: mgr.data().rule.required.max(1),
                    },
                };
                let mut pts = Vec::new();
                for o in mgr
                    .data()
                    .objectives
                    .iter()
                    .filter(|o| o.kind.eq_ignore_ascii_case("capture"))
                {
                    pts.push(crate::engine::objective::CapturePoint::new(
                        o.id.clone(),
                        o.x,
                        o.z,
                        o.radius,
                        o.capture_time,
                    ));
                }
                let mut obj_state = crate::engine::objective::ObjectiveState::new(rule);
                obj_state.points = pts;
                log::info!(
                    "map: 关卡系统启用 {}（{} 出生点 / {} 障碍 / {} 目标，规则 {}）",
                    mgr.data().name,
                    mgr.data().spawn_points.len(),
                    mgr.data().obstacles.len(),
                    mgr.data().objectives.len(),
                    obj_state.rule.rule_kind()
                );
                self.map_mgr = Some(mgr);
                self.map_path = Some(path);
                self.level_list = list;
                self.level_idx = 0;
                self.obj_state = Some(obj_state);
                // 加载完成 → LoadingMap 态（按任意键开玩）
                self.game_state = GameState::LoadingMap;
            }
            Err(e) => {
                log::warn!("map: 关卡加载失败 {}: {}，回退程序化地图", path, e);
            }
        }
    }

    /// 开始菜单任意键：进入游戏（开始/重开一局）
    pub fn on_any_key(&mut self, player: &glam::Vec3) {
        if self.game_state == GameState::StartMenu || self.game_state == GameState::LoadingMap {
            self.start_run(player);
        }
    }

    /// 死亡/失败结算界面 R：重开一局（重开本关）
    pub fn request_restart(&mut self, player: &glam::Vec3) {
        match self.game_state {
            GameState::GameOver | GameState::Victory(_) | GameState::Defeat => {
                self.start_run(player);
            }
            _ => {}
        }
    }

    /// 开始一局：复位血量/弹药/分数/波次/关卡，重建第 1 关地图，清掉残留 NPC 后生成第 1 波
    fn start_run(&mut self, player: &glam::Vec3) {
        self.hud.health = self.hud.max_health;
        self.hud.ammo = self.hud.max_ammo;
        self.hud.reserve = self.weapons.active_firearm_ref().reserve();
        self.hud.settings_open = false;
        self.hud.confirm_quit = false;
        self.hud.victory_banner = None;
        self.hud.cancel_rebind();
        self.score = 0;
        self.wave = 1;
        self.wave_timer = 0.0;
        self.fire_cooldown = 0.0;
        self.weapons.active_firearm().reset();
        self.pending_kick = (0.0, 0.0);
        // 重开一局 = 从第 1 关全新地图开始（同时把玩家拉回原点安全区）
        self.apply_level(1);
        // 关卡系统：玩家出生点用地图 Blue 出生点（未启用时保持原点）
        if let Some(mgr) = self.map_mgr.as_ref() {
            if let Some((sx, sy, sz)) = mgr.spawn_point("blue") {
                self.player_body.pos = Pv::new(sx, sy, sz);
            } else {
                self.player_body.pos = Pv::new(0.0, 0.0, 0.0);
            }
        } else {
            self.player_body.pos = Pv::new(0.0, 0.0, 0.0);
        }
        self.player_body.vel = Pv::ZERO;
        self.move_forward = false;
        self.move_backward = false;
        self.move_left = false;
        self.move_right = false;
        self.footstep_timer = 0.0;
        self.projectiles.clear();
        self.shots = 0;
        self.hits = 0;
        self.total_collisions = 0;
        self.last_damage_time = 0.0;
        self.stress_round = 0;
        if !self.npcs.is_empty() {
            log::info!("game: purged {} leftover npcs on run start", self.npcs.len());
            self.npcs.clear();
        }
        if self.stress {
            self.spawn_stress_battle(player);
        } else {
            self.spawn_wave(1, player);
        }
        // 关卡系统：重开本关 → 据点进度/击杀/计时归零（rule 保持当前地图的规则）
        if let Some(obj) = self.obj_state.as_mut() {
            for pt in obj.points.iter_mut() {
                pt.progress = 0.0;
                pt.owner = None;
            }
            obj.kills = 0;
            obj.elapsed = 0.0;
            obj.won_team = None;
        }
        self.game_state = GameState::Playing;
        log::info!("game: run started (wave 1)");
    }

    /// 累计有效波次：跨关不回落（level 2 第 1 波 ≈ 原第 4 波强度）
    fn effective_wave(&self, wave: u32) -> u32 {
        wave + (self.level.saturating_sub(1)) * WAVES_PER_LEVEL
    }

    /// 应用关卡布局：重建物理障碍刚体 + AI 导航网格（确定性，种子 = 关卡号）。
    ///
    /// 由 new() / start_run() / 升关时调用；同时把玩家拉回原点安全区，防止卡进障碍。
    fn apply_level(&mut self, level: u32) {
        self.level = level;
        // 地图来源：关卡系统启用（RV3D_MAP/RV3D_MAPS）→ TOML 障碍；否则程序化生成（默认行为）。
        // 关卡系统地图已在 init_map_system / advance_level / reload_current_map 加载到 map_mgr。
        if let Some(mgr) = self.map_mgr.as_ref() {
            let mut obstacles = Vec::new();
            for def in mgr.obstacles() {
                let (kind, x, z, half_w, half_d) =
                    crate::engine::map::obstacle_to_map_obstacle(def);
                let max_hp = obstacle_max_hp(kind);
                obstacles.push(MapObstacle {
                    x,
                    z,
                    half_w,
                    half_d,
                    kind,
                    max_hp,
                    hp: max_hp,
                });
            }
            self.map = LevelMap { obstacles };
        } else {
            // 地图生成换核执行（线程优化第 3 步）：走 ai_pool（AMD CCD1 / Intel E-core），
            // 生成计算不吃主线程所在簇；join 语义保证返回时地图已就绪。
            self.map = crate::engine::cpu::ai_pool().run_sync(move || generate_level_map(level));
        }
        // 任务目标：普通波次 = 本关 WAVES_PER_LEVEL 波出场总数（含援军）；
        // 压力模式 = 歼灭一队即本轮胜利（spawn_stress_battle 每轮重置）
        self.objective = MissionObjective::new(if self.stress {
            self.stress_sides as u32
        } else {
            self.level_objective_target(level)
        });
        self.hud.victory_banner = None;
        // 物理世界：清掉上一关障碍，重建为当前关卡障碍盒（贴地 AABB，供玩家碰撞/投射物拦截）
        self.world.bodies.clear();
        self.world.spheres.clear();
        for ob in &self.map.obstacles {
            self.world.bodies.push(Body::new(
                Pv::new(ob.x, MAP_BLOCK_HEIGHT * 0.5, ob.z),
                Pv::new(ob.half_w, MAP_BLOCK_HEIGHT * 0.5, ob.half_d),
            ));
        }
        // AI 网格：障碍盒覆盖的格全部标记阻挡（NPC 寻路绕行 / 掩体点判定共用同一网格）
        let mut grid = GridMap::new(GRID_SIZE, GRID_SIZE);
        let mut blocked_cells = 0usize;
        for ob in &self.map.obstacles {
            let g0 = world_to_grid(ob.x - ob.half_w, ob.z - ob.half_d);
            let g1 = world_to_grid(ob.x + ob.half_w, ob.z + ob.half_d);
            for gx in g0.x..=g1.x {
                for gz in g0.y..=g1.y {
                    let pos = GridPos::new(gx, gz);
                    if grid.in_bounds(pos) {
                        grid.block(pos);
                        blocked_cells += 1;
                    }
                }
            }
        }
        self.grid = grid;
        // 升关/重开时把玩家拉回原点安全区（中央环带无阻碍，见 MAP_RING_INNER 注释）
        self.player_body.pos = Pv::new(0.0, 0.0, 0.0);
        self.player_body.vel = Pv::ZERO;
        // 障碍种类分布统计（供日志/调试；渲染 marker 颜色由 main.rs 按 kind 映射）
        let mut kind_counts = [0u32; 3];
        for ob in &self.map.obstacles {
            kind_counts[match ob.kind {
                ObstacleKind::Wall => 0,
                ObstacleKind::Block => 1,
                ObstacleKind::Barrier => 2,
            }] += 1;
        }
        log::info!(
            "map: level {} generated: {} obstacle bodies (wall={} block={} barrier={}), {} grid cells blocked",
            level,
            self.map.obstacles.len(),
            kind_counts[0],
            kind_counts[1],
            kind_counts[2],
            blocked_cells
        );
    }

    /// 本关任务目标：WAVES_PER_LEVEL 波出场敌人总数（含援军；count 按
    /// RV3D_NPC_SCALE 缩放后四舍五入，与 spawn_wave 的出生数量逐波一致）
    fn level_objective_target(&self, level: u32) -> u32 {
        let mut total = 0u32;
        for w in 1..=WAVES_PER_LEVEL {
            let effective = w + (level.saturating_sub(1)) * WAVES_PER_LEVEL;
            let profile = wave_profile(effective);
            let count = (profile.count as f32 * self.npc_scale).round().max(1.0) as u32;
            total += count + profile.reinforcement_count;
        }
        total.max(1)
    }

    /// 任务目标达成：一次性胜利日志 + HUD 横幅（游戏继续，不阻断波次生成/补员逻辑）
    fn on_objective_complete(&mut self) {
        self.hud.victory_banner = Some(if self.stress {
            "VICTORY — 本轮敌军全灭".to_string()
        } else {
            "VICTORY — 本关敌军全灭".to_string()
        });
        log::info!(
            "objective: {} 歼灭全部敌人达成（{} 击杀）→ victory",
            if self.stress {
                "压力模式本轮"
            } else {
                "普通模式本关"
            },
            self.objective.eliminated
        );
    }

    /// 当前关卡障碍列表（main.rs 每帧转成渲染 marker 用）
    pub fn map_obstacles(&self) -> &[MapObstacle] {
        &self.map.obstacles
    }

    /// 设置面板：进入"等待按键绑定"（Enter 触发，绑定当前选中的键位动作）
    pub fn begin_rebind(&mut self) {
        if let Some(action) = self.hud.selected_action() {
            self.hud.begin_rebind(action);
        }
    }

    /// 设置面板：完成绑定（非 ESC 按键触发），绑定后持久化配置
    pub fn complete_rebind(&mut self, code: u32) {
        if self.hud.complete_rebind(code).is_some() {
            crate::config::save(&self.current_config());
        }
    }

    /// 设置面板：取消绑定（ESC 触发）
    pub fn cancel_rebind(&mut self) {
        self.hud.cancel_rebind();
    }

    /// 设置面板是否正在等待按键绑定（main.rs 据此拦截按键）
    pub fn rebinding_active(&self) -> bool {
        self.hud.rebinding_action().is_some()
    }

    /// 当前可持久化配置（键位 + 音量 + 灵敏度）
    fn current_config(&self) -> crate::config::GameConfig {
        crate::config::GameConfig {
            volume: self.hud.volume,
            music_volume: self.hud.music_volume,
            sensitivity: self.hud.sensitivity,
            bindings: self.hud.key_bindings,
            resolution: self.hud.resolution(),
            resolution_explicit: true, // 保存时总是写 resolution 行，加载后视为显式选择
            quality: self.hud.quality_index as u32,
        }
    }

    /// 初始化网络环回演示：绑定环回 Server + 连入 Client，发起 Join
    fn init_network_demo() -> Result<NetworkDemo, String> {
        let server = Server::bind("127.0.0.1:0").map_err(|e| format!("bind 失败: {}", e))?;
        let addr = server.local_addr().map_err(|e| format!("local_addr 失败: {}", e))?;
        let client = Client::connect(addr).map_err(|e| format!("connect 失败: {}", e))?;
        client
            .send(&NetworkMessage::Join {
                player_id: 0,
                name: "local".into(),
            })
            .map_err(|e| format!("发送 Join 失败: {}", e))?;
        Ok(NetworkDemo {
            server,
            client,
            seq: 0,
            last_log: 0.0,
        })
    }

    /// 每帧推进所有已接入系统
    pub fn update(&mut self, dt: f32, camera: &Camera) {
        self.frame_no = self.frame_no.wrapping_add(1);
        self.last_dt = dt;
        self.time += dt;
        self.fire_cooldown = (self.fire_cooldown - dt).max(0.0);
        // 武器架切换计时/换弹计时 + HUD 武器/弹药/换弹状态同步
        self.weapons.update(dt);
        self.hud.ammo = self.weapons.active_firearm_ref().magazine();
        self.hud.max_ammo = self.weapons.active_firearm_ref().max_magazine();
        self.hud.reserve = self.weapons.active_firearm_ref().reserve();
        self.hud.reloading = self.weapons.active_firearm_ref().is_reloading();
        self.hud.reload_progress = self.weapons.active_firearm_ref().reload_progress();
        self.hud.weapon_name = self.weapons.active_name().to_string();
        self.hud.switching = self.weapons.is_switching();
        self.hud.grenades = self.grenades;
        // 手榴弹推进（抛物线 + 引信）
        self.update_grenades(dt);
        // 关卡号同步（由关卡推进 / 重开写入，供 HUD 显示）
        self.hud.level = self.level;
        // 命中标记衰减 + 音量同步
        self.hud.tick(dt);
        self.audio.mixer_mut().set_master(self.hud.volume);
        // 音乐通道独立音量（设置面板 MUSIC 项，0..=1）
        self.audio
            .mixer_mut()
            .set_channel_volume(Channel::Music, self.hud.music_volume);
        // 程序化环境音乐：战斗状态开大、菜单/结算调小（1.5s 淡入淡出由 audio 内部插值）
        let music_target = match self.game_state {
            GameState::Playing => 1.0,
            _ => 0.3,
        };
        self.audio.set_music_target(music_target);
        // 第一人称玩家移动（WASD + 碰撞）
        if self.game_state == GameState::Playing && camera.mode == CameraMode::FirstPerson {
            self.move_first_person(camera, dt);
        }
        // fps 统计（1 秒窗口）
        self.frames += 1;
        let window_secs = self.fps_window_start.elapsed().as_secs_f32();
        if window_secs >= 1.0 {
            self.hud.fps = self.frames as f32 / window_secs;
            self.frames = 0;
            self.fps_window_start = Instant::now();
        }
        let t0 = std::time::Instant::now();
        self.world.step(dt);
        self.drain_collisions();
        self.stage_physics_us = t0.elapsed().as_micros() as u64;
        let t0 = std::time::Instant::now();
        match self.game_state {
            GameState::StartMenu => {
                // 菜单吸引模式：世界照常运行（NPC 游走/追击），不结算伤害与波次
                self.update_projectiles(dt, true);
                self.update_ai(dt, camera);
            }
            GameState::LoadingMap => {
                // 关卡加载为同步操作（init_map_system 已载入），此态仅作状态机过渡
            }
            GameState::Playing => {
                self.update_projectiles(dt, true);
                self.update_ai(dt, camera);
                self.update_waves(dt, &camera.position());
                // 关卡系统：每帧推进据点占领 + 胜负判定（未启用时无操作）
                self.update_objectives(dt, &camera.position());
            }
            GameState::GameOver
            | GameState::Victory(_)
            | GameState::Defeat => {
                // 冻结玩法：AI/伤害/波次停止；投射物继续飞行但不再判定命中/击杀
                self.update_projectiles(dt, false);
            }
        }
        self.stage_ai_us = t0.elapsed().as_micros() as u64;
        // 爆炸实体生命周期 + 震屏衰减（生成在 update_projectiles 内，AoE 已即时结算）
        self.step_explosions(dt);
        // 冲击波/爆炸 SIMD 实测（默认关；RV3D_EXPLOSION_SIM=1 时每帧推进压力场并输出加速比）
        if self.explosion_sim {
            self.step_explosion_sim();
        }
        // 状态日志（1 秒一条，冒烟断言 game: wave= 序列用）
        if self.time - self.last_status_log >= 1.0 {
            self.last_status_log = self.time;
            let enemy_hp = self.npcs.first().map(|n| n.max_hp).unwrap_or(0.0);
            log::info!(
                "game: wave={} enemies={} enemy_hp={:.0} hp={:.0}/{:.0} score={} phys_us={} ai_us={} audio_us={} net_us={}",
                self.wave,
                self.npcs.len(),
                enemy_hp,
                self.hud.health,
                self.hud.max_health,
                self.score,
                self.stage_physics_us,
                self.stage_ai_us,
                self.stage_audio_us,
                self.stage_net_us
            );
        }
        // 音频：每帧按 dt 渲染样本（SilentSink 丢弃输出，混音/衰减链路真实运行）
        let t0 = std::time::Instant::now();
        let frames = ((self.audio_sample_rate as f32) * dt) as usize;
        self.audio
            .tick(&AudioListener::new(camera.position()), frames.min(8192));
        self.stage_audio_us = t0.elapsed().as_micros() as u64;
        let t0 = std::time::Instant::now();
        self.update_net(camera);
        self.step_net(camera);
        self.stage_net_us = t0.elapsed().as_micros() as u64;
    }

    /// 关卡系统每帧推进（仅 Playing 态调用；未启用时直接返回）：
    /// 1. 据点占领：玩家（Blue 阵营）站在据点内且无 Red NPC 在场 → 进度增长；
    ///    有敌对 NPC 在场 → 压制衰减；玩家撤离 → 缓慢消散。
    /// 2. 胜负判定：规则达成 → 切换 GameState::Victory(team) / Defeat（幂等，只触发一次）。
    /// 3. 限时统计同步给 ObjectiveState。
    fn update_objectives(&mut self, dt: f32, player: &glam::Vec3) {
        let Some(obj) = self.obj_state.as_mut() else { return };
        let player_team = crate::engine::ai::Team::Blue; // 玩家恒为 Blue 阵营
        for pt in obj.points.iter_mut() {
            let inside = pt.is_inside(player.x, player.z);
            let has_enemy = self
                .npcs
                .iter()
                .any(|n| n.team != player_team && pt.is_inside(n.position[0], n.position[2]));
            let players_inside: Vec<crate::engine::ai::Team> =
                if inside { vec![player_team] } else { Vec::new() };
            crate::engine::objective::update_point(pt, dt, &players_inside, has_enemy);
        }
        obj.elapsed += dt as f64;
        match obj.evaluate() {
            crate::engine::objective::WinState::Victory(team) => {
                obj.won_team = Some(team);
                self.game_state = GameState::Victory(team);
                log::info!("objective: 关卡胜利，获胜方 {:?}", team);
            }
            crate::engine::objective::WinState::Defeat => {
                obj.won_team = Some(player_team.opposite());
                self.game_state = GameState::Defeat;
                log::info!("objective: 关卡失败（时间到/据点尽失）");
            }
            crate::engine::objective::WinState::None => {}
        }
    }

    /// 关卡系统击杀计数：普通波次击杀（damage_npc）调用，KillCount 规则用
    fn objective_register_kill(&mut self) {
        if let Some(obj) = self.obj_state.as_mut() {
            obj.kills = obj.kills.saturating_add(1);
        }
    }

    /// 是否启用 survive（防守波次）规则
    fn is_survive_rule(&self) -> bool {
        matches!(
            self.obj_state.as_ref().map(|o| o.rule),
            Some(crate::engine::objective::GameRule::Survive { .. })
        )
    }

    /// survive 总波数（默认 0 = 非 survive）
    fn survive_total_waves(&self) -> u32 {
        match self.obj_state.as_ref().map(|o| o.rule) {
            Some(crate::engine::objective::GameRule::Survive { waves }) => waves,
            _ => 0,
        }
    }

    /// survive 波间补给窗口：血量回复 50% + 当前武器弹匣补满 + 手榴弹补满
    fn supply_survive_break(&mut self) {
        self.hud.health = (self.hud.health + self.hud.max_health * 0.5).min(self.hud.max_health);
        self.weapons.active_firearm().reset();
        self.grenades = self.grenades_max;
        log::info!(
            "survive: 波间补给（血量 {:.0}% + 弹药补满 + 手榴弹 {}）",
            self.hud.health / self.hud.max_health * 100.0,
            self.grenades
        );
    }

    /// 置位关卡胜负归属（幂等：已判定则不覆盖）
    fn set_won_team(&mut self, team: crate::engine::ai::Team) {
        if let Some(obj) = self.obj_state.as_mut() {
            if obj.won_team.is_none() {
                obj.won_team = Some(team);
            }
        }
    }

    /// 关卡系统据点数据（供 main.rs 渲染世界标记）：(id, x, z, 归属, 进度 0..=1)。
    /// 未启用关卡系统或无据点 → 空列表。
    pub fn capture_points(&self) -> Vec<(String, f32, f32, Option<crate::engine::ai::Team>, f32)> {
        self.obj_state
            .as_ref()
            .map(|o| {
                o.points
                    .iter()
                    .map(|p| (p.id.clone(), p.x, p.z, p.owner, p.progress))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 关卡系统热重载（F5）：重新读取当前地图 TOML 并重建物理/AI 网格/据点
    pub fn reload_current_map(&mut self) -> Result<(), String> {
        let Some(path) = self.map_path.clone() else {
            return Ok(()); // 未启用关卡系统：无事可做
        };
        {
            let Some(mgr) = self.map_mgr.as_mut() else {
                return Ok(());
            };
            mgr.reload(&path)?;
        }
        // 重建物理/AI 网格（复用 apply_level 的障碍→刚体/网格逻辑）
        self.apply_level(self.level);
        // 重建据点（进度归零，规则沿用新地图的 rule）
        let rule = {
            let d = self.map_mgr.as_ref().map(|m| &m.data().rule);
            match d.map(|r| r.kind.as_str()) {
                Some("kill") => crate::engine::objective::GameRule::KillCount {
                    target: d.map(|r| r.target).unwrap_or(0),
                },
                Some("time") => crate::engine::objective::GameRule::TimeLimit {
                    seconds: d.map(|r| r.seconds).unwrap_or(0.0),
                },
                Some("survive") => crate::engine::objective::GameRule::Survive {
                    waves: d.map(|r| r.waves.max(1)).unwrap_or(1),
                },
                _ => crate::engine::objective::GameRule::CapturePoints {
                    required: d.map(|r| r.required.max(1)).unwrap_or(1),
                },
            }
        };
        let mut obj = crate::engine::objective::ObjectiveState::new(rule);
        obj.points = self
            .map_mgr
            .as_ref()
            .map(|m| {
                m.data()
                    .objectives
                    .iter()
                    .filter(|o| o.kind.eq_ignore_ascii_case("capture"))
                    .map(|o| {
                        crate::engine::objective::CapturePoint::new(
                            o.id.clone(),
                            o.x,
                            o.z,
                            o.radius,
                            o.capture_time,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.obj_state = Some(obj);
        log::info!("map: 热重载完成（{}）", path);
        Ok(())
    }

    /// 关卡系统下一关（胜利结算 N 键）：进入列表下一张地图；已到最后一关 → 返回 false（通关）
    pub fn advance_level(&mut self, player: &glam::Vec3) -> bool {
        if self.level_list.is_empty() {
            return false; // 单关模式（RV3D_MAP）：无下一关
        }
        if self.level_idx + 1 >= self.level_list.len() {
            return false; // 最后一关通关
        }
        self.level_idx += 1;
        let path = self.level_list[self.level_idx].clone();
        match crate::engine::map::MapManager::load(&path) {
            Ok(mgr) => {
                self.map_mgr = Some(mgr);
                self.map_path = Some(path);
                self.apply_level(self.level);
                self.start_run(player);
                log::info!(
                    "map: 进入下一关 {}（第 {} 关）",
                    self.map_mgr
                        .as_ref()
                        .map(|m| m.data().name.clone())
                        .unwrap_or_default(),
                    self.level_idx + 1
                );
                true
            }
            Err(e) => {
                log::warn!("map: 下一关加载失败 {}: {}", path, e);
                false
            }
        }
    }

    /// 服务器模式（RV3D_NET=server）：绑定好的 Server 交给 Game 托管，开始权威模拟 + 快照广播
    pub fn set_net_server(&mut self, server: Server) {
        log::info!("net: server mode active");
        self.net_server = Some(server);
    }

    /// 客户端模式（RV3D_NET=client）：连上的 Client 交给 Game 托管，开始输入上报 + 快照缓冲
    pub fn set_net_client(&mut self, client: Client) {
        log::info!("net: client mode active");
        self.net_client = Some(client);
    }

    /// main.rs 转发本帧开火意图（客户端模式随 Input 上报服务端）
    pub fn set_net_fire(&mut self, fire: bool) {
        self.net_fire_pending = fire;
    }

    /// 服务器模式最近一次客户端输入视角（main.rs 应用到相机，快照权威视角）
    pub fn net_look(&self) -> Option<(f32, f32)> {
        self.net_look
    }

    /// 网络对战模式步进（RV3D_NET=server|client；默认关闭，不破坏单机模式）。
    /// 与旧环回演示（update_net）互斥：演示用 RV3D_NET=1|demo，对战用 server|client。
    fn step_net(&mut self, camera: &Camera) {
        self.step_net_server(camera);
        self.step_net_client(camera);
        // 1 秒一条状态日志（联调/冒烟断言依据）
        if self.time - self.last_net_log >= 1.0 {
            self.last_net_log = self.time;
            match (&self.net_server, &self.net_client) {
                (Some(server), _) => {
                    log::info!(
                        "net: server clients={} seq={} npcs={}",
                        server.client_count(),
                        self.net_snap_seq,
                        self.npcs.len()
                    );
                }
                (None, Some(client)) => {
                    log::info!(
                        "net: client id={:?} seq={} entities={} own={:?} connected={} obj={} rule={}",
                        client.player_id(),
                        client.snapshot_seq(),
                        client.entities().len(),
                        client.own_state(),
                        client.is_connected(),
                        client.objective_state().len(),
                        client.objective_rule()
                    );
                }
                _ => {}
            }
        }
    }

    /// 服务器模式：收客户端输入应用 + 超时清理 + 每 tick 广播快照
    fn step_net_server(&mut self, camera: &Camera) {
        let mut inputs: Vec<NetInput> = Vec::new();
        {
            let Some(server) = self.net_server.as_mut() else {
                return;
            };
            while let Ok(Some((msg, from))) = server.recv() {
                match msg {
                    NetworkMessage::Join { name, .. } => {
                        let _ = server.handle_join(from, name);
                    }
                    NetworkMessage::Input { input, .. } => inputs.push(input),
                    _ => {}
                }
            }
            // 断线基本处理：超过 SERVER_TIMEOUT 无包的客户端移除注册（重连为后续 TODO）
            let removed = server.timeout_clients(SERVER_TIMEOUT);
            if !removed.is_empty() {
                log::warn!("net: server 移除超时客户端: {:?}", removed);
            }
        }
        // 应用最新一份客户端输入（本轮单客户端为主；多客户端按到达顺序取最后一份）
        if let Some(input) = inputs.pop() {
            self.apply_net_input(input);
        }
        // 每 tick 广播快照：本机玩家 + 全部 NPC（位置/朝向/血量）
        self.net_snap_seq = self.net_snap_seq.wrapping_add(1);
        let cam_pos = camera.position();
        let player = PlayerState::new([cam_pos.x, cam_pos.y, cam_pos.z], camera.yaw);
        let npcs = self
            .npcs
            .iter()
            .map(|n| NpcSnapshot {
                id: n.id as u32,
                pos: n.position,
                facing: n.facing,
                hp: n.hp,
            })
            .collect();
        let snapshot = NetworkMessage::Snapshot {
            seq: self.net_snap_seq,
            time: self.time,
            player_id: 0,
            player,
            npcs,
        };
        if let Some(server) = self.net_server.as_ref() {
            let _ = server.broadcast(&snapshot, None);
            // 目标状态（据点归属/进度）广播：关卡系统启用时组包（归属码 0=中立/1=Red/2=Blue）。
            // 未启用关卡系统（obj_state=None）→ 空据点列表广播（客户端据此可知无目标）。
            if let Some(obj) = self.obj_state.as_ref() {
                let points = obj
                    .points
                    .iter()
                    .map(|p| {
                        let owner = match p.owner {
                            None => 0u8,
                            Some(crate::engine::ai::Team::Red) => 1,
                            Some(crate::engine::ai::Team::Blue) => 2,
                        };
                        (p.id.clone(), owner, p.progress)
                    })
                    .collect();
                let obj_msg = NetworkMessage::ObjectiveState {
                    seq: self.net_snap_seq,
                    time: self.time,
                    rule_kind: obj.rule.rule_kind().to_string(),
                    points,
                };
                let _ = server.broadcast(&obj_msg, None);
            }
        }
    }

    /// 客户端模式：握手重试 + 每 tick 上报输入/姿态 + 收快照进插值缓冲
    fn step_net_client(&mut self, camera: &Camera) {
        let Some(client) = self.net_client.as_mut() else {
            return;
        };
        // 握手：未确认时按 0.5s 重发 Join（UDP 尽力而为下唯一带重试的报文）
        client.retry_join("steel", std::time::Duration::from_millis(500));
        // 每 tick 上报本地输入/姿态
        self.net_input_seq = self.net_input_seq.wrapping_add(1);
        let input = NetInput {
            forward: self.move_forward,
            backward: self.move_backward,
            left: self.move_left,
            right: self.move_right,
            fire: self.net_fire_pending,
            yaw: camera.yaw,
            pitch: camera.pitch,
        };
        let _ = client.send(&NetworkMessage::Input {
            seq: self.net_input_seq,
            time: client.now() as f32,
            input,
        });
        // 收快照：进入实体插值表（位置平滑；渲染消费为后续 TODO，先保证数据缓冲正确）
        while let Ok(Some((msg, _))) = client.recv() {
            client.handle_message(msg);
        }
        // 断线基本处理：已加入后超过 CLIENT_TIMEOUT 无数据报 → 告警（重连为后续 TODO）
        if client.player_id().is_some() && client.snapshot_timeout() {
            log::warn!(
                "net: client 断线（{}s 无数据），等待重连...",
                CLIENT_TIMEOUT.as_secs()
            );
        }
    }

    /// 服务端应用客户端输入：移动标志 + 开火（方向 = 输入视角）+ 记录视角供 main.rs 应用
    fn apply_net_input(&mut self, input: NetInput) {
        self.move_forward = input.forward;
        self.move_backward = input.backward;
        self.move_left = input.left;
        self.move_right = input.right;
        self.net_look = Some((input.yaw, input.pitch));
        if input.fire {
            let eye = self.player_eye();
            let dir = glam::Vec3::new(
                input.pitch.cos() * input.yaw.sin(),
                input.pitch.sin(),
                input.pitch.cos() * input.yaw.cos(),
            );
            self.fire([eye.x, eye.y, eye.z], [dir.x, dir.y, dir.z]);
        }
    }

    /// 网络环回演示：server 收包回环广播，client 发包/收包做远端插值；不参与帧率逻辑
    fn update_net(&mut self, camera: &Camera) {
        let Some(demo) = &mut self.net_demo else {
            return;
        };
        // 服务器收包：Join 分配 id 回 ack；其余消息回环广播给所有客户端（含发送者）
        while let Ok(Some((msg, from))) = demo.server.recv() {
            match &msg {
                NetworkMessage::Join { name, .. } => {
                    let _ = demo.server.handle_join(from, name.clone());
                }
                _ => {
                    let _ = demo.server.broadcast(&msg, None);
                }
            }
        }
        // 客户端：每帧发送自身位置
        demo.seq = demo.seq.wrapping_add(1);
        let pos = camera.position();
        let player_id = demo.client.player_id().unwrap_or(0);
        let _ = demo.client.send(&NetworkMessage::Position {
            player_id,
            seq: demo.seq,
            state: PlayerState::new([pos.x, pos.y, pos.z], 0.0),
        });
        // 客户端收包：Join 确认 + 回环 Position（进入远端插值缓冲）
        while let Ok(Some((msg, _))) = demo.client.recv() {
            demo.client.handle_message(msg);
        }
        // 每秒一条日志：远端玩家数与插值采样
        if self.time - demo.last_log >= 1.0 {
            demo.last_log = self.time;
            let t = demo.client.now();
            let n = demo.client.remote_players().len();
            let sample = demo
                .client
                .remote_players()
                .values()
                .next()
                .map(|r| r.state_at(t));
            log::info!("net: remote_players={} sample={:?}", n, sample);
        }
    }

    /// 构建 HUD quad 列表：血条/弹药/FPS + 调试行（LOD、实体数、NPC 状态、碰撞/命中）
    pub fn hud_quads(&mut self, near: u32, far: u32, lod: &str) -> Vec<crate::ui::Quad> {
        use crate::ui::{render_text, Color, HudScreen};
        // 每帧同步 HUD 显示字段
        self.hud.score = self.score;
        self.hud.wave = self.wave;
        self.hud.countdown = self.wave_timer.max(0.0);
        self.hud.survive_waves = self.survive_total_waves();
        self.hud.objective = (self.objective.eliminated, self.objective.target);
        // 关卡系统：据点状态同步到 HUD（id/归属/进度）。
        // 单机 = 本机 obj_state；联机客户端 = 网络 ObjectiveState 广播（归属码 0=中立/1=Red/2=Blue）。
        // 无联机且无关卡系统 → 空列表（HUD 不显示进度条，行为零回归）。
        self.hud.capture_points = if let Some(o) = self.obj_state.as_ref() {
            o.points
                .iter()
                .map(|p| (p.id.clone(), p.owner, p.progress))
                .collect()
        } else if let Some(client) = self.net_client.as_ref() {
            client
                .objective_state()
                .iter()
                .map(|(id, code, progress)| {
                    let owner = match code {
                        1 => Some(crate::engine::ai::Team::Red),
                        2 => Some(crate::engine::ai::Team::Blue),
                        _ => None,
                    };
                    (id.clone(), owner, *progress)
                })
                .collect()
        } else {
            Vec::new()
        };
        self.hud.screen = if self.hud.settings_open {
            HudScreen::Settings
        } else {
            match self.game_state {
                GameState::StartMenu => HudScreen::Start,
                GameState::LoadingMap => HudScreen::Start,
                GameState::GameOver | GameState::Defeat => HudScreen::GameOver,
                GameState::Victory(_) => HudScreen::Game,
                GameState::Playing => HudScreen::Game,
            }
        };
        let mut quads = self.hud.layout();
        if self.game_state != GameState::Playing {
            return quads;
        }
        let mut counts = [0u32; 4];
        for npc in &self.npcs {
            counts[npc.state_machine.state() as usize] += 1;
        }
        let line1 = format!(
            "LOD: {}  entities: {}/65536  npc: I{} P{} C{} A{}",
            lod,
            near + far,
            counts[0],
            counts[1],
            counts[2],
            counts[3]
        );
        render_text(&line1, 10.0, 44.0, Color::YELLOW, 1.3, &mut quads);
        let line2 = format!(
            "collisions: {}  hits: {}  ammo: {:.0}%",
            self.total_collisions(),
            self.hits(),
            self.weapons.active_firearm_ref().ammo_ratio() * 100.0
        );
        render_text(&line2, 10.0, 62.0, Color::CYAN, 1.3, &mut quads);
        quads
    }

    /// 构建默认光照场景（方向光 + 环境光 + 2 点光；阴影未绑定贴图，保持关闭）
    pub fn light_uniform(&self) -> super::lighting::LightUniform {
        use super::lighting::{DirectionalLight, LightUniform, PointLight, ShadowConfig};
        let sun = DirectionalLight::new(
            glam::Vec3::new(-0.4, 0.9, -0.3).normalize(),
            glam::Vec3::new(1.0, 0.95, 0.85),
            1.2,
        );
        let point_a = PointLight::new(
            glam::Vec3::new(0.0, 6.0, 0.0),
            glam::Vec3::new(0.9, 0.6, 0.4),
            1.5,
        );
        let point_b = PointLight::new(
            glam::Vec3::new(-24.0, 5.0, -16.0),
            glam::Vec3::new(0.4, 0.7, 1.0),
            1.0,
        );
        // 阴影贴图（2026-08-11）：正交光空间以地图中心为 target、半宽 250m，
        // 覆盖障碍环带（58-130m）与两军接火区；相机无现成引用，取原点近似。
        // ShadowConfig.light_dir 语义 = 表面→光源方向，与 sun.direction 一致直接传入
        // （旧实现传 -sun.direction 使光相机在地面下方仰视：阴影图只剩背面剔除后的
        //   竖面，地面/地形整片缺失，阴影完全失效）。
        // RV3D_NO_SHADOW=1 关闭阴影（仅环境光+点光源），用于 A/B 验证与阴影 pass 性能对比。
        let shadow = if std::env::var("RV3D_NO_SHADOW").as_deref() == Ok("1") {
            None
        } else {
            Some(ShadowConfig::new(sun.direction, glam::Vec3::ZERO, 250.0, 1.0, 500.0))
        };
        LightUniform::build(
            Some(&sun),
            &[point_a, point_b],
            glam::Vec3::new(0.5, 0.55, 0.6),
            0.35,
            shadow.as_ref(),
        )
    }

    /// 尝试开火（受射速冷却限制）。`origin`/`direction` 来自相机；返回是否真的开火。
    pub fn fire(&mut self, origin: [f32; 3], direction: [f32; 3]) -> bool {
        if self.fire_cooldown > 0.0 {
            return false;
        }
        // 切枪计时中禁止开火（纯计时器，无动画）
        if self.weapons.is_switching() {
            return false;
        }
        match self.weapons.active_firearm().try_fire(origin, direction) {
            Some(projectile) => {
                self.fire_cooldown = self.weapons.active_firearm_ref().fire_interval();
                let (kick_pitch, kick_yaw) = self.weapons.active_firearm_ref().current_kick();
                self.pending_kick.0 += kick_pitch;
                self.pending_kick.1 += kick_yaw;
                self.projectiles.push(projectile);
                self.shots += 1;
                // 程序化枪声：按武器音色参数（M1 清脆 crack / Thompson 低闷长尾），
                // 带确定性音量抖动（0.95..=1.0）避免机械重复
                let shot_scale = 0.95 + 0.05 * ((self.shots % 5) as f32 / 4.0);
                let shot_params = match self.weapons.active_name() {
                    "Thompson SMG" => crate::audio::THOMPSON_SHOT,
                    _ => crate::audio::M1_SHOT,
                };
                self.audio.synth_mut().play_shot_with(
                    glam::Vec3::new(origin[0], origin[1], origin[2]),
                    shot_scale,
                    shot_params,
                );
                log::info!(
                    "weapons: shot #{} ({} alive) [{}]",
                    self.shots,
                    self.projectiles.len(),
                    self.weapons.active_name()
                );
                true
            }
            None => {
                // 空弹匣自动换弹（try_fire 内部触发）或换弹中：换弹提示音
                if self.weapons.active_firearm_ref().is_reloading() {
                    let src = AudioSource::new(
                        glam::Vec3::new(origin[0], origin[1], origin[2]),
                        1.0,
                    );
                    self.sfx.play(
                        &mut self.audio.mixer_mut(),
                        SfxKind::Reload,
                        src,
                        Channel::Sfx,
                        false,
                    );
                }
                false
            }
        }
    }

    /// 累计命中数（供 UI / 日志）
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// 取走本帧开火累计的后坐力（pitch/yaw 弧度），由 main.rs 施加到相机
    pub fn drain_kick(&mut self) -> (f32, f32) {
        let kick = self.pending_kick;
        self.pending_kick = (0.0, 0.0);
        kick
    }

    /// 是否处于开火后坐期（fire_cooldown > 0 = 刚开火，枪模后坐动画用）
    pub fn is_firing(&self) -> bool {
        self.fire_cooldown > 0.0
    }

    /// 玩家脚底位置（世界坐标）
    pub fn player_pos(&self) -> glam::Vec3 {
        glam::Vec3::new(
            self.player_body.pos.x,
            self.player_body.pos.y,
            self.player_body.pos.z,
        )
    }

    /// 玩家眼睛位置（脚底 + 身高），main.rs 每帧同步给第一人称相机
    pub fn player_eye(&self) -> glam::Vec3 {
        glam::Vec3::new(
            self.player_body.pos.x,
            self.player_body.pos.y + self.player_body.eye_height,
            self.player_body.pos.z,
        )
    }

    /// 转发 WASD 按键状态（FPS 玩家移动；仅 Playing + 第一人称生效）
    pub fn set_movement(&mut self, forward: bool, backward: bool, left: bool, right: bool) {
        self.move_forward = forward;
        self.move_backward = backward;
        self.move_left = left;
        self.move_right = right;
    }

    /// 切换武器（数字键 1/2 或滚轮）：切换到指定槽位；切枪计时中忽略重复切换
    pub fn switch_weapon(&mut self, index: usize) {
        // 槽位越界防御（WeaponRack::len 暴露武器数量）
        if index >= self.weapons.len() {
            return;
        }
        self.weapons.switch_to(index);
    }

    /// 循环切换武器（滚轮向上 = 下一把，向下 = 上一把）
    pub fn cycle_weapon(&mut self, delta: i32) {
        if delta > 0 {
            self.weapons.switch_next();
        } else if delta < 0 {
            self.weapons.switch_prev();
        }
    }

    /// 请求换弹（R 键）；已在换弹/满弹匣/无备弹/切枪中时无副作用
    pub fn request_reload(&mut self) {
        if self.weapons.is_switching() {
            return;
        }
        let was_reloading = self.weapons.active_firearm_ref().is_reloading();
        self.weapons.active_firearm().start_reload();
        if !was_reloading && self.weapons.active_firearm_ref().is_reloading() {
            let src = AudioSource::new(self.player_eye(), 1.0);
            self.sfx.play(
                &mut self.audio.mixer_mut(),
                SfxKind::Reload,
                src,
                Channel::Sfx,
                false,
            );
        }
    }

    /// 调试补给（设置面板 N 键）：当前武器弹匣补满 + 手榴弹补满 + 提示音
    pub fn give_ammo(&mut self) {
        self.weapons.active_firearm().reset();
        self.grenades = self.grenades_max;
        let src = AudioSource::new(self.player_eye(), 1.0);
        self.sfx.play(
            &mut self.audio.mixer_mut(),
            SfxKind::UiBlip,
            src,
            Channel::Sfx,
            false,
        );
    }

        /// 当前飞行/落地手榴弹位置列表（渲染用：世界内可见手雷实体）
    pub fn grenade_positions(&self) -> Vec<[f32; 3]> {
        self.grenades_vec.iter().map(|g| g.position()).collect()
    }

    /// 投掷手榴弹（G 键）：库存 >0 且切枪/换弹中时投掷。方向 = 相机方向 + 上仰角
    /// （水平方向 + 0.25rad 上抛，保证抛物线落地）；引信 1.5-2.5s 确定性伪随机。
    pub fn throw_grenade(&mut self, origin: [f32; 3], direction: [f32; 3]) -> bool {
        if self.grenades == 0 || self.weapons.is_switching() {
            return false;
        }
        self.grenades -= 1;
        // 投掷哨声（高音下滑，与枪声区分）
        self.audio
            .synth_mut()
            .play_grenade_throw(glam::Vec3::new(origin[0], origin[1], origin[2]));
        // 上抛：水平方向（归一化）+ 固定上仰分量；Grenade::new 会归一化 dir 再乘 speed
        let mut dir = glam::Vec3::new(direction[0], direction[1], direction[2]);
        if dir.length_squared() < 1e-6 {
            dir = glam::Vec3::Z;
        }
        dir.y = 0.0;
        if dir.length_squared() < 1e-6 {
            dir = glam::Vec3::Z;
        }
        let dir = dir.normalize();
        let vx = dir.x;
        let vz = dir.z;
        let vy = 0.35; // 上抛分量（竖直向上 ≈ 35% 总初速）
        // 引信：确定性伪随机（随投掷次数变化），落在 1.5-2.5s
        let fuse = GRENADE_FUSE_MIN
            + (self.shots.wrapping_mul(13) % 1000) as f32 / 1000.0
                * (GRENADE_FUSE_MAX - GRENADE_FUSE_MIN);
        self.grenades_vec.push(Grenade::new(
            origin,
            [vx, vy, vz],
            GRENADE_SPEED,
            fuse,
        ));
        log::info!(
            "grenade: thrown #{} fuse={:.2}s remaining={}",
            self.grenades_vec.len(),
            fuse,
            self.grenades
        );
        true
    }

    /// 手榴弹每帧推进：抛物线 + 引信；到期 → 爆炸（复用 spawn_explosion：AoE 伤害 +
    /// 径向击退 + 震屏 + 自发光闪光）。落地（y ≤ 地形高度）也触发爆炸。
    fn update_grenades(&mut self, dt: f32) {
        let mut explosions: Vec<[f32; 3]> = Vec::new();
        for g in &mut self.grenades_vec {
            g.update(dt);
            // 落地或引信到期 → 爆炸
            let ground = terrain_height_at(g.position()[0], g.position()[2]);
            if g.exploded() || g.position()[1] <= ground + 0.05 {
                // 落地滚动音（短促低音 thud；爆炸音由 spawn_explosion 的 SfxKind::Explosion 承担）
                self.audio
                    .synth_mut()
                    .play_grenade_bounce(glam::Vec3::new(g.position()[0], g.position()[1], g.position()[2]));
                log::info!(
                    "grenade: detonate fuse_max={:.2}s",
                    g.fuse_max()
                );
                explosions.push(g.position());
            }
        }
        if !explosions.is_empty() {
            self.grenades_vec.retain(|g| !g.exploded() && g.position()[1] > terrain_height_at(g.position()[0], g.position()[2]) + 0.05);
            for center in explosions {
                // 手榴弹 AoE：半径 8m、伤害 120（近距可秒标准 NPC）、径向击退
                self.spawn_explosion(center, GRENADE_EXPLOSION_RADIUS, GRENADE_EXPLOSION_DAMAGE, true);
            }
        }
    }

    /// NPC 投掷手榴弹（压力模式 / survive 防守波次）：交火中（Attack 态）的 NPC 按
    /// 确定性概率（5-8%，随 id/帧哈希）向敌对目标投掷；冷却 10-18s 确定性伪随机。
    /// - 普通波次（打玩家）不调用 → AI 行为零回归；
    /// - 压力模式目标 = pick_stress_targets 的敌对 NPC（阵营区分，目标与投掷者异阵营）；
    /// - survive 目标 = 玩家（防守方 NPC 进攻）。
    fn npc_throw_grenades(&mut self, dt: f32, targets: &[Option<(usize, [f32; 3], f32)>]) {
        if dt <= 0.0 {
            return;
        }
        // 先递减冷却
        for npc in &mut self.npcs {
            npc.grenade_timer = (npc.grenade_timer - dt).max(0.0);
        }
        let mut to_throw: Vec<(usize, [f32; 3])> = Vec::new(); // (npc_idx, 目标位置)
        for (i, npc) in self.npcs.iter().enumerate() {
            if npc.grenade_timer > 0.0 || npc.state_machine.state() != NpcState::Attack {
                continue;
            }
            // 确定性概率：id/帧哈希 → 5-8%（投掷窗口内约每 12-20 帧判定一次）
            let h = (npc.id as u64 * 31 + self.frame_no as u64 * 7) % 100;
            if h >= 8 {
                continue;
            }
            // 目标：压力模式 = 敌对 NPC（异阵营）；survive = 玩家
            let target_pos = if self.stress {
                targets
                    .get(i)
                    .and_then(|t| t.as_ref())
                    .filter(|(_, _, _)| {
                        // 只投掷异阵营目标（pick_stress_targets 已保证异阵营，此处冗余防御）
                        true
                    })
                    .map(|(_, tp, _)| *tp)
            } else {
                Some([
                    self.player_body.pos.x,
                    self.player_body.pos.y,
                    self.player_body.pos.z,
                ])
            };
            if let Some(tp) = target_pos {
                to_throw.push((i, tp));
            }
        }
        for (i, tp) in to_throw {
            let npc = &self.npcs[i];
            let origin = npc.position;
            // 方向：本 NPC → 目标（水平）+ 上抛分量（与玩家投掷同链路，参数化）
            let dx = tp[0] - origin[0];
            let dz = tp[2] - origin[2];
            let len = (dx * dx + dz * dz).sqrt().max(1e-4);
            let dir = [dx / len, 0.35, dz / len];
            let fuse = GRENADE_FUSE_MIN
                + (npc.id as u32 * 17 % 1000) as f32 / 1000.0
                    * (GRENADE_FUSE_MAX - GRENADE_FUSE_MIN);
            self.grenades_vec
                .push(Grenade::new(origin, dir, GRENADE_SPEED * 0.9, fuse));
            log::info!(
                "grenade: npc #{} throws at ({:.0}, {:.0}) fuse={:.2}s",
                npc.id,
                tp[0],
                tp[2],
                fuse
            );
            // 冷却 10-18s（确定性伪随机）
            if let Some(npc) = self.npcs.get_mut(i) {
                npc.grenade_timer = 10.0 + (npc.id as f32 * 3.7 % 8.0);
            }
        }
    }

    /// 设置面板开关（ESC 切换）：开/关都播提示音
    pub fn toggle_settings(&mut self) {
        let was_open = self.hud.settings_open;
        self.hud.toggle_settings();
        if was_open {
            // 关闭设置面板：取消进行中的键位绑定，并持久化键位/音量/灵敏度
            self.hud.cancel_rebind();
            crate::config::save(&self.current_config());
        }
        let src = AudioSource::new(self.player_eye(), 1.0);
        self.sfx.play(
            &mut self.audio.mixer_mut(),
            SfxKind::UiBlip,
            src,
            Channel::Sfx,
            false,
        );
    }

    /// 设置面板是否打开（main.rs 据此拦截游戏输入/释放光标）
    pub fn settings_open(&self) -> bool {
        self.hud.settings_open
    }

    /// 循环切换设置面板选中项（音量/灵敏度/7 个键位动作）
    pub fn cycle_settings(&mut self) {
        self.hud.cycle_settings_selection();
    }

    /// 按当前选中项调整设置（滚轮 delta）
    pub fn adjust_settings(&mut self, delta: f32) {
        if self.hud.settings_selection == 0 {
            self.hud.adjust_volume(delta);
        } else if self.hud.settings_selection == 1 {
            self.hud.adjust_sensitivity(delta);
        } else if self.hud.settings_selection == 2 {
            self.hud.adjust_music_volume(delta);
        }
        // selection >= 3 是分辨率/画质/键位行，滚轮不做调整（Enter 进入绑定）
    }

    /// 灵敏度（0..=1）→ 相机 rad/px（0.0005..=0.0025，默认 0.5 → 0.0015）
    pub fn sensitivity_rads(&self) -> f32 {
        0.0005 + self.hud.sensitivity * 0.002
    }

    /// 第一人称玩家移动：WASD 相对相机朝向，与演示刚体碰撞推回，y 每帧贴地形
    fn move_first_person(&mut self, camera: &Camera, dt: f32) {
        let fwd = glam::Vec3::new(camera.forward().x, 0.0, camera.forward().z).normalize_or_zero();
        let right = camera.right();
        let mut dx = 0.0f32;
        let mut dz = 0.0f32;
        if self.move_forward {
            dx += fwd.x;
            dz += fwd.z;
        }
        if self.move_backward {
            dx -= fwd.x;
            dz -= fwd.z;
        }
        if self.move_right {
            dx += right.x;
            dz += right.z;
        }
        if self.move_left {
            dx -= right.x;
            dz -= right.z;
        }
        let len = (dx * dx + dz * dz).sqrt();
        if len > 1e-4 {
            let step = (PLAYER_SPEED * dt).min(0.5);
            let (mx, mz) = self
                .player_body
                .try_move(&self.world, dx / len * step, dz / len * step);
            let moved = (mx * mx + mz * mz).sqrt();
            if moved > 0.01 && self.time - self.footstep_timer >= FOOTSTEP_INTERVAL {
                self.footstep_timer = self.time;
                // 程序化脚步：短促宽带噪声，交替强弱（0.8 / 1.0）确定性变化
                let step_scale = 0.8 + 0.2 * ((self.time * 2.0) as u32 % 2) as f32;
                let pos = self.player_pos();
                self.audio.synth_mut().play_footstep(pos, step_scale);
            }
        }
        self.player_body.pos.y = terrain_height_at(self.player_body.pos.x, self.player_body.pos.z);
        self.player_body.grounded = true;
    }

    /// 投射物推进 + 碰撞检测：物理刚体/球体命中即销毁；NPC 命中扣血，hp≤0 移除并计分。
    ///
    /// `allow_kills = false`（GameOver 冻结）：投射物照常飞行/到期，但不判定任何命中。
    fn update_projectiles(&mut self, dt: f32, allow_kills: bool) {
        for p in self.projectiles.iter_mut() {
            if p.is_alive() {
                p.update(dt);
            }
        }
        let mut hit_count = 0u32;
        let old = std::mem::take(&mut self.projectiles);
        let mut alive = Vec::with_capacity(old.len());
        for p in old {
            if !p.is_alive() {
                // 弹着点：过期投射物引爆 —— AoE 伤害 + 冲击波击退 + 闪光 marker
                // （GameOver 冻结玩法：damage=0 只保留视觉与音效，不结算伤害/击退）
                self.spawn_explosion(
                    p.position,
                    EXPLOSION_RADIUS,
                    if allow_kills { EXPLOSION_DAMAGE } else { 0.0 },
                    true,
                );
                self.audio.synth_mut().play_explosion(
                    glam::Vec3::new(p.position[0], p.position[1], p.position[2]),
                    0.45,
                );
                continue;
            }
            if !allow_kills {
                alive.push(p);
                continue;
            }
            if self.collide_physics(&p) {
                hit_count += 1;
                // 命中障碍：爆炸 AoE（对附近 NPC 造成冲击波伤害 + 闪光，但不推挤，
                // 保持冒烟"站定瞄准"语义）+ 按武器伤害结算障碍 HP，摧毁后移除碰撞/阻挡
                if let Some(idx) = self.hit_obstacle_index(&p) {
                    self.spawn_explosion(p.position, EXPLOSION_RADIUS, EXPLOSION_DAMAGE, false);
                    self.damage_obstacle(idx, p.damage);
                }
                continue;
            }
            if let Some(idx) = self.hit_npc_index(&p) {
                hit_count += 1;
                self.damage_npc(idx, p.damage);
                continue;
            }
            alive.push(p);
        }
        self.projectiles = alive;
        self.hits += hit_count as u64;
        if hit_count > 0 {
            self.hud.show_hit_marker();
            let src = AudioSource::new(self.player_eye(), 1.0);
            self.sfx.play(
                &mut self.audio.mixer_mut(),
                SfxKind::Hit,
                src,
                Channel::Sfx,
                false,
            );
            log::info!(
                "weapons: {} projectile hit(s), total_hits={}",
                hit_count,
                self.hits
            );
        }
    }

    /// 投射物是否命中物理刚体/球体（命中即销毁，不产生击杀）
    fn collide_physics(&self, p: &Projectile) -> bool {
        let (px, py, pz) = (p.position[0], p.position[1], p.position[2]);
        for body in &self.world.bodies {
            let aabb = body.aabb();
            if px >= aabb.min.x && px <= aabb.max.x
                && py >= aabb.min.y && py <= aabb.max.y
                && pz >= aabb.min.z && pz <= aabb.max.z {
                return true;
            }
        }
        for s in &self.world.spheres {
            let dx = px - s.center.x;
            let dy = py - s.center.y;
            let dz = pz - s.center.z;
            if dx * dx + dy * dy + dz * dz <= s.radius * s.radius {
                return true;
            }
        }
        false
    }

    /// 投射物命中的障碍刚体下标（球体/未命中返回 None）。
    /// 下标与 `map.obstacles`/`world.bodies` 严格对应（apply_level 同序构建、同序移除）。
    fn hit_obstacle_index(&self, p: &Projectile) -> Option<usize> {
        let (px, py, pz) = (p.position[0], p.position[1], p.position[2]);
        self.world.bodies.iter().position(|body| {
            let aabb = body.aabb();
            px >= aabb.min.x
                && px <= aabb.max.x
                && py >= aabb.min.y
                && py <= aabb.max.y
                && pz >= aabb.min.z
                && pz <= aabb.max.z
        })
    }

    /// 障碍受伤结算：扣血至 0 → 摧毁（从物理刚体/AI 网格/渲染 marker 中移除）。
    /// 渲染侧无需改动：main.rs 每帧按 `map_obstacles()` 生成 marker，摧毁后自动不再绘制。
    fn damage_obstacle(&mut self, idx: usize, dmg: f32) {
        let Some(ob) = self.map.obstacles.get_mut(idx) else {
            return;
        };
        ob.hp = (ob.hp - dmg).max(0.0);
        if ob.hp > 0.0 {
            return;
        }
        let ob = *ob; // Copy：记录位置/尺寸供解除阻挡
        let kind = ob.kind;
        // 解除 AI 网格阻挡：NPC 寻路可穿过缺口，掩体点随之消失
        let g0 = world_to_grid(ob.x - ob.half_w, ob.z - ob.half_d);
        let g1 = world_to_grid(ob.x + ob.half_w, ob.z + ob.half_d);
        for gx in g0.x..=g1.x {
            for gz in g0.y..=g1.y {
                let pos = GridPos::new(gx, gz);
                if self.grid.in_bounds(pos) {
                    self.grid.clear(pos);
                }
            }
        }
        log::info!(
            "obstacle: #{} {:?} destroyed at ({:.1}, {:.1})",
            idx,
            kind,
            ob.x,
            ob.z
        );
        // world.bodies 与 map.obstacles 按序一一对应（程序化/关卡生成时同步建刚体）；
        // 测试注入的障碍无对应刚体 → 容忍 idx 越界（跳过刚体移除）
        if idx < self.world.bodies.len() {
            self.world.bodies.remove(idx);
        }
        self.map.obstacles.remove(idx);
    }

    /// NPC 受伤结算：扣血至 0 → 移除 + 计分 + 任务目标推进；返回是否击杀。
    /// 调用方保证 `idx` 有效；下标移除后不再回移（调用方按逆序遍历或立即退出）。
    fn damage_npc(&mut self, idx: usize, dmg: f32) -> bool {
        let npc = &mut self.npcs[idx];
        npc.hp -= dmg;
        if npc.hp > 0.0 {
            return false;
        }
        let id = npc.id;
        let victim_team = npc.team;
        self.npcs.remove(idx);
        self.score += KILL_SCORE;
        // 击杀提示（右上角 feed）：玩家击杀敌方 NPC
        self.hud.push_kill(format!("YOU KILLED {} #{}", team_name(victim_team), id));
        log::info!(
            "kill: npc #{} eliminated (wave {}) score={}",
            id,
            self.wave,
            self.score
        );
        // 任务目标：歼灭数推进（达成 → 胜利横幅/日志，波次推进不受影响）
        if self.objective.progress(1) {
            self.on_objective_complete();
        }
        // 关卡系统：击杀计数（KillCount 规则用）
        self.objective_register_kill();
        true
    }

    /// 生成爆炸实体：AoE 伤害 + 径向击退（生成时一次性结算，逆序遍历避免下标回移）。
    /// - 伤害衰减复用 `simd::shockwave_pressure`（所有 NPC 一次批算，指令集选路可测）；
    /// - `knockback=true` 时对命中 NPC 施加径向推挤（advance_npc 每帧指数衰减）；
    /// - 玩家在冲击半径内 → 震屏（`camera_shake_offset` 每帧读取）。
    fn spawn_explosion(&mut self, center: [f32; 3], radius: f32, damage: f32, knockback: bool) {
        log::info!(
            "explosion: at ({:.1}, {:.1}, {:.1}) radius={:.0} dmg={:.0} knockback={}",
            center[0],
            center[1],
            center[2],
            radius,
            damage,
            knockback
        );
        self.explosions.push(Explosion {
            center,
            radius,
            max_damage: damage,
            age: 0.0,
            lifetime: EXPLOSION_LIFETIME,
        });
        // 玩家震屏 + 自伤：随距离线性衰减，最近处满强度（与 NPC 是否在场无关）。
        // 玩家自伤伤害封顶（max_damage * SELF_DAMAGE_CAP），爆炸中心偏移保证不被自己秒杀
        // （手榴弹上抛飞行 ~0.4s + 引信 1.5s → 玩家通常已远离落地中心）。
        let eye = self.player_eye();
        let dx = eye.x - center[0];
        let dz = eye.z - center[2];
        let dist = (dx * dx + dz * dz).sqrt();
        if dist < SHAKE_RADIUS {
            self.shake_timer = SHAKE_DURATION;
            self.shake_strength = SHAKE_STRENGTH * (1.0 - dist / SHAKE_RADIUS).clamp(0.15, 1.0);
        }
        // 玩家自伤：仅在半径内且游戏进行中（结算画面不扣血）；伤害 = 距离衰减 × 封顶系数
        if damage > 0.0
            && dist < radius
            && self.game_state == GameState::Playing
            && self.hud.health > 0.0
        {
            let fall = 1.0 - (dist / radius).clamp(0.0, 1.0);
            let self_dmg = (damage * fall * SELF_DAMAGE_FACTOR).min(SELF_DAMAGE_CAP);
            if self_dmg > 0.0 {
                self.hud.health = (self.hud.health - self_dmg).max(0.0);
                log::info!(
                    "explosion: 玩家自伤 {:.0}（dist {:.1}m fall {:.2}）→ hp {:.0}",
                    self_dmg,
                    dist,
                    fall,
                    self.hud.health
                );
                if self.hud.health <= 0.0 {
                    // 手榴弹炸死自己：普通模式 GameOver；survive 规则 Defeat
                    if self.is_survive_rule() {
                        if let Some(obj) = self.obj_state.as_mut() {
                            obj.won_team = Some(crate::engine::ai::Team::Red);
                        }
                        self.game_state = GameState::Defeat;
                    } else {
                        self.game_state = GameState::GameOver;
                    }
                    log::info!("explosion: 玩家被自己手榴弹炸死");
                }
            }
        }
        // 障碍 AoE 伤害：爆炸半径内障碍施加冲击伤害（复用 damage_obstacle 血量体系，
        // 可摧毁掩体；环带障碍与 TOML 关卡障碍一致生效）。按距离线性衰减（半径边缘 0）。
        if damage > 0.0 && !self.map.obstacles.is_empty() {
            let r2 = radius * radius;
            let mut i = self.map.obstacles.len();
            while i > 0 {
                i -= 1;
                let ob = self.map.obstacles[i];
                // 障碍中心到爆炸中心距离（水平）
                let dx = ob.x - center[0];
                let dz = ob.z - center[2];
                let d2 = dx * dx + dz * dz;
                if d2 > r2 {
                    continue;
                }
                let fall = 1.0 - (d2.sqrt() / radius).clamp(0.0, 1.0);
                self.damage_obstacle(i, damage * fall * EXPLOSION_OBSTACLE_FACTOR);
            }
        }
        if damage <= 0.0 || self.npcs.is_empty() {
            return;
        }
        let points: Vec<[f32; 3]> = self.npcs.iter().map(|n| n.position).collect();
        let mut falloff = vec![0.0f32; points.len()];
        crate::engine::simd::shockwave_pressure(center, radius, 1.0, &points, &mut falloff);
        let mut i = self.npcs.len();
        while i > 0 {
            i -= 1;
            let f = falloff[i];
            if f <= 0.0 {
                continue;
            }
            if knockback {
                let dx = self.npcs[i].position[0] - center[0];
                let dz = self.npcs[i].position[2] - center[2];
                let d = (dx * dx + dz * dz).sqrt().max(1e-4);
                self.npcs[i].knockback[0] += dx / d * KNOCKBACK_SPEED * f;
                self.npcs[i].knockback[1] += dz / d * KNOCKBACK_SPEED * f;
            }
            self.damage_npc(i, damage * f);
        }
    }

    /// 推进爆炸实体生命周期（年龄 + 过期移除）与震屏衰减。每帧调用（update 尾部）。
    fn step_explosions(&mut self, dt: f32) {
        for ex in self.explosions.iter_mut() {
            ex.age += dt;
        }
        self.explosions.retain(|ex| ex.age < ex.lifetime);
        self.shake_timer = (self.shake_timer - dt).max(0.0);
    }

    /// 当前爆炸实体（main.rs 每帧生成膨胀淡出的闪光 marker）
    pub fn explosions(&self) -> &[Explosion] {
        &self.explosions
    }

    /// 本帧爆炸震屏偏移（世界 x/z 抖动，随剩余时间线性衰减）；无震屏时返回 (0, 0)。
    pub fn camera_shake_offset(&self) -> (f32, f32) {
        if self.shake_timer <= 0.0 {
            return (0.0, 0.0);
        }
        let s = self.shake_strength * (self.shake_timer / SHAKE_DURATION);
        let t = self.time;
        ((t * 47.13).sin() * s, (t * 53.71).cos() * s)
    }

    /// 投射物命中的 NPC 下标（segment-sphere 相交：上一帧位置→当前位置连线与命中球求交，
    /// 命中球中心在 NPC 头顶 +0.8、半径 0.8；高速弹（200m/s 每帧 3.3m）避免隧道效应漏判）
    fn hit_npc_index(&self, p: &Projectile) -> Option<usize> {
        let (ax, ay, az) = (p.prev_position()[0], p.prev_position()[1], p.prev_position()[2]);
        let (bx, by, bz) = (p.position[0], p.position[1], p.position[2]);
        let (dx, dy, dz) = (bx - ax, by - ay, bz - az);
        let len2 = dx * dx + dy * dy + dz * dz;
        if len2 < 1e-9 {
            return None;
        }
        for (i, npc) in self.npcs.iter().enumerate() {
            let cx = npc.position[0];
            let cy = npc.position[1] + 0.8;
            let cz = npc.position[2];
            let r = 0.8;
            // 点到射线最近点参数 t（clamp 到 [0,1] 段内），再算距离²
            let fx = ax - cx;
            let fy = ay - cy;
            let fz = az - cz;
            let t = -(fx * dx + fy * dy + fz * dz) / len2;
            let t = t.clamp(0.0, 1.0);
            let qx = fx + t * dx;
            let qy = fy + t * dy;
            let qz = fz + t * dz;
            if qx * qx + qy * qy + qz * qz <= r * r {
                return Some(i);
            }
        }
        None
    }

    /// 波次推进：全部 NPC hp≤0 移除（`npcs` 为空）才算清空；清空后 3 秒倒计时刷下一波。
    ///
    /// 跑远的存活 NPC 仍留在列表里，不算清空（必须击杀全部）。
    fn update_waves(&mut self, dt: f32, player: &glam::Vec3) {
        // 援军波：波开始后 reinforcement_at 秒补怪 1..=2 只；波已清空则不再补（清波条件不变）
        let profile = wave_profile(self.effective_wave(self.wave));
        if !self.reinforcement_done
            && profile.kind == WaveKind::Reinforced
            && !self.npcs.is_empty()
            && self.time - self.wave_started_at >= profile.reinforcement_at.unwrap_or(f32::MAX)
        {
            self.reinforcement_done = true;
            let slot_base = (profile.count as f32 * self.npc_scale).round().max(1.0) as u32;
            let divisor = slot_base.max(1);
            let effective = self.effective_wave(self.wave);
            for k in 0..profile.reinforcement_count {
                self.spawn_npc_ring(
                    player,
                    slot_base + k,
                    divisor,
                    self.wave,
                    profile.speed,
                    profile.hp,
                    profile.attack_range,
                    role_for(slot_base + k, effective, profile.flank_chance),
                );
            }
            log::info!(
                "wave: reinforcement +{} on wave {}",
                profile.reinforcement_count,
                self.wave
            );
        }
        if self.npcs.is_empty() {
            if self.wave_timer <= 0.0 {
                self.wave_timer = WAVE_INTERMISSION;
                self.score += WAVE_CLEAR_BONUS;
                log::info!(
                    "wave: wave {} cleared (+{}), next in {:.0}s",
                    self.wave,
                    WAVE_CLEAR_BONUS,
                    WAVE_INTERMISSION
                );
            } else {
                self.wave_timer -= dt;
                if self.wave_timer <= 0.0 {
                    // 每关 WAVES_PER_LEVEL 波清完 → 升关：重新生成地图并回到本关第 1 波；
                    // 难度按累计有效波次递进（effective_wave），跨关不回落
                    // survive 规则：总波数 = rule.waves，守住全部波 → 胜利（补给窗口后进入胜利态）
                    if self.is_survive_rule() && self.wave >= self.survive_total_waves() {
                        self.hud.victory_banner = Some("防区固守！全部波次守住".to_string());
                        self.game_state = GameState::Victory(crate::engine::ai::Team::Blue);
                        self.set_won_team(crate::engine::ai::Team::Blue);
                        log::info!(
                            "survive: 全部 {} 波守住 → 胜利",
                            self.survive_total_waves()
                        );
                    } else if self.wave >= WAVES_PER_LEVEL {
                        let next_level = self.level + 1;
                        self.apply_level(next_level);
                        self.wave = 1;
                        log::info!(
                            "level: advanced to level {} (map regenerated)",
                            next_level
                        );
                    } else {
                        self.wave += 1;
                        // survive：波间补给窗口（血量回复 + 弹药补满）
                        if self.is_survive_rule() {
                            self.supply_survive_break();
                        }
                    }
                    self.spawn_wave(self.wave, player);
                }
            }
        }
    }

    /// 生成第 n 波敌人：数量/速度/血量随波次递进，环形出生在玩家周围。
    ///
    /// 出生前清掉残留存活 NPC，保证新旧波不共存；
    /// Boss 波最后一只为主怪（高血量/慢速/攻击距离略长，见 wave_profile）；援军波重置补怪计时。
    fn spawn_wave(&mut self, n: u32, player: &glam::Vec3) {
        // 同步波次号：update_waves/update_ai 都按 self.wave 取 profile，直接调用时也必须一致
        self.wave = n;
        if !self.npcs.is_empty() {
            log::info!(
                "wave: purged {} leftover npcs before wave {}",
                self.npcs.len(),
                n
            );
            self.npcs.clear();
        }
        // 难度按累计有效波次：跨关不回落（level 2 第 1 波 ≈ 原第 4 波强度）
        let effective = self.effective_wave(n);
        let profile = wave_profile(effective);
        // NPC 数量按 RV3D_NPC_SCALE 缩放（默认 1.0；测试/冒烟不设变量行为不变）
        let count = (profile.count as f32 * self.npc_scale).round().max(1.0) as usize;
        let speed = profile.speed;
        let hp = profile.hp;
        let attack_range = profile.attack_range;
        for i in 0..count {
            // Boss 波最后一只为主怪：替换常规小怪，max_hp 大 → 渲染侧体型/外观体现
            let (spd, hpx, rng) = match profile.boss {
                Some(b) if i + 1 == count => (b.speed, b.hp, b.attack_range),
                _ => (speed, hp, attack_range),
            };
            // Boss 主怪固定突击角色：高血量压阵直线推进（保证参团冲锋，不绕侧/站桩）
            let role = if profile.boss.is_some() && i + 1 == count {
                TacticalRole::Rusher
            } else {
                role_for(i as u32, effective, profile.flank_chance)
            };
            let id = self.spawn_npc_ring(player, i as u32, count as u32, n, spd, hpx, rng, role);
            if profile.boss.is_some() && i + 1 == count {
                log::info!(
                    "wave: boss #{} spawn (hp={:.0} speed={:.1} attack={:.0})",
                    id,
                    hpx,
                    spd,
                    rng
                );
            }
        }
        // 援军波计时基准：波开始时间 + 补怪标志重置
        self.wave_started_at = self.time;
        self.reinforcement_done = false;
        log::info!(
            "wave: wave {} spawned {} enemies (kind={:?} total={} speed={:.1} hp={:.0} effective={})",
            n,
            count,
            profile.kind,
            profile.total_count,
            speed,
            hp,
            effective
        );
    }

    /// 环形出生一只 NPC：按 `slot/divisor` 均分角度 + 波次相位，半径 40..80m 确定性抖动，
    /// 出生点避开障碍格（沿径向外推最多 8 步，每步 4m）。返回新 NPC id。
    fn spawn_npc_ring(
        &mut self,
        player: &glam::Vec3,
        slot: u32,
        divisor: u32,
        wave_n: u32,
        speed: f32,
        hp: f32,
        attack_range: f32,
        role: TacticalRole,
    ) -> usize {
        let tau = std::f32::consts::TAU;
        let angle = slot as f32 * (tau / divisor.max(1) as f32) + wave_n as f32 * 0.37;
        let radius = 40.0 + 40.0 * ((slot * 7 + wave_n * 3) % 5) as f32 / 4.0;
        let (x, z) = self.push_out_of_obstacle(
            (player.x + angle.cos() * radius).clamp(-250.0, 250.0),
            (player.z + angle.sin() * radius).clamp(-250.0, 250.0),
        );
        let id = self.next_npc_id as usize;
        self.next_npc_id += 1;
        let y = terrain_height_at(x, z);
        log::info!("wave: npc #{} spawn ({:.1}, {:.1}, {:.1})", id, x, y, z);
        self.npcs.push(Npc {
            id,
            position: [x, y, z],
            speed,
            attack_range,
            home: [x, z],
            state_machine: NpcStateMachine::new(),
            perception: NpcPerception::default(),
            path: Vec::new(),
            path_index: 0,
            hp,
            max_hp: hp,
            role,
            tactic: Tactic::Advance,
            dodge_timer: 0.0,
            hit_cooldown: 0.0,
            last_hp: hp,
            team: Team::Red,
            facing: (player.z - z).atan2(player.x - x),
            fire_accum: 0.0,
            knockback: [0.0, 0.0],
            grenade_timer: 0.0,
        });
        id
    }

    /// 出生点避开障碍盒（网格阻挡格）：沿径向向外推，最多 8 步（每步 4m），确定性。
    /// 普通波次与压力模式共用。
    fn push_out_of_obstacle(&self, x: f32, z: f32) -> (f32, f32) {
        let mut sx = x;
        let mut sz = z;
        for _ in 0..8 {
            if self.grid.is_passable(world_to_grid(sx, sz)) {
                break;
            }
            let d = (sx * sx + sz * sz).sqrt().max(1.0);
            sx += sx / d * 4.0;
            sz += sz / d * 4.0;
        }
        (sx.clamp(-250.0, 250.0), sz.clamp(-250.0, 250.0))
    }

    /// 压力模式开战：红蓝各 `stress_sides` 名 NPC 分两半场环形出生（半径 150m+，避障外推），
    /// 角色/速度/血量/攻击距离按第 1 波 profile 确定性分配。清掉旧 NPC（全量重开一轮）。
    fn spawn_stress_battle(&mut self, player: &glam::Vec3) {
        self.npcs.clear();
        // 新一轮：任务目标重置为本轮歼灭一队（补员/波次逻辑不变）；
        // 上一轮胜利横幅保留到下一轮（start_run/apply_level 时才清空）
        self.objective = MissionObjective::new(self.stress_sides as u32);
        let sides = self.stress_sides as u32;
        let profile = wave_profile(self.effective_wave(1));
        for side in 0..2u32 {
            let team = if side == 0 { Team::Red } else { Team::Blue };
            let base_angle = if side == 0 { 0.0 } else { std::f32::consts::PI };
            for i in 0..sides {
                // 半场 ±~63° 扇形铺开，半径 150m + 确定性抖动（超出障碍环带 58-130m）
                let spread = -1.1 + (i as f32 / sides.max(1) as f32) * 2.2;
                let angle = base_angle + spread;
                let radius = STRESS_SPAWN_RADIUS + 12.0 * ((i * 7 + side) % 5) as f32;
                let (x, z) = self.push_out_of_obstacle(
                    (player.x + angle.cos() * radius).clamp(-250.0, 250.0),
                    (player.z + angle.sin() * radius).clamp(-250.0, 250.0),
                );
                let id = self.next_npc_id as usize;
                self.next_npc_id += 1;
                let y = terrain_height_at(x, z);
                self.npcs.push(Npc {
                    id,
                    position: [x, y, z],
                    speed: profile.speed,
                    attack_range: profile.attack_range,
                    home: [x, z],
                    state_machine: NpcStateMachine::new(),
                    perception: NpcPerception::default(),
                    path: Vec::new(),
                    path_index: 0,
                    hp: profile.hp,
                    max_hp: profile.hp,
                    role: role_for(i, 1, profile.flank_chance),
                    tactic: Tactic::Advance,
                    dodge_timer: 0.0,
                    hit_cooldown: 0.0,
                    last_hp: profile.hp,
                    team,
                    facing: (player.z - z).atan2(player.x - x),
                    fire_accum: 0.0,
                    knockback: [0.0, 0.0],
                grenade_timer: 0.0,
                });
            }
        }
        log::info!(
            "battle: 压力模式第 {} 轮开战（红 {} vs 蓝 {}，共 {} 名 NPC，并行 AI={}）",
            self.stress_round,
            sides,
            sides,
            self.npcs.len(),
            self.ai_parallel
        );
    }

    /// 推进单个 NPC：感知 → 状态机 → 战术决策 → 躲避 → A* 路径 → 移动 → 朝向。
    /// 与旧版串行循环体逐行为一致（普通波次目标=玩家，行为不变；压力模式目标=敌对 NPC）。
    fn step_npc(index: usize, npc: &mut Npc, ctx: &AiStepCtx) {
        // 视野半径：压力模式全场可见（两军立即接火），普通模式保持原值
        let sight = if ctx.stress { STRESS_SIGHT } else { NPC_SIGHT };
        // 目标位置：压力模式取预选敌对 NPC（快照位置 + 朝向），普通模式恒为玩家
        let target_pos = match (ctx.stress, ctx.targets.get(index).copied().flatten()) {
            (true, Some((_, tp, _))) => glam::Vec3::new(tp[0], 0.0, tp[2]),
            _ => *ctx.player,
        };
        let dx = npc.position[0] - target_pos.x;
        let dz = npc.position[2] - target_pos.z;
        let dist = (dx * dx + dz * dz).sqrt();
        let yaw_to = yaw_to_target(target_pos.x, target_pos.z, npc.position[0], npc.position[2]);
        let facing_angle = angle_diff(ctx.player_yaw, yaw_to).abs();
        // 绕背判定用的"目标朝向"：普通模式 = 玩家视角；压力模式 = 朝向目标 NPC 的方向
        // （facing 坐标系：atan2(dz,dx)，与 npc.facing 同源可比）
        let target_yaw = if ctx.stress && ctx.targets.get(index).copied().flatten().is_some() {
            (npc.position[2] - target_pos.z).atan2(npc.position[0] - target_pos.x)
        } else {
            ctx.player_yaw
        };
        // 「目标是否面朝本 NPC」：普通模式 = 玩家视角（行为不变）；压力模式 = 目标 NPC 的
        // 朝向 facing 是否大致指向本 NPC（总指挥指令单 #1 阶段二：让 NPC-vs-NPC 触发包抄/偷袭）。
        // 坐标系：target_yaw = (本NPC.z - 目标.z).atan2(本NPC.x - 目标.x) 即「目标→本NPC」方向，
        // 与 npc.facing（atan2(dz,dx)）同源可直接比。angle_diff 已处理 ±π 环绕。
        let target_facing = match (ctx.stress, ctx.targets.get(index).copied().flatten()) {
            (true, Some((_, _, tf))) => angle_diff(tf, target_yaw).abs() < std::f32::consts::FRAC_PI_2,
            _ => facing_angle < std::f32::consts::FRAC_PI_2,
        };
        let prev = npc.state_machine.state();
        let took_hit = npc.hp < npc.last_hp - 0.001;
        let under_fire = ctx.under_fire.get(index).copied().unwrap_or(false);
        npc.perception = NpcPerception {
            enemy_visible: dist < sight,
            enemy_in_range: dist < npc.attack_range,
            start_patrol: prev == NpcState::Idle,
            patrol_finished: false,
            player_aiming: facing_angle < AIM_ANGLE && dist < sight,
            player_facing: target_facing,
            took_hit,
            low_hp: npc.hp < npc.max_hp * LOW_HP_RATIO,
            under_fire,
        };
        let state = npc.state_machine.update(npc.perception);
        // 躲避触发：仅移动态（Attack 站定是冒烟瞄准依据）；受击反应更强、冷却更久
        if state != NpcState::Attack
            && npc.hit_cooldown <= 0.0
            && dist < DODGE_TRIGGER_DIST
            && (took_hit || under_fire)
        {
            npc.dodge_timer = if took_hit {
                DODGE_HIT_TIME
            } else {
                DODGE_THREAT_TIME
            };
            npc.hit_cooldown = DODGE_COOLDOWN;
        }
        npc.last_hp = npc.hp;
        // 战术决策：低血量撤退 / 角色行为 / 目标是否面朝（偷袭）；冲锋覆盖为突进。
        // Flanker 的包抄/偷袭战术不被冲锋覆盖（保持侧翼机动，实现互射战场的包抄/偷袭——
        // 总指挥指令单 #1 阶段二）；其余角色冲锋时全队直突（行为与原设计一致）。
        let mut tactic = pick_tactic(npc.role, &npc.perception);
        let is_flank_maneuver = matches!(tactic, Tactic::Flank | Tactic::Ambush);
        if ctx.charge
            && npc.role != TacticalRole::Suppressor
            && tactic != Tactic::Retreat
            && !is_flank_maneuver
        {
            tactic = Tactic::Advance;
        }
        // 掩体利用：突击/压制手接近射程边缘时先评估障碍环带掩体（先移动到掩体再推进开火）。
        // 环带内无射程内掩体（如玩家处于中央安全区）时保持原直线推进 → 冒烟站定语义不变；
        // 只在 Chase 态生效且冲锋时不做（冲锋 = 全队直突）。
        // 压力模式（NPC-vs-NPC）：目标在射程内且本 NPC 附近（40m）存在障碍格 → 也进入
        // 掩体利用（互射战场用障碍环带/关卡掩体，总指挥指令单 #2 阶段二）。
        if state == NpcState::Chase
            && !ctx.charge
            && matches!(tactic, Tactic::Advance | Tactic::Suppress)
        {
            // 压力模式：目标在射程附近（≤ attack_range + 40m）即进入掩体利用——
            // advance 沿目标方向找遮挡掩体（NPC 穿越障碍带时自然利用），不要求当前位置附近有障碍。
            let range = if ctx.stress {
                npc.attack_range + COVER_SEEK_RANGE * 2.0
            } else {
                npc.attack_range + COVER_SEEK_RANGE
            };
            if dist <= range {
                tactic = Tactic::CoverSeek;
            }
        }
        // 压力模式 Attack 态：若 NPC 站定于障碍掩体旁（贴掩体探头射击），战术标记为
        // CoverSeek——让互射战场中「利用掩体交火」的 NPC 持续可见（供战术分布采样与观察）。
        // 普通模式行为不变（冒烟依赖 Attack 站定日志与纯 Advance/Suppress 语义）。
        if ctx.stress && state == NpcState::Attack {
            let npc_g = world_to_grid(npc.position[0], npc.position[2]);
            if !crate::engine::ai::find_cover_points(ctx.grid, npc_g, COVER_MAX_DIST).is_empty() {
                tactic = Tactic::CoverSeek;
            }
        }
        npc.tactic = tactic;
        let (bx, bz) = (npc.position[0], npc.position[2]);
        advance_npc(
            npc,
            state,
            tactic,
            &target_pos,
            target_yaw,
            ctx.grid,
            ctx.ring_inner,
            ctx.ring_outer,
            ctx.obstacles,
            ctx.time,
            ctx.dt,
            ctx.stress,
        );
        // 朝向更新：移动时朝移动方向；站定时面向目标（渲染士兵模型用）
        let mdx = npc.position[0] - bx;
        let mdz = npc.position[2] - bz;
        if mdx * mdx + mdz * mdz > 1e-6 {
            npc.facing = mdz.atan2(mdx);
        } else {
            npc.facing = (npc.position[2] - target_pos.z).atan2(npc.position[0] - target_pos.x);
        }
        // 攻击态站定：打位置日志，冒烟 harness 读日志后从对跖点瞄准点射
        if state == NpcState::Attack && prev != NpcState::Attack {
            log::info!(
                "npc: #{} stand ({:.1}, {:.1}, {:.1})",
                npc.id,
                npc.position[0],
                npc.position[1],
                npc.position[2]
            );
        }
    }

    /// 串行推进全部 NPC（普通波次路径；与并行路径逐 NPC 行为一致）
    fn step_ai_serial(npcs: &mut [Npc], ctx: &AiStepCtx) {
        for (i, npc) in npcs.iter_mut().enumerate() {
            Self::step_npc(i, npc, ctx);
        }
    }

    /// 双池并行推进全部 NPC（线程优化第 2 步，2026-08-11）：
    /// 数组已由 `partition_ai_tiers` 稳定重排为 [Near..., Far...]，`near_len` 为分界。
    /// - 近组（Near）：延迟敏感 → `cpu::scene_pool()`（AMD CCD0 / Intel 仅 P-core），
    ///   调用线程参与首段，与主线程同簇通信延迟最低；
    /// - 远组（Far）：延迟不敏感重计算 → `cpu::ai_pool()`（AMD CCD1 / Intel E-core）。
    /// 各 NPC 更新彼此独立（目标/感知/路径均为本帧快照），并行与串行结果逐位一致。
    fn step_ai_parallel(npcs: &mut [Npc], near_len: usize, ctx: &AiStepCtx) {
        let (near, far) = npcs.split_at_mut(near_len);
        let near_pool = crate::engine::cpu::scene_pool();
        near_pool.par_for_each_mut(near, |_, start, slice| {
            for (k, npc) in slice.iter_mut().enumerate() {
                Self::step_npc(start + k, npc, ctx);
            }
        });
        let far_pool = crate::engine::cpu::ai_pool();
        far_pool.par_for_each_mut(far, |_, start, slice| {
            for (k, npc) in slice.iter_mut().enumerate() {
                // 远组降频（压力模式）：无感知/非交互远 NPC 按 id 分帧跳过，
                // 交互中（攻击/感知/受击/被瞄准）恒每帧步进。
                if ctx.decimate_far && should_decimate_far(npc, ctx.frame) {
                    continue;
                }
                Self::step_npc(near_len + start + k, npc, ctx);
            }
        });
    }

    /// 冲击波压力场 SIMD 实测（默认关，见 RV3D_EXPLOSION_SIM）：
    /// 爆心沿确定性圆周轨迹扫掠，64×64=4096 采样点波前每帧推进一次；
    /// 每秒输出一次指令集加速比基准突发（65536 点 × 32 轮：单帧 4096 点太小，
    /// 时钟噪声会淹没真实差距；突发取平均才可测出 AVX-512/AVX2 的浮点收益）。
    fn step_explosion_sim(&mut self) {
        if self.shock_points.is_empty() {
            // 64×64 采样网格覆盖 512m 场地（-256..256，与实例场同域）
            let n = 64usize;
            let step = 512.0 / n as f32;
            for iz in 0..n {
                for ix in 0..n {
                    let x = (ix as f32 - (n as f32 - 1.0) * 0.5) * step;
                    let z = (iz as f32 - (n as f32 - 1.0) * 0.5) * step;
                    self.shock_points.push([x, 1.0, z]);
                }
            }
            self.shock_out = vec![0.0f32; self.shock_points.len()];
        }
        if self.bench_points.is_empty() {
            // 256×256=65536 采样点：与实例场同密度，覆盖 512m 场地
            let n = 256usize;
            let step = 512.0 / n as f32;
            for iz in 0..n {
                for ix in 0..n {
                    let x = (ix as f32 - (n as f32 - 1.0) * 0.5) * step;
                    let z = (iz as f32 - (n as f32 - 1.0) * 0.5) * step;
                    self.bench_points.push([x, 1.0, z]);
                }
            }
        }
        // 爆心：确定性圆周扫掠（不依赖玩家输入，基准可复现）
        let center = [self.time.sin() * 40.0, 1.0, self.time.cos() * 40.0];
        let t0 = std::time::Instant::now();
        self.explosion_path = crate::engine::simd::shockwave_pressure(
            center,
            60.0,
            1000.0,
            &self.shock_points,
            &mut self.shock_out,
        );
        self.stage_explosion_us = t0.elapsed().as_micros() as u64;
        if self.time - self.last_explosion_log >= 1.0 {
            self.last_explosion_log = self.time;
            // 基准突发：65536 点 × 32 轮，选路 vs 标量各跑一遍取平均
            let rounds = 32u32;
            let n = self.bench_points.len();
            let mut simd_out = vec![0.0f32; n];
            let mut scalar_out = vec![0.0f32; n];
            let t1 = std::time::Instant::now();
            for _ in 0..rounds {
                self.explosion_path = crate::engine::simd::shockwave_pressure(
                    center,
                    60.0,
                    1000.0,
                    &self.bench_points,
                    &mut simd_out,
                );
            }
            let simd_us = (t1.elapsed().as_micros() as u64) / rounds as u64;
            let t2 = std::time::Instant::now();
            for _ in 0..rounds {
                crate::engine::simd::shockwave_pressure_scalar(
                    center,
                    60.0,
                    1000.0,
                    &self.bench_points,
                    &mut scalar_out,
                );
            }
            let scalar_us = (t2.elapsed().as_micros() as u64) / rounds as u64;
            let eq = simd_out == scalar_out;
            let speedup = scalar_us as f64 / simd_us.max(1) as f64;
            log::info!(
                "simd: path={} bench_points={} rounds={} simd_us={} scalar_us={} speedup={:.2}x bitwise_eq={}",
                self.explosion_path,
                n,
                rounds,
                simd_us,
                scalar_us,
                speedup,
                eq
            );
        }
    }

    /// 推进 NPC：感知 → 状态机 → 战术决策 → A* 路径 → 移动 → 地形高度
    ///
    /// 战术层（见 ai.rs）：
    /// - 角色分工：每波确定性分配 突击/包抄/压制/掩体跃进，左右包抄按 id 奇偶分工
    /// - 进攻协同：Chase/Attack 过半 → 同步冲锋（压制手除外，保持压制）
    /// - 躲避攻击：移动态受击/被火力威胁 → 侧向弹开（Attack 站定是冒烟瞄准依据，不躲）
    /// - 偷袭绕路：包抄手在玩家未面朝时绕大圈逼近，被发现转侧翼
    fn update_ai(&mut self, dt: f32, camera: &Camera) {
        let player = camera.position();
        let grid = self.grid.clone();
        let time = self.time;
        let player_yaw = camera.yaw;
        // 分层调度（2026-08-11）：稳定重排 npcs 为 [Near..., Far...]，
        // 返回 Near 段长度。Near = 与玩家实时交互/近距离（延迟敏感，走 scene_pool
        // = P 核/CCD0），Far = 远距离重计算（走 ai_pool = CCD1/E 核）。
        // 重排后 under_fire / targets 均在当前数组顺序上构建，帧内索引对齐；
        // 各 NPC 步进彼此独立（AiStepCtx 只读），重排不改变步进语义。
        let stress = self.stress;
        let tier_params = AiTierParams::default();
        let near_len = partition_ai_tiers(&mut self.npcs, |npc| {
            ai_tier_of(npc, &player, stress, &tier_params)
        });
        // 同步冲锋判定：本帧开始时 Chase/Attack 数量过半 → 全队突进
        let active = self
            .npcs
            .iter()
            .filter(|n| {
                matches!(
                    n.state_machine.state(),
                    NpcState::Chase | NpcState::Attack
                )
            })
            .count() as u32;
        let charge = should_charge(active, self.npcs.len() as u32, self.charge_active);
        self.charge_active = charge;
        // 弹道威胁预扫：存活子弹水平距离 < THREAT_RADIUS 且朝 NPC 方向飞行 → 该 NPC 受火力威胁
        let under_fire = {
            let mut flags = vec![false; self.npcs.len()];
            for p in &self.projectiles {
                if !p.is_alive() {
                    continue;
                }
                let v = p.velocity();
                for (i, npc) in self.npcs.iter().enumerate() {
                    if flags[i] {
                        continue;
                    }
                    let dx = npc.position[0] - p.position[0];
                    let dz = npc.position[2] - p.position[2];
                    if dx * dx + dz * dz > THREAT_RADIUS * THREAT_RADIUS {
                        continue;
                    }
                    if dx * v[0] + dz * v[2] > 0.0 {
                        flags[i] = true;
                    }
                }
            }
            flags
        };
        // 压力模式：每 NPC 预选最近敌对目标（敌对 NPC 优先、玩家兜底；O(n²) 纯读，串行）
        let targets: Vec<Option<(usize, [f32; 3], f32)>> = if self.stress {
            pick_stress_targets(&self.npcs, STRESS_SIGHT)
        } else {
            Vec::new()
        };
        // 掩体利用评估用的当前关卡障碍环带（theme 随关卡轮换）
        let theme = theme_for_level(self.level);
        {
            let ctx = AiStepCtx {
                player: &player,
                player_yaw,
                charge,
                under_fire: &under_fire,
                targets: &targets,
                grid: &grid,
                time,
                dt,
                stress: self.stress,
                frame: self.frame_no,
                decimate_far: self.stress
                    && std::env::var("RV3D_AI_DECIMATE")
                        .map_or(true, |v| v != "off" && v != "0"),
                ring_inner: theme.ring_inner,
                ring_outer: theme.ring_outer,
                obstacles: &self.map.obstacles,
            };
            if self.npcs.len() >= PARALLEL_AI_MIN && self.ai_parallel {
                // safety: 已分层（Near 在前），双池分片互不相交；ctx 只含共享只读数据
                Self::step_ai_parallel(&mut self.npcs, near_len, &ctx);
            } else {
                Self::step_ai_serial(&mut self.npcs, &ctx);
            }
        }
        // NPC 投掷手榴弹：压力模式 / survive 防守波次中，交火 NPC 低概率（5-8%）投掷
        // （仅对敌对目标方向；阵营区分不炸友军）。普通波次（打玩家）不投掷 → 行为零回归。
        if self.stress || self.is_survive_rule() {
            self.npc_throw_grenades(dt, &targets);
        }
        // 压力模式：攻击态 NPC 对目标 NPC 结算伤害（每满 1 秒 dps；玩家无敌旁观）
        if self.stress {
            self.apply_npc_combat(dt, &targets);
        }
        // 压力模式：移除阵亡 NPC；任一阵营团灭 → 全量补员开新一轮
        if self.stress {
            self.update_stress_respawns(&player);
        }
        if self.time - self.ai_log_time >= 1.0 {
            self.ai_log_time = self.time;
            let mut counts = [0u32; 4];
            let mut tactics = [0u32; 8];
            for npc in &self.npcs {
                counts[npc.state_machine.state() as usize] += 1;
                tactics[npc.tactic as usize] += 1;
            }
            log::info!(
                "ai: npcs={} near={} far={} idle={} patrol={} chase={} attack={} tactics={:?}",
                self.npcs.len(),
                near_len,
                self.npcs.len() - near_len,
                counts[0],
                counts[1],
                counts[2],
                counts[3],
                tactics
            );
        }
        // 攻击态 NPC 对玩家造成伤害（1 秒一次），驱动 HUD 血条
        // 伤害值取当前有效波次的 dps（Boss 波更高，见 wave_profile）
        let dps = wave_profile(self.effective_wave(self.wave)).dps;
        if !self.stress && self.time - self.last_damage_time >= 1.0
            && self.game_state == GameState::Playing
            && self
                .npcs
                .iter()
                .any(|n| n.state_machine.state() == NpcState::Attack)
            && self.hud.health > 0.0
        {
            self.hud.health = (self.hud.health - dps).max(0.0);
            self.last_damage_time = self.time;
            if self.hud.health <= 0.0 {
                // 击杀提示：玩家被敌方击杀
                self.hud.push_kill("YOU WERE KILLED".to_string());
                // survive 规则：玩家死亡即失败（Defeat 结算）；否则普通 GameOver
                if self.is_survive_rule() {
                    if let Some(obj) = self.obj_state.as_mut() {
                        obj.won_team = Some(crate::engine::ai::Team::Red);
                    }
                    self.game_state = GameState::Defeat;
                    log::info!(
                        "survive: 玩家阵亡于第 {} 波 → 失败",
                        self.wave
                    );
                } else {
                    self.game_state = GameState::GameOver;
                    log::info!(
                        "game: player down, score={} wave={} (GameOver: gameplay frozen, projectiles coast without kills)",
                        self.score,
                        self.wave
                    );
                }
            }
        }
    }

    /// 压力模式 NPC 互射：攻击态且目标在攻击距离内 → 每满 1 秒对目标结算 dps。
    /// 目标索引在帧内有效（互射结算后统一移除，不在中途删）。友军永远不被伤害。
    fn apply_npc_combat(&mut self, dt: f32, targets: &[Option<(usize, [f32; 3], f32)>]) {
        if self.npcs.len() < 2 {
            return;
        }
        let dps = wave_profile(self.effective_wave(self.wave)).dps;
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for (i, npc) in self.npcs.iter().enumerate() {
            if npc.state_machine.state() != NpcState::Attack {
                continue;
            }
            if let Some(Some((t, _, _))) = targets.get(i) {
                if *t >= self.npcs.len() {
                    continue;
                }
                let dx = self.npcs[*t].position[0] - npc.position[0];
                let dz = self.npcs[*t].position[2] - npc.position[2];
                if dx * dx + dz * dz <= npc.attack_range * npc.attack_range {
                    pairs.push((i, *t));
                }
            }
        }
        for (i, t) in pairs {
            self.npcs[i].fire_accum += dt;
            if self.npcs[i].fire_accum >= 1.0 {
                self.npcs[i].fire_accum = 0.0;
                self.npcs[t].hp -= dps;
                // 击杀提示：NPC 互射击杀（attacker team killed victim team + id）
                if self.npcs[t].hp <= 0.0 && self.npcs[t].hp > -dps {
                    let (a, v) = (self.npcs[i].team, self.npcs[t].team);
                    let vid = self.npcs[t].id;
                    self.hud.push_kill(format!("{} KILLED {} #{}", team_name(a), team_name(v), vid));
                }
            }
        }
    }

    /// 压力模式减员与补员：移除阵亡 NPC；任一阵营团灭 → 全量补员开新一轮。
    fn update_stress_respawns(&mut self, player: &glam::Vec3) {
        // 菜单态不结算（初始 NPC 全为红方，会误判蓝方团灭提前开战）
        if self.game_state == GameState::StartMenu {
            return;
        }
        let before = self.npcs.len();
        self.npcs.retain(|n| n.hp > 0.0);
        let red = self.npcs.iter().filter(|n| n.team == Team::Red).count();
        let blue = self.npcs.len() - red;
        if self.npcs.len() != before {
            // 任务目标：本轮歼灭数推进（达成 → 胜利横幅/日志；补员逻辑不受影响）
            if self.objective.progress((before - self.npcs.len()) as u32) {
                self.on_objective_complete();
            }
            log::info!(
                "battle: 阵亡 {}（红={} 蓝={} 存活）",
                before - self.npcs.len(),
                red,
                blue
            );
        }
        if red == 0 || blue == 0 {
            self.stress_round += 1;
            log::info!(
                "battle: 第 {} 轮结束（红={} 蓝={}），全量补员开新轮",
                self.stress_round - 1,
                red,
                blue
            );
            self.spawn_stress_battle(player);
        }
    }

    /// 累计碰撞事件数（供 UI / 日志）
    pub fn total_collisions(&self) -> u64 {
        self.total_collisions
    }

    /// 取走本帧碰撞事件并累计计数（限频打一条日志）
    fn drain_collisions(&mut self) {
        if let Ok(mut buf) = self.event_buf.lock() {
            self.collisions = std::mem::take(&mut *buf);
        }
        if self.collisions.is_empty() {
            return;
        }
        self.total_collisions += self.collisions.len() as u64;
        if self.time - self.last_event_log_time >= 0.5 {
            self.last_event_log_time = self.time;
            let kinds: Vec<String> = self
                .collisions
                .iter()
                .map(|e| format!("{:?}", e.kind))
                .collect();
            log::info!("physics: {} events this frame ({})", self.collisions.len(), kinds.join(", "));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_tier_classify_boundaries() {
        let p = AiTierParams::default();
        // 交互中即使超远也 Near（每帧步进，不降频）
        assert_eq!(classify_ai_tier(1.0e9, true, &p), AiTier::Near);
        // 距离 ≤ 阈值 → Near（含边界相等）
        assert_eq!(classify_ai_tier(0.0, false, &p), AiTier::Near);
        assert_eq!(
            classify_ai_tier(p.near_radius * p.near_radius, false, &p),
            AiTier::Near
        );
        // 超远且不交互 → Far
        assert_eq!(
            classify_ai_tier(p.near_radius * p.near_radius + 1.0, false, &p),
            AiTier::Far
        );
    }

    #[test]
    fn ai_tier_partition_stable_and_split() {
        // (id, 期望档位) 乱序输入；分区后 Near 段在前、组内相对顺序保持
        let mut items = vec![
            (3u32, AiTier::Far),
            (0, AiTier::Near),
            (4, AiTier::Far),
            (1, AiTier::Near),
            (2, AiTier::Near),
        ];
        let near_len = partition_ai_tiers(&mut items, |(_, t)| *t);
        assert_eq!(near_len, 3);
        let (near, far) = items.split_at(near_len);
        assert!(near.iter().all(|(_, t)| *t == AiTier::Near));
        assert!(far.iter().all(|(_, t)| *t == AiTier::Far));
        // 稳定分区：Near 原顺序 0,1,2；Far 原顺序 3,4
        assert_eq!(
            near.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(
            far.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![3, 4]
        );
    }

    #[test]
    fn ai_tier_partition_empty_and_uniform() {
        let mut empty: Vec<u32> = Vec::new();
        assert_eq!(partition_ai_tiers(&mut empty, |_| AiTier::Near), 0);
        let mut all_near = vec![7u32, 8, 9];
        assert_eq!(partition_ai_tiers(&mut all_near, |_| AiTier::Near), 3);
        assert_eq!(all_near, vec![7, 8, 9]); // 稳定：全 Near 原序不变
        let mut all_far = vec![7u32, 8, 9];
        assert_eq!(partition_ai_tiers(&mut all_far, |_| AiTier::Far), 0);
        assert_eq!(all_far, vec![7, 8, 9]);
    }

    #[test]
    fn ai_tier_partition_with_real_npc_tier() {
        // 真实 Npc：按到玩家距离平方 + 交互标志分层
        let params = AiTierParams::default();
        let mut npcs = vec![
            npc_at(0, Team::Red, [10.0, 0.0, 0.0]),  // 近 → Near
            npc_at(1, Team::Red, [500.0, 0.0, 0.0]), // 远 → Far
            npc_at(2, Team::Red, [300.0, 0.0, 0.0]), // 远 → Far
            npc_at(3, Team::Red, [30.0, 0.0, 0.0]),  // 近 → Near
        ];
        let player = [0.0f32, 0.0, 0.0];
        let near_len = partition_ai_tiers(&mut npcs, |n| {
            let dx = n.position[0] - player[0];
            let dz = n.position[2] - player[2];
            classify_ai_tier(dx * dx + dz * dz, false, &params)
        });
        assert_eq!(near_len, 2);
        assert_eq!(npcs[0].id, 0);
        assert_eq!(npcs[1].id, 3);
        assert_eq!(npcs[2].id, 1);
        assert_eq!(npcs[3].id, 2);
    }

    #[test]
    fn far_decimate_skips_idle_npcs_by_frame() {
        // 远组无感知非攻击 NPC（600m > STRESS_SIGHT=512 → 感知不到任何目标）：
        // decimate_far=true 时按 id 分帧跳过（id=7 → 7%4=3），
        // 命中帧步进（位置变化）、跳过帧冻结（位置不变）。
        let mut npcs = [npc_at(7, Team::Red, [600.0, 0.0, 0.0])];
        let game = Game::new();
        let grid = game.grid.clone();
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        let flags = vec![false; 1];
        let targets = vec![None];
        for frame in 0..8u32 {
            let ctx = AiStepCtx {
                player: &player,
                player_yaw: 0.0,
                charge: false,
                under_fire: &flags,
                targets: &targets,
                grid: &grid,
                time: 1.0 + frame as f32 / 60.0,
                dt: 1.0 / 60.0,
                stress: true,
                frame,
                decimate_far: true,
                ring_inner: MAP_RING_INNER,
                ring_outer: MAP_RING_OUTER,
                obstacles: &game.map.obstacles,
            };
            let idle_before = npcs[0].position;
            Game::step_ai_parallel(&mut npcs, 0, &ctx); // near_len=0 → 全远组
            if frame % AI_FAR_DECIMATE == 7 % AI_FAR_DECIMATE {
                assert_ne!(npcs[0].position, idle_before, "id=7 命中帧应步进（frame {frame}）");
            } else {
                assert_eq!(npcs[0].position, idle_before, "id=7 跳过帧应冻结（frame {frame}）");
            }
        }
    }

    #[test]
    fn should_decimate_far_excludes_interactions() {
        // 交互中（感知/受击/被火力威胁/被瞄准/攻击态）永不降频；无交互按 id 分帧
        let mut n = npc_at(7, Team::Red, [300.0, 0.0, 0.0]);
        n.perception.enemy_visible = true;
        assert!(!should_decimate_far(&n, 0), "感知敌人不降频");
        n.perception.enemy_visible = false;
        n.perception.took_hit = true;
        assert!(!should_decimate_far(&n, 0), "受击不降频");
        n.perception.took_hit = false;
        n.perception.under_fire = true;
        assert!(!should_decimate_far(&n, 0), "被火力威胁不降频");
        n.perception.under_fire = false;
        n.perception.player_aiming = true;
        assert!(!should_decimate_far(&n, 0), "被玩家瞄准不降频");
        n.perception.player_aiming = false;
        // 推进状态机到 Attack（同 stress_npc_combat 的确定性推进）
        let p = NpcPerception {
            enemy_visible: true,
            enemy_in_range: true,
            ..NpcPerception::default()
        };
        n.state_machine.update(p);
        n.state_machine.update(p);
        assert_eq!(n.state_machine.state(), NpcState::Attack);
        assert!(!should_decimate_far(&n, 0), "攻击态不降频");
        // 无交互：id=7 → 7%4=3；frame 0/2 降频、frame 3 命中
        n.state_machine = NpcStateMachine::new();
        assert!(should_decimate_far(&n, 0));
        assert!(should_decimate_far(&n, 2));
        assert!(!should_decimate_far(&n, 3));
    }

    /// 关卡障碍刚体应贴地落地并静止（程序化地图替代原 3 AABB + 2 球体演示场景）
    #[test]
    fn map_obstacles_ground_and_settle() {
        let mut game = Game::new();
        assert!(!game.world.bodies.is_empty(), "level 1 map should populate physics");
        assert!(game.world.spheres.is_empty(), "procedural map uses AABB walls only");
        for _ in 0..120 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        for body in &game.world.bodies {
            assert!(body.grounded, "body should be grounded");
            let bottom = body.position.y - body.half_extents.y;
            assert!((bottom - game.world.ground_y).abs() < 0.01, "body should rest on ground");
        }
    }

    /// 程序化地图：同种子确定、异种子不同、障碍都在安全环外且两两不重叠
    #[test]
    fn map_generation_is_deterministic_and_safe() {
        let a = generate_level_map(1);
        let b = generate_level_map(1);
        let c = generate_level_map(2);
        assert_eq!(a.obstacles, b.obstacles, "same seed must produce identical layout");
        assert_ne!(a.obstacles, c.obstacles, "different seed must produce different layout");
        assert!(!a.obstacles.is_empty() && !c.obstacles.is_empty());
        for ob in a.obstacles.iter().chain(c.obstacles.iter()) {
            let d = (ob.x * ob.x + ob.z * ob.z).sqrt();
            assert!(d >= MAP_RING_INNER - 0.5, "obstacle too close to origin: {:.1}m", d);
        }
        for (i, o1) in a.obstacles.iter().enumerate() {
            for o2 in a.obstacles.iter().skip(i + 1) {
                let overlap = (o1.x - o2.x).abs() < o1.half_w + o2.half_w
                    && (o1.z - o2.z).abs() < o1.half_d + o2.half_d;
                assert!(!overlap, "obstacles must not overlap");
            }
        }
    }

    /// 地图主题：第一关保持冒烟基准（58m 安全环 + Wall 种类 + 与主题化生成完全一致）
    #[test]
    fn map_theme_level1_preserves_smoke_ring() {
        let t1 = theme_for_level(1);
        assert_eq!(t1.ring_inner, MAP_RING_INNER, "第一关安全环必须 58m");
        assert_eq!(t1.kind, ObstacleKind::Wall);
        let a = generate_level_map(1);
        let b = generate_level_map_with_theme(1, theme_for_level(1));
        assert_eq!(a.obstacles, b.obstacles, "generate_level_map 必须等于主题化生成");
        assert!(
            a.obstacles.iter().all(|o| o.kind == ObstacleKind::Wall),
            "第一关障碍全部为 Wall"
        );
        for ob in &a.obstacles {
            let d = (ob.x * ob.x + ob.z * ob.z).sqrt();
            assert!(d >= MAP_RING_INNER - 0.5, "58m 环带内必须无障碍: {:.1}m", d);
        }
    }

    /// 地图主题：按关卡轮换种类/安全环/密度，同主题确定性一致、异主题布局不同
    #[test]
    fn map_themes_rotate_and_differ() {
        // 主题轮换周期：1/4/7 同主题，2/5/8、3/6/9 依次轮换
        assert_eq!(theme_for_level(1), theme_for_level(4));
        assert_ne!(theme_for_level(1), theme_for_level(2));
        assert_ne!(theme_for_level(2), theme_for_level(3));
        assert_ne!(theme_for_level(3), theme_for_level(4));
        assert_eq!(theme_for_level(2).kind, ObstacleKind::Block);
        assert_eq!(theme_for_level(3).kind, ObstacleKind::Barrier);
        // 安全环半径随主题变化，且都不低于 NPC 站定下限（见 MAP_RING_INNER 注释）
        for level in 1..=6 {
            assert!(
                theme_for_level(level).ring_inner >= MAP_RING_INNER - 0.5,
                "level {} 安全环过低",
                level
            );
        }
        // 同 seed 不同主题 → 不同布局；同主题同 seed → 确定性一致
        let wall = generate_level_map_with_theme(1, theme_for_level(1));
        let block = generate_level_map_with_theme(1, theme_for_level(2));
        assert_ne!(wall.obstacles, block.obstacles, "不同主题必须产生不同布局");
        assert_eq!(
            wall.obstacles,
            generate_level_map_with_theme(1, theme_for_level(1)).obstacles
        );
        assert!(wall.obstacles.iter().all(|o| o.kind == ObstacleKind::Wall));
        assert!(block.obstacles.iter().all(|o| o.kind == ObstacleKind::Block));
    }

    /// 升关：每关 WAVES_PER_LEVEL 波清完后 level+1、wave 回 1、地图重新生成、难度按有效波次递进
    #[test]
    fn level_advances_after_waves_per_level() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert_eq!(game.level, 1);
        let l1 = game.map.obstacles.clone();
        // 快速清完 3 波（每波清空 npcs 后推进 3.3s 倒计时）
        for _ in 0..3 {
            game.npcs.clear();
            for _ in 0..330 {
                game.update(0.01, &Camera::new());
            }
        }
        assert_eq!(game.level, 2, "level should advance after WAVES_PER_LEVEL waves");
        assert_eq!(game.wave, 1, "wave resets to 1 on level up");
        assert!(!game.npcs.is_empty(), "level 2 wave 1 should spawn enemies");
        let l2 = game.map.obstacles.clone();
        assert_ne!(l1, l2, "level 2 must regenerate the map layout");
        // 物理世界与网格同步重建
        assert_eq!(game.world.bodies.len(), l2.len());
    }

    /// 清第 WAVES_PER_LEVEL 波之前不应升关（wave 3 是升关临界，wave 2 仍停留原关）
    #[test]
    fn level_does_not_advance_before_last_wave() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.npcs.clear();
        for _ in 0..330 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.level, 1, "wave 1 → 2 must not advance level");
        assert_eq!(game.wave, 2);
    }

    /// 开火产生投射物，命中物理刚体后销毁并计数
    #[test]
    fn weapon_fire_hits_physics_body() {
        let mut game = Game::new();
        for _ in 0..120 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        let body = game.world.bodies[0];
        let target = [body.position.x, body.position.y, body.position.z];
        // 从目标正上方竖直向下射击
        assert!(game.fire([target[0], target[1] + 50.0, target[2]], [0.0, -1.0, 0.0]));
        assert_eq!(game.shots, 1);
        for _ in 0..200 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(game.hits() >= 1, "projectile should hit the body");
        assert!(game.projectiles.is_empty(), "hit projectile should be removed");
    }

    /// 射速冷却：连续开火被限流
    #[test]
    fn weapon_fire_rate_limits_shots() {
        let mut game = Game::new();
        let origin = [0.0, 5.0, 0.0];
        let dir = [0.0, 0.0, -1.0];
        assert!(game.fire(origin, dir));
        // 冷却期内再开火应被拒绝
        for _ in 0..10 {
            assert!(!game.fire(origin, dir));
            game.update(1.0 / 240.0, &Camera::new());
        }
        assert_eq!(game.shots, 1);
    }

    /// HUD：喂入渲染统计后能产出覆盖层 quad（血条 + 调试文本）
    #[test]
    fn hud_quads_produce_overlay() {
        let mut game = Game::new();
        game.hud.fps = 60.0;
        let quads = game.hud_quads(100, 200, "high");
        assert!(!quads.is_empty(), "hud should produce overlay quads");
        assert!(quads.len() >= 3, "health bar + debug text lines expected");
    }

    /// HUD：游戏画面含分数/波次/准星等元素
    #[test]
    fn hud_game_screen_has_score_wave_crosshair() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.hud.fps = 60.0;
        game.score = 30;
        game.wave = 2;
        let quads = game.hud_quads(100, 200, "high");
        assert!(!quads.is_empty());
        assert!(quads.len() >= 5, "health/ammo bars + score/wave + crosshair");
    }

    /// HUD：开始菜单与死亡结算画面都产出元素
    #[test]
    fn hud_menu_and_gameover_screens() {
        let mut game = Game::new();
        let menu = game.hud_quads(0, 0, "high");
        assert!(!menu.is_empty(), "start menu should draw overlay + title");
        game.game_state = GameState::GameOver;
        let over = game.hud_quads(0, 0, "high");
        assert!(!over.is_empty(), "game over screen should draw overlay + score");
    }

    /// 光照场景：开启标志位、方向光与环境光生效
    #[test]
    fn light_uniform_enabled() {
        let game = Game::new();
        let u = game.light_uniform();
        assert!(u.flags.x >= 1.0, "lighting should be enabled");
        assert!(u.directional.direction.w >= 1.0, "directional enabled");
        assert!(u.ambient.w > 0.0, "ambient intensity set");
        assert!(u.points[0].position.w >= 1.0, "point light A enabled");
        assert!(u.points[1].position.w >= 1.0, "point light B enabled");
    }

    /// 音频：环境风合成器常开，tick 每帧渲染（audio_us 链路真实运行）
    #[test]
    fn audio_synth_ambient_runs_with_tick() {
        let mut game = Game::new();
        assert!(
            game.audio.synth().ambient_active(),
            "ambient wind should be playing"
        );
        let cam = Camera::new();
        for _ in 0..60 {
            game.update(1.0 / 60.0, &cam);
        }
        assert!(
            game.audio.synth().ambient_active(),
            "ambient wind should persist"
        );
    }

    /// AI：NPC 站在地形高度上，且相机在原点时能离开 Idle 状态
    #[test]
    fn ai_npcs_stand_on_terrain_and_leave_idle() {
        let mut game = Game::new();
        let cam = Camera::new();
        assert_eq!(game.npcs.len(), 8);
        for _ in 0..120 {
            game.update(1.0 / 60.0, &cam);
        }
        for npc in &game.npcs {
            let h = terrain_height_at(npc.position[0], npc.position[2]);
            assert!(
                (npc.position[1] - h).abs() < 0.001,
                "npc y should match terrain height"
            );
        }
        assert!(
            game.npcs
                .iter()
                .any(|n| n.state_machine.state() != NpcState::Idle),
            "at least one npc should leave Idle near the player"
        );
    }

    /// 网格坐标转换往返一致
    #[test]
    fn grid_conversion_roundtrip() {
        for (x, z) in [(-255.0, -255.0), (0.0, 0.0), (255.0, 255.0), (123.4, -67.8)] {
            let g = world_to_grid(x, z);
            let (wx, wz) = grid_to_world(g);
            assert_eq!(g, world_to_grid(wx, wz));
        }
    }

    /// 初始状态为 StartMenu（开始菜单）
    #[test]
    fn game_state_starts_in_menu() {
        let game = Game::new();
        assert_eq!(game.state(), GameState::StartMenu);
    }

    /// 开始菜单任意键 → Playing，重置并生成第 1 波
    #[test]
    fn start_menu_any_key_begins_run() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert_eq!(game.state(), GameState::Playing);
        assert_eq!(game.wave, 1);
        assert_eq!(game.score, 0);
        assert_eq!(game.hud.health, game.hud.max_health);
        assert_eq!(game.npcs.len(), (4 + 2 * 1).min(24), "wave 1 spawns 6");
    }

    /// 死亡结算 R 重开：状态复位并重新生成第 1 波
    #[test]
    fn gameover_restart_resets_run() {
        let mut game = Game::new();
        game.game_state = GameState::GameOver;
        game.score = 999;
        game.hud.health = 0.0;
        game.request_restart(&glam::Vec3::ZERO);
        assert_eq!(game.state(), GameState::Playing);
        assert_eq!(game.score, 0);
        assert_eq!(game.wave, 1);
        assert_eq!(game.hud.health, game.hud.max_health);
        assert!(!game.npcs.is_empty());
    }

    /// 波次清空（npcs 空）后开始 3 秒倒计时，随后刷出下一波
    #[test]
    fn wave_spawns_after_clear() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert_eq!(game.wave, 1);
        assert_eq!(game.npcs.len(), 6);
        game.npcs.clear();
        game.update(0.01, &Camera::new());
        assert!(game.wave_timer > 0.0, "countdown should start after clear");
        assert_eq!(game.wave, 1, "wave must not advance during countdown");
        // 推进 3 秒以上
        for _ in 0..320 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.wave, 2, "next wave should spawn after countdown");
        assert!(!game.npcs.is_empty(), "wave 2 should spawn enemies");
        assert_eq!(game.npcs.len(), (4 + 2 * 2).min(24));
    }

    /// 波次递进：下一波数量/速度/血量都高于上一波
    #[test]
    fn wave_scales_count_speed_hp() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let (c1, s1, h1) = (game.npcs.len(), game.npcs[0].speed, game.npcs[0].max_hp);
        game.npcs.clear();
        for _ in 0..320 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.wave, 2);
        let (c2, s2, h2) = (game.npcs.len(), game.npcs[0].speed, game.npcs[0].max_hp);
        assert!(c2 > c1, "wave 2 should have more enemies: {} vs {}", c2, c1);
        assert!(s2 > s1, "wave 2 should be faster: {} vs {}", s2, s1);
        assert!(h2 > h1, "wave 2 should have more hp: {} vs {}", h2, h1);
    }

    /// 清空一波奖励分
    #[test]
    fn wave_clear_awards_bonus() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.npcs.clear();
        game.update(0.01, &Camera::new());
        assert_eq!(game.score, WAVE_CLEAR_BONUS, "clearing wave 1 awards bonus");
    }

    /// 残留 NPC 清除：刷新波前清掉旧波存活 NPC，新旧不共存
    #[test]
    fn spawn_wave_purges_leftovers() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let old_ids: Vec<usize> = game.npcs.iter().map(|n| n.id).collect();
        game.spawn_wave(3, &glam::Vec3::ZERO);
        assert_eq!(game.npcs.len(), (4 + 2 * 3).min(24), "wave 3 count");
        assert!(
            game.npcs.iter().all(|n| !old_ids.contains(&n.id)),
            "all old-wave npcs must be purged before the new wave"
        );
    }

    /// 投射物命中 NPC 扣血（一次命中 25 伤害）
    #[test]
    fn projectile_damages_npc() {
        let mut game = Game::new();
        let npc_pos = game.npcs[0].position;
        let hp_before = game.npcs[0].hp;
        assert!(game.fire([npc_pos[0], npc_pos[1] + 2.0, npc_pos[2]], [0.0, -1.0, 0.0]));
        for _ in 0..3 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(game.npcs[0].hp, hp_before - 25.0, "one hit should deal weapon damage");
        assert_eq!(game.npcs.len(), 8, "non-lethal hit keeps the npc");
    }

    /// 击杀：hp≤0 移除 NPC 并计分
    #[test]
    fn projectile_kill_scores_and_removes_npc() {
        let mut game = Game::new();
        game.npcs[0].hp = 20.0;
        let npc_pos = game.npcs[0].position;
        assert!(game.fire([npc_pos[0], npc_pos[1] + 2.0, npc_pos[2]], [0.0, -1.0, 0.0]));
        for _ in 0..3 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(game.npcs.len(), 7, "killed npc should be removed");
        assert_eq!(game.score, KILL_SCORE);
    }

    /// 玩家受伤：攻击态 NPC 每秒扣血，血量为 0 进入 GameOver
    #[test]
    fn player_damage_and_gameover() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        // 把一只 NPC 放到玩家（原点）脚边，保证进入 Attack
        game.npcs[0].position = [1.0, 0.0, 1.0];
        game.hud.health = 6.0;
        let cam = Camera::new();
        for _ in 0..300 {
            game.update(1.0 / 60.0, &cam);
            if game.state() == GameState::GameOver {
                break;
            }
        }
        assert_eq!(game.state(), GameState::GameOver, "health 0 should end the run");
        assert_eq!(game.hud.health, 0.0);
    }

    /// GameOver 冻结：投射物继续飞行但不产生新击杀、不计分
    #[test]
    fn gameover_freezes_kills() {
        let mut game = Game::new();
        game.game_state = GameState::GameOver;
        game.npcs[0].hp = 20.0;
        let npc_pos = game.npcs[0].position;
        assert!(game.fire([npc_pos[0], npc_pos[1] + 2.0, npc_pos[2]], [0.0, -1.0, 0.0]));
        for _ in 0..10 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(game.npcs.len(), 8, "no kills allowed after game over");
        assert_eq!(game.score, 0, "no score after game over");
        assert_eq!(game.hud.health, 100.0, "player health locked after game over");
    }

    /// 网络环回演示：init 能绑定环回 server，client 的 Join 能被 server 收到并回 ack
    #[test]
    fn net_loopback_demo_join_roundtrip() {
        let mut demo = Game::init_network_demo().expect("loopback demo should init");
        let mut got_join = false;
        // UDP 环回投递可能有毫秒级延迟：带超时轮询（与 net.rs recv_until 同款模式），避免偶发失败
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
        while !got_join && std::time::Instant::now() < deadline {
            if let Ok(Some((msg, from))) = demo.server.recv() {
                if let NetworkMessage::Join { player_id, .. } = &msg {
                    got_join = *player_id == 0;
                    assert!(demo.server.handle_join(from, "local".into()).is_ok());
                }
            }
            if !got_join {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        assert!(got_join, "server should receive the Join sent at init");
    }

    /// 网络对战闭环（RV3D_NET=server|client 的纯逻辑等价物，无 Vulkan/winit）：
    /// 客户端上报输入 → 服务端应用 → 广播快照 → 客户端插值缓冲
    #[test]
    fn net_server_client_loopback_closed_loop() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut server_game = Game::new();
        let mut client_game = Game::new();
        server_game.set_net_server(server);
        client_game.set_net_client(Client::connect(addr).unwrap());
        client_game.set_movement(true, false, false, false);
        let camera = Camera::new();
        // UDP 环回投递可能有毫秒级延迟：多轮推进让 握手 → 输入 → 快照 完整走通
        for _ in 0..20 {
            client_game.update(1.0 / 60.0, &camera);
            server_game.update(1.0 / 60.0, &camera);
        }
        // 最后一轮收尾：轮询直到追平服务端最新快照（UDP 环回投递有毫秒级延迟）
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
        while client_game.net_client.as_ref().unwrap().snapshot_seq()
            != server_game.net_snap_seq
            && std::time::Instant::now() < deadline
        {
            client_game.update(1.0 / 60.0, &camera);
        }
        let client = client_game.net_client.as_ref().unwrap();
        assert_eq!(client.player_id(), Some(1), "握手应分配 player id 1");
        assert!(client.snapshot_seq() > 0, "应收到服务端快照");
        assert!(client.own_state().is_some(), "快照应含本机权威状态");
        assert!(
            client.entities().len() >= server_game.npcs.len(),
            "快照应包含全部 NPC + 服务端本机玩家"
        );
        assert!(!client.snapshot_timeout(), "回环测试不应超时");
        assert!(server_game.move_forward, "服务端应应用客户端输入");
        assert_eq!(
            client.snapshot_seq(),
            server_game.net_snap_seq,
            "客户端应追平最新快照"
        );
    }

    /// 目标状态回环：服务端广播 ObjectiveState → 客户端解析据点归属/进度
    #[test]
    fn net_objective_state_loopback_broadcast_consumed() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut server_game = Game::new();
        let mut client_game = Game::new();
        server_game.set_net_server(server);
        client_game.set_net_client(Client::connect(addr).unwrap());
        // 给服务端注入一个据点（模拟关卡系统启用）：
        // 用 CapturePoint 直接塞进 obj_state（绕过 RV3D_MAP 加载，纯逻辑回环）
        let rule = crate::engine::objective::GameRule::CapturePoints { required: 1 };
        let mut obj = crate::engine::objective::ObjectiveState::new(rule);
        obj.points.push(crate::engine::objective::CapturePoint::new(
            "A", 0.0, 0.0, 5.0, 10.0,
        ));
        server_game.obj_state = Some(obj);
        let camera = Camera::new();
        for _ in 0..20 {
            client_game.update(1.0 / 60.0, &camera);
            server_game.update(1.0 / 60.0, &camera);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
        while !client_game
            .net_client
            .as_ref()
            .unwrap()
            .has_objective()
            && std::time::Instant::now() < deadline
        {
            client_game.update(1.0 / 60.0, &camera);
        }
        let client = client_game.net_client.as_ref().unwrap();
        assert!(client.has_objective(), "客户端应收到目标状态");
        assert_eq!(client.objective_rule(), "capture");
        let pts = client.objective_state();
        assert_eq!(pts.len(), 1, "应收到 1 个据点");
        assert_eq!(pts[0].0, "A");
        assert_eq!(pts[0].1, 0, "中立据点归属码 = 0");
        assert_eq!(pts[0].2, 0.0, "中立据点进度 = 0");
    }

    /// FPS 玩家：WASD 移动改变位置，眼睛高度 1.6m
    #[test]
    fn fps_player_moves_with_wasd() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let cam = Camera::new(); // yaw=0 → forward -Z
        let start_z = game.player_body.pos.z;
        game.set_movement(true, false, false, false);
        for _ in 0..60 {
            game.update(1.0 / 60.0, &cam);
        }
        assert!(
            game.player_pos().z < start_z - 5.0,
            "W 应沿 -Z 移动约 6m: {} -> {}",
            start_z,
            game.player_pos().z
        );
        assert!(
            (game.player_eye().y - 1.6).abs() < 1e-5,
            "眼睛高度应为 1.6m"
        );
        // S 后退回原点附近
        game.set_movement(false, true, false, false);
        for _ in 0..60 {
            game.update(1.0 / 60.0, &cam);
        }
        assert!(
            game.player_body.pos.z > start_z - 0.5,
            "S 应退回原点附近: {}",
            game.player_body.pos.z
        );
    }

    /// FPS 玩家：撞到演示刚体被推回，不会穿模
    #[test]
    fn fps_player_collides_with_map_obstacle() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        // 取关卡地图第一个障碍盒（AABB 中心 (x,z) 半宽 half_w/half_d）；
        // 玩家放其 +Z 侧 5m，W 前进（-Z 方向）应被挡在 0.5m（玩家半径）外
        let ob = game.map.obstacles[0];
        game.player_body.pos = Pv::new(ob.x, 0.0, ob.z + ob.half_d + 5.0);
        let cam = Camera::new();
        game.set_movement(true, false, false, false);
        for _ in 0..120 {
            game.update(1.0 / 60.0, &cam);
        }
        let z = game.player_body.pos.z;
        assert!(z < ob.z + ob.half_d + 5.0, "玩家应朝障碍移动: {}", z);
        assert!(
            z > ob.z + ob.half_d + 0.3 && z < ob.z + ob.half_d + 0.7,
            "碰撞应把玩家挡在障碍 +Z 面外约 0.5m (期望 ~{}): {}",
            ob.z + ob.half_d + 0.5,
            z
        );
    }

    /// 换弹：R 触发后计时完成，弹匣补满
    #[test]
    fn firearm_reload_cycle_via_game() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let origin = [0.0, 5.0, 0.0];
        let dir = [0.0, 0.0, -1.0];
        let mut fired = 0;
        for _ in 0..120 {
            if game.fire(origin, dir) {
                fired += 1;
            }
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(fired >= 5, "2 秒内应打出至少 5 发: {}", fired);
        let before = game.weapons.active_firearm_ref().magazine();
        assert!(before < 30, "弹匣应消耗过");
        game.request_reload();
        assert!(game.weapons.active_firearm_ref().is_reloading(), "R 应开始换弹");
        for _ in 0..200 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(!game.weapons.active_firearm_ref().is_reloading(), "换弹应完成");
        assert_eq!(game.weapons.active_firearm_ref().magazine(), 30, "换弹后弹匣应补满");
    }

    /// 开火产生后坐力，drain 一次后清零
    #[test]
    fn fire_applies_recoil_kick() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert!(game.fire([0.0, 5.0, 0.0], [0.0, 0.0, -1.0]));
        let (pitch_kick, _) = game.drain_kick();
        assert!(pitch_kick > 0.0, "上跳后坐力应为正");
        assert_eq!(game.drain_kick(), (0.0, 0.0), "drain 后应清零");
    }

    /// 命中 NPC 触发命中标记
    #[test]
    fn projectile_hit_shows_hit_marker() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let npc_pos = game.npcs[0].position;
        assert!(game.fire(
            [npc_pos[0], npc_pos[1] + 2.0, npc_pos[2]],
            [0.0, -1.0, 0.0]
        ));
        for _ in 0..3 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(
            game.hud.hit_marker_timer > 0.0,
            "命中应触发准星命中标记"
        );
    }

    /// 波次难度曲线：spawn 数量/速度/血量/攻击距离与 wave_profile 一致
    #[test]
    fn wave_profile_drives_spawn() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let p = wave_profile(1);
        assert_eq!(game.npcs.len(), p.count as usize, "波次数量");
        for npc in &game.npcs {
            assert!((npc.speed - p.speed).abs() < 1e-6, "速度");
            assert!((npc.max_hp - p.hp).abs() < 1e-6, "血量");
            assert!((npc.attack_range - p.attack_range).abs() < 1e-6, "攻击距离");
        }
    }

    /// 特殊波次：Boss 波最后一只为主怪（高血量/慢速/攻击距离略长），其余仍按 profile
    #[test]
    fn boss_wave_spawns_slow_tanky_elite() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.spawn_wave(5, &glam::Vec3::ZERO);
        let p = wave_profile(5);
        assert_eq!(p.kind, WaveKind::Boss);
        assert_eq!(game.npcs.len(), p.count as usize, "Boss 波数量仍按 count");
        let boss = game.npcs.last().unwrap();
        let b = p.boss.expect("Boss 波应有主怪参数");
        assert!((boss.max_hp - b.hp).abs() < 1e-6, "主怪血量");
        assert!((boss.speed - b.speed).abs() < 1e-6, "主怪速度");
        assert!((boss.attack_range - b.attack_range).abs() < 1e-6, "主怪攻击距离");
        for npc in game.npcs.iter().take(game.npcs.len() - 1) {
            assert!((npc.max_hp - p.hp).abs() < 1e-6, "小怪血量按 profile");
            assert!(npc.speed > boss.speed, "主怪应慢于小怪");
        }
    }

    /// 特殊波次：援军波在波开始 1.5s 后补怪 1..=2 只，且只触发一次
    #[test]
    fn reinforcement_wave_spawns_mid_wave() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.spawn_wave(3, &glam::Vec3::ZERO);
        let p = wave_profile(3);
        assert_eq!(p.kind, WaveKind::Reinforced);
        assert_eq!(game.npcs.len(), p.count as usize, "援军补怪前数量 = count");
        // 推进 2 秒：1.5s 处应触发补怪
        for _ in 0..120 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(game.reinforcement_done, "援军应已触发");
        assert_eq!(
            game.npcs.len(),
            (p.count + p.reinforcement_count) as usize,
            "补怪后数量 = count + reinforcement_count"
        );
        // 再推进 1 秒：不应二次补怪
        let before = game.npcs.len();
        for _ in 0..60 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(game.npcs.len(), before, "援军只触发一次");
    }

    /// 特殊波次：清波条件不变（全歼才算清空），Boss/援军波后波次推进正常
    #[test]
    fn special_waves_keep_clear_and_progress() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        // 常规波（波 2）：清掉当前 npc 后应正常推进到波 3
        game.spawn_wave(2, &glam::Vec3::ZERO);
        assert_eq!(game.wave, 2);
        game.npcs.clear();
        for _ in 0..330 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.wave, 3, "常规波清空后应推进");
        assert_eq!(
            wave_profile(3).kind,
            WaveKind::Reinforced,
            "推进后的波 3 应为援军波"
        );
        // 第 3 波是每关最后一波：清空后升关到 level 2 wave 1
        game.npcs.clear();
        for _ in 0..330 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.level, 2, "第 3 波清空应升关");
        assert_eq!(game.wave, 1, "升关后回本关第 1 波");
        // Boss 波：level 2 第 2 波 = 累计有效波 5，清空后推进到本关第 3 波
        game.spawn_wave(2, &glam::Vec3::ZERO);
        assert_eq!(game.wave, 2);
        let p5 = wave_profile(5);
        assert_eq!(p5.kind, WaveKind::Boss, "有效波 5 应为 Boss 波");
        assert_eq!(p5.total_count, p5.count, "Boss 波总敌人数 = count（含主怪）");
        game.npcs.clear();
        for _ in 0..330 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.level, 2, "Boss 波清空后应留在本关");
        assert_eq!(game.wave, 3, "Boss 波清空后应推进到本关第 3 波");
    }

    /// 设置面板：开关、音量/灵敏度调整、选中项循环
    #[test]
    fn settings_panel_toggle_and_adjust() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert!(!game.settings_open(), "初始应关闭");
        game.toggle_settings();
        assert!(game.settings_open(), "toggle 后应打开");
        let vol = game.hud.volume;
        game.adjust_settings(0.1);
        assert!(
            (game.hud.volume - (vol + 0.1).min(1.0)).abs() < 1e-6,
            "音量应增加"
        );
        game.cycle_settings();
        let sens = game.hud.sensitivity;
        game.adjust_settings(-0.1);
        assert!(game.hud.sensitivity < sens, "灵敏度应降低");
        game.toggle_settings();
        assert!(!game.settings_open(), "再 toggle 应关闭");
    }

    // ---- 爆炸/冲击波（AoE 玩法）----

    /// 测试用爆炸：替换 npcs 后引爆，返回爆炸实体引用
    fn explode_on(npcs: Vec<Npc>, center: [f32; 3], damage: f32, knockback: bool) -> Game {
        let mut game = Game::new();
        game.npcs = npcs;
        game.spawn_explosion(center, EXPLOSION_RADIUS, damage, knockback);
        game
    }

    /// AoE 伤害：爆心全伤、随距离衰减、超出半径无损（衰减语义 = shockwave_pressure）
    #[test]
    fn explosion_aoe_damage_falloff() {
        let game = explode_on(
            vec![
                npc_at(1, Team::Red, [0.0, 0.0, 0.0]),
                npc_at(2, Team::Red, [4.0, 0.0, 0.0]),
                npc_at(3, Team::Red, [20.0, 0.0, 0.0]),
            ],
            [0.0, 1.0, 0.0],
            EXPLOSION_DAMAGE,
            true,
        );
        let hp: Vec<f32> = game.npcs.iter().map(|n| n.hp).collect();
        assert!(hp[0] < hp[1], "爆心最近者受伤最重");
        assert!(hp[1] < hp[2], "随距离衰减");
        assert!((hp[2] - 100.0).abs() < 1e-6, "超出半径不受伤");
        assert!(hp[0] < 100.0 - 30.0, "近爆心伤害显著");
        assert!(game.npcs[1].knockback[0] > 0.0, "径向推挤方向向外（+x）");
        assert_eq!(game.npcs[2].knockback, [0.0, 0.0], "超出半径无推挤");
    }

    /// 爆炸对障碍 AoE 伤害：半径内障碍掉血、可摧毁；超出半径无损（阶段二）
    #[test]
    fn explosion_damages_obstacles_in_radius() {
        let mut game = Game::new();
        // 注入两个障碍：一个 Barrier（100HP，爆心可摧毁）在（0,0），一个在远处（50,50）
        game.map.obstacles.push(MapObstacle {
            x: 0.0,
            z: 0.0,
            half_w: 1.0,
            half_d: 1.0,
            kind: ObstacleKind::Barrier,
            max_hp: 100.0,
            hp: 100.0,
        });
        game.map.obstacles.push(MapObstacle {
            x: 50.0,
            z: 50.0,
            half_w: 1.0,
            half_d: 1.0,
            kind: ObstacleKind::Wall,
            max_hp: 150.0,
            hp: 150.0,
        });
        game.spawn_explosion([0.0, 1.0, 0.0], 8.0, 120.0, true);
        // Game::new() 预置 20 个环带障碍 + 注入 2 个 = 22；爆心 Barrier（100HP）被
        // 120×1.0×fall(1.0)=120 伤摧毁 → 21
        assert_eq!(game.map.obstacles.len(), 21, "爆心 Barrier（100HP）被 120 伤摧毁");
        // 注入的远处障碍（50,50）保留且无伤
        let far = game
            .map
            .obstacles
            .iter()
            .find(|o| (o.x - 50.0).abs() < 1.0 && (o.z - 50.0).abs() < 1.0)
            .expect("远处障碍应保留");
        assert!((far.hp - 150.0).abs() < 1e-5, "远处障碍无伤");
    }

    /// NPC 投掷手榴弹：压力模式 Attack 态 NPC 冷却结束 → 朝敌对目标投掷（阶段二）
    #[test]
    fn npc_throws_grenade_in_stress_combat() {
        let mut game = Game::new();
        game.stress = true;
        let mut a = npc_at(0, Team::Red, [0.0, 0.0, 0.0]);
        let mut b = npc_at(1, Team::Blue, [8.0, 0.0, 0.0]);
        // 推进到 Attack 态
        let p = NpcPerception {
            enemy_visible: true,
            enemy_in_range: true,
            ..NpcPerception::default()
        };
        a.state_machine.update(p);
        a.state_machine.update(p);
        b.state_machine.update(p);
        b.state_machine.update(p);
        a.grenade_timer = 0.0; // 冷却结束，允许投掷
        b.grenade_timer = 999.0; // 对侧不投掷（验证确定性只投掷冷却结束者）
        game.npcs = vec![a, b];
        let targets = pick_stress_targets(&game.npcs, STRESS_SIGHT);
        let before = game.grenades_vec.len();
        // 帧号取 id*31 % 8 < 8 恒真 → 必投掷（h = (0*31 + frame*7) % 100，frame 取使 h<8）
        game.frame_no = 1;
        game.npc_throw_grenades(1.0 / 60.0, &targets);
        assert!(
            game.grenades_vec.len() > before,
            "冷却结束的 Attack 态 NPC 应投掷手榴弹"
        );
    }

    /// 爆炸击杀：hp≤0 移除 + 计分 + 任务推进
    #[test]
    fn explosion_kills_and_scores() {
        let mut npcs = vec![npc_at(7, Team::Red, [0.5, 0.0, 0.0])];
        npcs[0].hp = 30.0;
        npcs[0].last_hp = 30.0;
        let game = explode_on(npcs, [0.0, 1.0, 0.0], EXPLOSION_DAMAGE, true);
        assert!(game.npcs.is_empty(), "30hp NPC 被爆心击杀");
        assert_eq!(game.score, KILL_SCORE, "击杀计分");
        assert_eq!(game.objective.eliminated, 1, "任务目标推进");
    }

    /// survive 规则波次推进：清波 → 补给窗口（血量回复/弹药补满）→ 下一波；
    /// 守住全部波 → 胜利态（阶段一）
    #[test]
    fn survive_rule_advances_waves_and_wins_at_last() {
        let mut game = Game::new();
        game.obj_state = Some(crate::engine::objective::ObjectiveState::new(
            crate::engine::objective::GameRule::Survive { waves: 2 },
        ));
        game.on_any_key(&glam::Vec3::ZERO);
        game.hud.health = 50.0; // 半血，验证补给回复
        game.grenades = 0;
        let camera = Camera::new();
        // 清空 NPC（模拟玩家清完 wave 1）+ 推进 update_waves
        game.npcs.clear();
        game.wave_timer = 0.0;
        for _ in 0..200 {
            game.update(1.0 / 60.0, &camera);
        }
        // wave 1 清完 → wave_timer 置为 WAVE_INTERMISSION → 递减到 0 → wave 2 + 补给
        assert_eq!(game.wave, 2, "survive 第 1 波清完应进入第 2 波");
        assert!(
            game.hud.health > 50.0,
            "波间补给应回复血量: {}",
            game.hud.health
        );
        assert_eq!(game.grenades, game.grenades_max, "波间补给应补满手榴弹");
        // 清完 wave 2（最后一波）→ 胜利
        game.npcs.clear();
        game.wave_timer = 0.0;
        for _ in 0..300 {
            game.update(1.0 / 60.0, &camera);
            if game.game_state == GameState::Victory(crate::engine::ai::Team::Blue) {
                break;
            }
        }
        assert_eq!(
            game.game_state,
            GameState::Victory(crate::engine::ai::Team::Blue),
            "守住全部波次应胜利"
        );
    }

    /// 玩家自伤：手榴弹爆炸在玩家附近 → 掉血但不秒杀（封顶 SELF_DAMAGE_CAP）；
    /// 爆炸中心偏移保证玩家不被自己秒杀（阶段二）
    #[test]
    fn grenade_self_damage_capped_not_fatal() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.hud.health = 100.0;
        // 玩家在 (0, 1.6, 0)，爆炸在 3m 外（半径 8m 内，fall ≈ 0.625）
        game.spawn_explosion([3.0, 0.0, 0.0], 8.0, 120.0, true);
        let hp = game.hud.health;
        assert!(hp < 100.0, "近距离爆炸应造成自伤: {}", hp);
        assert!(
            hp >= 100.0 - SELF_DAMAGE_CAP,
            "自伤封顶（不被秒杀）: hp={} cap={}",
            hp,
            SELF_DAMAGE_CAP
        );
        assert!(
            game.game_state == GameState::Playing,
            "封顶自伤不应致死"
        );
    }

    /// 冲击波推挤：生成时获得径向速度，后续帧按指数衰减并产生位移
    #[test]
    fn explosion_knockback_moves_and_decays() {
        let mut game = explode_on(
            vec![npc_at(1, Team::Red, [3.0, 0.0, 0.0])],
            [0.0, 1.0, 0.0],
            EXPLOSION_DAMAGE,
            true,
        );
        let v0 = game.npcs[0].knockback[0];
        assert!(v0 > 1.0, "近爆心推挤速度可观");
        let d0 = game.npcs[0].position[0].abs();
        // 推挤帧：位移方向向外（+x），速度按指数衰减
        game.update(0.05, &Camera::new());
        assert!(
            game.npcs[0].position[0] > d0 + 0.05,
            "推挤帧应产生向外位移"
        );
        assert!(
            game.npcs[0].knockback[0] < v0,
            "速度应衰减"
        );
        // 数帧后速度归零（衰减到阈值清 0）
        for _ in 0..30 {
            game.update(0.05, &Camera::new());
        }
        assert_eq!(game.npcs[0].knockback, [0.0, 0.0], "衰减后归零");
    }

    /// 爆炸实体生命周期：生成 → 年龄推进 → 超时移除；玩家在冲击半径内触发震屏
    #[test]
    fn explosion_visual_lifecycle_and_shake() {
        let mut game = Game::new();
        game.spawn_explosion([0.0, 1.0, 0.0], EXPLOSION_RADIUS, EXPLOSION_DAMAGE, false);
        assert_eq!(game.explosions().len(), 1, "生成即入列");
        // 玩家在原点：爆炸半径 8m 内 → 震屏触发
        assert!(game.shake_timer > 0.0, "近爆应触发震屏");
        let (sx, sz) = game.camera_shake_offset();
        assert!(sx != 0.0 || sz != 0.0, "震屏期间偏移非零");
        game.step_explosions(0.1);
        assert_eq!(game.explosions().len(), 1, "存活期内保留");
        assert!((game.explosions()[0].age - 0.1).abs() < 1e-6, "年龄推进");
        game.step_explosions(0.3);
        assert!(game.explosions().is_empty(), "超过 lifetime 移除");
        game.step_explosions(SHAKE_DURATION + 0.1);
        assert_eq!(game.shake_timer, 0.0, "震屏超时归零");
        assert_eq!(game.camera_shake_offset(), (0.0, 0.0));
    }

    /// 冻结玩法（GameOver / damage=0）：爆炸只有视觉，不结算伤害/击退
    #[test]
    fn explosion_zero_damage_is_visual_only() {
        let game = explode_on(
            vec![npc_at(1, Team::Red, [1.0, 0.0, 0.0])],
            [0.0, 1.0, 0.0],
            0.0,
            true,
        );
        assert_eq!(game.npcs[0].hp, 100.0, "damage=0 不扣血");
        assert_eq!(game.npcs[0].knockback, [0.0, 0.0], "damage=0 不推挤");
        assert_eq!(game.explosions().len(), 1, "视觉实体仍生成");
    }

    // ---- 压力模式（64v64 大战场）----

    /// 测试用 NPC 构造器（全字段确定性初始化）
    fn npc_at(id: usize, team: Team, pos: [f32; 3]) -> Npc {
        Npc {
            id,
            position: pos,
            speed: 4.0,
            attack_range: 12.0,
            home: [pos[0], pos[2]],
            state_machine: NpcStateMachine::new(),
            perception: NpcPerception::default(),
            path: Vec::new(),
            path_index: 0,
            hp: 100.0,
            max_hp: 100.0,
            role: TacticalRole::Rusher,
            tactic: Tactic::Advance,
            dodge_timer: 0.0,
            hit_cooldown: 0.0,
            last_hp: 100.0,
            team,
            facing: 0.0,
            fire_accum: 0.0,
            knockback: [0.0, 0.0],
            grenade_timer: 0.0,
        }
    }

    #[test]
    fn stress_spawn_creates_balanced_two_teams() {
        let mut game = Game::new();
        game.stress = true;
        game.stress_sides = 8;
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        game.spawn_stress_battle(&player);
        assert_eq!(game.npcs.len(), 16, "8v8 应出生 16 名 NPC");
        let red = game.npcs.iter().filter(|n| n.team == Team::Red).count();
        let blue = game.npcs.iter().filter(|n| n.team == Team::Blue).count();
        assert_eq!(red, 8);
        assert_eq!(blue, 8);
        // 红半场 +X、蓝半场 -X，出生点在场内且不在障碍格上
        for n in &game.npcs {
            assert!(n.position[0].abs() <= 250.0 && n.position[2].abs() <= 250.0);
            assert!(
                game.grid.is_passable(world_to_grid(n.position[0], n.position[2])),
                "npc #{} 出生点应在可通行格",
                n.id
            );
            if n.team == Team::Red {
                assert!(n.position[0] > 0.0, "红方应在 +X 半场");
            } else {
                assert!(n.position[0] < 0.0, "蓝方应在 -X 半场");
            }
        }
    }

    #[test]
    fn stress_target_picking_prefers_nearest_enemy() {
        // 3 红 + 2 蓝；红0 最近敌 = 蓝4（√34 ≈ 5.8 < 蓝3 的 8）
        let npcs = vec![
            npc_at(0, Team::Red, [0.0, 0.0, 0.0]),
            npc_at(1, Team::Red, [10.0, 0.0, 0.0]),
            npc_at(2, Team::Red, [-100.0, 0.0, -100.0]),
            npc_at(3, Team::Blue, [8.0, 0.0, 0.0]),
            npc_at(4, Team::Blue, [5.0, 0.0, 3.0]),
        ];
        let targets = pick_stress_targets(&npcs, NPC_SIGHT);
        assert_eq!(targets[0], Some((4, npcs[4].position, npcs[4].facing)));
        assert_eq!(targets[1], Some((3, npcs[3].position, npcs[3].facing)), "红1(10,0) 最近敌 = 蓝3(8,0)");
        assert_eq!(targets[2], None, "视野外无敌人 → 玩家兜底");
        assert_eq!(targets[3], Some((1, npcs[1].position, npcs[1].facing)), "蓝3 最近敌 = 红1");
        assert_eq!(targets[4], Some((0, npcs[0].position, npcs[0].facing)), "同距取索引小者");
    }

    #[test]
    fn stress_parallel_step_matches_serial() {
        // 64 NPC（32 红 / 32 蓝），串行与并行逐帧推进 6 帧，状态必须逐位一致。
        // 并行路径模拟真实流程：先 `partition_ai_tiers` 分层重排（Near 在前），
        // 再 pick targets（重排后索引对齐），然后双池 `step_ai_parallel(near_len)`；
        // 数组顺序因重排而不同，断言按 npc.id 对齐比较。
        let mut npcs_s: Vec<Npc> = Vec::new();
        let mut npcs_p: Vec<Npc> = Vec::new();
        for i in 0..64usize {
            let team = if i < 32 { Team::Red } else { Team::Blue };
            let x = if i < 32 {
                80.0 + i as f32 * 3.0
            } else {
                -80.0 - (i - 32) as f32 * 3.0
            };
            let z = ((i % 8) as f32 - 4.0) * 10.0;
            npcs_s.push(npc_at(i, team, [x, 0.0, z]));
            npcs_p.push(npc_at(i, team, [x, 0.0, z]));
        }
        let game = Game::new();
        let grid = game.grid.clone();
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        let tier_params = AiTierParams::default();
        for frame in 0..6u32 {
            let dt = 1.0 / 60.0;
            let time = 1.0 + frame as f32 * dt;
            let targets_s = pick_stress_targets(&npcs_s, STRESS_SIGHT);
            // 并行路径：分层重排 → 重排后 pick（与 update_ai 顺序一致）
            let near_len = partition_ai_tiers(&mut npcs_p, |n| {
                let dx = n.position[0] - player.x;
                let dz = n.position[2] - player.z;
                classify_ai_tier(dx * dx + dz * dz, false, &tier_params)
            });
            assert!(near_len > 0 && near_len < npcs_p.len(), "近/远组都应非空");
            let targets_p = pick_stress_targets(&npcs_p, STRESS_SIGHT);
            let flags_s = vec![false; npcs_s.len()];
            let flags_p = vec![false; npcs_p.len()];
            let ctx_s = AiStepCtx {
                player: &player,
                player_yaw: 0.0,
                charge: false,
                under_fire: &flags_s,
                targets: &targets_s,
                grid: &grid,
                time,
                dt,
                stress: true,
                frame,
                decimate_far: false,
                ring_inner: MAP_RING_INNER,
                ring_outer: MAP_RING_OUTER,
                obstacles: &game.map.obstacles,
            };
            let ctx_p = AiStepCtx {
                player: &player,
                player_yaw: 0.0,
                charge: false,
                under_fire: &flags_p,
                targets: &targets_p,
                grid: &grid,
                time,
                dt,
                stress: true,
                frame,
                decimate_far: false,
                ring_inner: MAP_RING_INNER,
                ring_outer: MAP_RING_OUTER,
                obstacles: &game.map.obstacles,
            };
            Game::step_ai_serial(&mut npcs_s, &ctx_s);
            Game::step_ai_parallel(&mut npcs_p, near_len, &ctx_p);
            // 按 id 对齐比较（并行路径数组已重排）
            let by_id: Vec<(&Npc, &Npc)> = npcs_s
                .iter()
                .map(|a| {
                    let b = npcs_p
                        .iter()
                        .find(|b| b.id == a.id)
                        .expect("并行路径应包含同一 NPC 集合");
                    (a, b)
                })
                .collect();
            for (a, b) in by_id {
                assert_eq!(a.position, b.position, "frame {} npc {}", frame, a.id);
                assert_eq!(a.hp, b.hp);
                assert_eq!(a.facing, b.facing);
                assert_eq!(a.tactic, b.tactic);
                assert_eq!(a.state_machine.state(), b.state_machine.state());
                assert_eq!(a.path, b.path);
            }
        }
    }

    /// 阶段二：压力模式「目标 NPC 朝向」驱动 player_facing → Flanker 触发 Flank/Ambush。
    /// 红 NPC 站在蓝 NPC 正面（蓝 facing 指向红）→ 红应判定「目标面朝我」→ Flank；
    /// 红 NPC 站在蓝 NPC 背面（蓝 facing 背对红）→ 红应判定「目标背对我」→ Ambush。
    #[test]
    fn stress_flanker_tactic_follows_target_facing() {
        // 蓝 NPC 面朝 +X 方向（facing = atan2(dz,dx)：dz=0,dx>0 → facing=0）
        let mut blue = npc_at(1, Team::Blue, [0.0, 0.0, 0.0]);
        blue.facing = 0.0; // 面朝 +X
        blue.role = TacticalRole::Flanker;
        // 红 NPC 在蓝的正+X 侧 10m（蓝面朝它 → 目标面朝本 NPC）
        let mut red_front = npc_at(0, Team::Red, [10.0, 0.0, 0.0]);
        red_front.role = TacticalRole::Flanker;
        // 红 NPC 在蓝的 -X 侧 10m（蓝背对它 → 目标背对本 NPC）
        let mut red_back = npc_at(2, Team::Red, [-10.0, 0.0, 0.0]);
        red_back.role = TacticalRole::Flanker;

        let game = Game::new();
        let grid = game.grid.clone();
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        let flags = vec![false; 3];

        let npcs_a = vec![red_front, blue, red_back];
        let targets_a = pick_stress_targets(&npcs_a, STRESS_SIGHT);
        let ctx_a = AiStepCtx {
            player: &player,
            player_yaw: 0.0,
            charge: false,
            under_fire: &flags,
            targets: &targets_a,
            grid: &grid,
            time: 1.0,
            dt: 1.0 / 60.0,
            stress: true,
            frame: 0,
            decimate_far: false,
            ring_inner: MAP_RING_INNER,
            ring_outer: MAP_RING_OUTER,
            obstacles: &game.map.obstacles,
        };
        let mut npcs_a = npcs_a;
        Game::step_ai_serial(&mut npcs_a, &ctx_a);
        // 红 0（正面）最近敌 = 蓝 1 且蓝面朝它 → player_facing=true → Flank
        assert_eq!(
            npcs_a[0].tactic,
            Tactic::Flank,
            "正面站位的红应触发 Flank（目标面朝本 NPC）"
        );
        // 红 2（背面）最近敌 = 蓝 1 但蓝背对它 → player_facing=false → Ambush
        assert_eq!(
            npcs_a[2].tactic,
            Tactic::Ambush,
            "背面站位的红应触发 Ambush（目标背对本 NPC）"
        );
    }

    /// 阶段二：压力模式 NPC 在障碍附近交火时触发 CoverSeek（掩体利用）。
    /// 红 NPC 位于障碍旁 20m 处（Chase 态目标在 30m 外）→ 应进入掩体利用；
    /// 开阔处（无障碍）目标在射程外 → 保持 Advance（不误触发）。
    #[test]
    fn stress_cover_seek_triggers_near_obstacle() {
        let mut game = Game::new();
        game.stress = true;
        // 构造一个障碍（墙）：在 (0,0) 位置放一个 MapObstacle → 网格会 block 该格
        // 用 street_fight 式的墙：x=0,z=0,half_w=5,half_d=0.5 → 格 (0,0) 及邻域 blocked
        let ob = MapObstacle {
            x: 0.0,
            z: 0.0,
            half_w: 5.0,
            half_d: 0.5,
            kind: ObstacleKind::Wall,
            max_hp: 150.0,
            hp: 150.0,
        };
        game.map.obstacles.push(ob);
        // 重建网格（把障碍格 block）
        let mut grid = GridMap::new(GRID_SIZE, GRID_SIZE);
        for o in &game.map.obstacles {
            let g0 = world_to_grid(o.x - o.half_w, o.z - o.half_d);
            let g1 = world_to_grid(o.x + o.half_w, o.z + o.half_d);
            for gx in g0.x..=g1.x {
                for gz in g0.y..=g1.y {
                    let pos = GridPos::new(gx, gz);
                    if grid.in_bounds(pos) {
                        grid.block(pos);
                    }
                }
            }
        }
        game.grid = grid;
        let grid = game.grid.clone();

        // 红 NPC 在障碍旁（-10, 0，距障碍 10m），Chase 态（感知目标但不在射程）
        let mut red = npc_at(0, Team::Red, [-10.0, 0.0, 0.0]);
        red.state_machine.update(NpcPerception {
            enemy_visible: true,
            enemy_in_range: false,
            ..NpcPerception::default()
        }); // Idle → Chase
        red.role = TacticalRole::Rusher;
        // 蓝目标在 30m 外（+20,0），Chase 态
        let mut blue = npc_at(1, Team::Blue, [20.0, 0.0, 0.0]);
        blue.state_machine.update(NpcPerception {
            enemy_visible: true,
            enemy_in_range: false,
            ..NpcPerception::default()
        });
        let npcs = vec![red, blue];
        let targets = pick_stress_targets(&npcs, STRESS_SIGHT);
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        let flags = vec![false; 2];
        let ctx = AiStepCtx {
            player: &player,
            player_yaw: 0.0,
            charge: false,
            under_fire: &flags,
            targets: &targets,
            grid: &grid,
            time: 1.0,
            dt: 1.0 / 60.0,
            stress: true,
            frame: 0,
            decimate_far: false,
            ring_inner: MAP_RING_INNER,
            ring_outer: MAP_RING_OUTER,
            obstacles: &game.map.obstacles,
        };
        let mut npcs = npcs;
        Game::step_ai_serial(&mut npcs, &ctx);
        // 红在障碍旁 + Chase + 目标 30m（≤ attack_range 12 + 40）→ 应进入 CoverSeek
        assert_eq!(
            npcs[0].tactic,
            Tactic::CoverSeek,
            "障碍旁的 NPC 在 Chase 接近目标时应利用掩体（CoverSeek）"
        );
    }

    #[test]
    fn stress_npc_combat_damages_target_only() {
        let mut game = Game::new();
        game.stress = true;
        let mut a = npc_at(0, Team::Red, [0.0, 0.0, 0.0]);
        let mut b = npc_at(1, Team::Blue, [6.0, 0.0, 0.0]);
        // 推进到 Attack 态：Idle → Chase → Attack
        let p = NpcPerception {
            enemy_visible: true,
            enemy_in_range: true,
            ..NpcPerception::default()
        };
        a.state_machine.update(p);
        a.state_machine.update(p);
        b.state_machine.update(p);
        b.state_machine.update(p);
        assert_eq!(a.state_machine.state(), NpcState::Attack);
        assert_eq!(b.state_machine.state(), NpcState::Attack);
        game.npcs = vec![a, b];
        let targets = vec![
            Some((1, game.npcs[1].position, game.npcs[1].facing)),
            Some((0, game.npcs[0].position, game.npcs[0].facing)),
        ];
        let hp_before = [game.npcs[0].hp, game.npcs[1].hp];
        let dps = wave_profile(game.effective_wave(1)).dps;
        game.apply_npc_combat(1.1, &targets);
        assert!(
            (game.npcs[0].hp - (hp_before[0] - dps)).abs() < 1e-3,
            "红 0 应被扣 dps"
        );
        assert!(
            (game.npcs[1].hp - (hp_before[1] - dps)).abs() < 1e-3,
            "蓝 1 应被扣 dps"
        );
    }

    #[test]
    fn stress_wipe_respawns_full_battle() {
        let mut game = Game::new();
        game.stress = true;
        game.stress_sides = 4;
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        game.spawn_stress_battle(&player);
        assert_eq!(game.npcs.len(), 8);
        let round0 = game.stress_round;
        for n in &mut game.npcs {
            if n.team == Team::Red {
                n.hp = 0.0;
            }
        }
        game.game_state = GameState::Playing;
        game.update_stress_respawns(&player);
        assert_eq!(game.stress_round, round0 + 1, "团灭应开新一轮");
        assert_eq!(game.npcs.len(), 8, "全量补员");
        let red = game.npcs.iter().filter(|n| n.team == Team::Red).count();
        assert_eq!(red, 4);
    }

    // ---- 新玩法：可破坏障碍 / 掩体利用 / 任务目标 ----

    /// 可破坏障碍：扣血不摧毁 → 保留；血尽 → 从物理刚体/AI 网格/渲染列表中移除
    #[test]
    fn obstacles_take_damage_and_destroy() {
        let mut game = Game::new();
        assert!(!game.map.obstacles.is_empty());
        let n = game.map.obstacles.len();
        assert_eq!(game.world.bodies.len(), n, "刚体与障碍应一一对应");
        let idx = 0;
        let ob0 = game.map.obstacles[idx];
        assert!(ob0.max_hp > 0.0 && ob0.hp == ob0.max_hp, "障碍出生满血");
        // 一格：扣血不摧毁
        game.damage_obstacle(idx, 25.0);
        assert_eq!(game.map.obstacles[idx].hp, ob0.max_hp - 25.0);
        assert_eq!(game.map.obstacles.len(), n);
        assert_eq!(game.world.bodies.len(), n);
        // 摧毁：清空剩余血量 → 列表同步移除
        game.damage_obstacle(idx, ob0.max_hp);
        assert_eq!(game.map.obstacles.len(), n - 1, "障碍应从渲染列表移除");
        assert_eq!(game.world.bodies.len(), n - 1, "物理刚体应同步移除");
        // 被摧毁障碍覆盖的网格格已解除阻挡（NPC 可穿过缺口）
        let g0 = world_to_grid(ob0.x - ob0.half_w, ob0.z - ob0.half_d);
        let g1 = world_to_grid(ob0.x + ob0.half_w, ob0.z + ob0.half_d);
        let mut any_passable = false;
        for gx in g0.x..=g1.x {
            for gz in g0.y..=g1.y {
                let pos = GridPos::new(gx, gz);
                if game.grid.in_bounds(pos) && game.grid.is_passable(pos) {
                    any_passable = true;
                }
            }
        }
        assert!(any_passable, "摧毁后障碍占格应解除阻挡");
        // 越界下标安全忽略
        game.damage_obstacle(usize::MAX, 999.0);
        assert_eq!(game.map.obstacles.len(), n - 1);
    }

    /// 掩体利用：环带内、射程内、紧邻存活障碍的遮挡掩体被选中；
    /// 中央安全区目标无可用掩体 → None（冒烟站定语义）
    #[test]
    fn attack_cover_picks_ring_band_shielding_cover() {
        let mut grid = GridMap::new(GRID_SIZE, GRID_SIZE);
        let obstacles = vec![MapObstacle {
            x: 60.0,
            z: 0.0,
            half_w: 3.0,
            half_d: 3.0,
            kind: ObstacleKind::Wall,
            max_hp: 150.0,
            hp: 150.0,
        }];
        for ob in &obstacles {
            let g0 = world_to_grid(ob.x - ob.half_w, ob.z - ob.half_d);
            let g1 = world_to_grid(ob.x + ob.half_w, ob.z + ob.half_d);
            for gx in g0.x..=g1.x {
                for gz in g0.y..=g1.y {
                    let pos = GridPos::new(gx, gz);
                    if grid.in_bounds(pos) {
                        grid.block(pos);
                    }
                }
            }
        }
        // 目标（被攻击方）在障碍东侧（环带内）；NPC 在障碍西侧追近
        let target = world_to_grid(66.0, 0.0);
        let npc = world_to_grid(50.0, 0.0);
        let cover = pick_attack_cover(
            &grid,
            npc,
            target,
            12.0,
            MAP_RING_INNER,
            MAP_RING_OUTER,
            COVER_MAX_DIST,
            &obstacles,
        );
        assert!(cover.is_some(), "环带内应有射程内遮挡掩体");
        let (wx, wz) = grid_to_world(cover.unwrap());
        let d_origin = (wx * wx + wz * wz).sqrt();
        assert!(
            d_origin >= MAP_RING_INNER && d_origin <= MAP_RING_OUTER,
            "掩体必须在障碍环带内: {:.1}m",
            d_origin
        );
        let (tx, tz) = grid_to_world(target);
        let d_t = ((wx - tx).powi(2) + (wz - tz).powi(2)).sqrt();
        assert!(d_t <= 12.0, "掩体必须在攻击距离内: {:.1}m", d_t);
        // 安全区目标（原点）：环带内无射程内掩体 → None（NPC 保持直线推进/站定）
        let origin = world_to_grid(0.0, 0.0);
        let none_cover = pick_attack_cover(
            &grid,
            npc,
            origin,
            12.0,
            MAP_RING_INNER,
            MAP_RING_OUTER,
            COVER_MAX_DIST,
            &obstacles,
        );
        assert!(none_cover.is_none(), "中央安全区目标应无可用掩体");
    }

    /// 任务目标：歼灭数推进、达成只触发一次、计数封顶在 target
    #[test]
    fn mission_objective_progress_and_completion() {
        let mut obj = MissionObjective::new(24);
        assert_eq!((obj.eliminated, obj.target, obj.done), (0, 24, false));
        assert!(!obj.progress(23), "未达目标不应完成");
        assert!(obj.progress(1), "第 24 击杀应达成目标");
        assert!(obj.done);
        assert!(!obj.progress(1), "达成后不再重复触发");
        assert_eq!(obj.eliminated, 24, "计数封顶在 target");
        let mut zero = MissionObjective::new(0);
        assert!(!zero.progress(1), "target=0 永不达成");
    }

    /// 任务目标：普通模式本关目标 = 3 波出场总数（含援军）；压力模式 = 歼灭一队
    #[test]
    fn objective_targets_per_mode_and_stress_victory() {
        let game = Game::new();
        assert!(!game.stress);
        // 第 1 关：wave1=6 + wave2=8 + wave3=10+援军2 = 26
        assert_eq!(game.objective.target, 26, "第 1 关任务目标应为 3 波出场总数");
        assert!(!game.objective.done);
        assert!(game.hud.victory_banner.is_none());
        // 压力模式：目标 = 歼灭一队；红方团灭 → 达成 + 横幅，且补员照常开新一轮
        let mut game = Game::new();
        game.stress = true;
        game.stress_sides = 4;
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        game.spawn_stress_battle(&player);
        assert_eq!(game.objective.target, 4, "压力模式目标 = 歼灭一队");
        for n in &mut game.npcs {
            if n.team == Team::Red {
                n.hp = 0.0;
            }
        }
        game.game_state = GameState::Playing;
        let round0 = game.stress_round;
        game.update_stress_respawns(&player);
        assert_eq!(game.stress_round, round0 + 1, "补员逻辑不受影响");
        assert!(game.hud.victory_banner.is_some(), "达成后应显示胜利横幅（保留到下一轮）");
        assert_eq!(game.objective.eliminated, 0, "新一轮目标已重置");
        assert_eq!(game.objective.target, 4, "新一轮目标 = 歼灭一队");
    }
}

/// 攻击态掩体选择：在障碍环带 `[ring_inner, ring_outer]` 内、紧邻存活障碍盒、
/// 且距目标不超过 `attack_range` 的遮挡掩体点中选最优（封闭性优先、其次离目标远——
/// 贴近射程边缘的掩体到位即可开火）。
///
/// - 掩体候选来自 `find_cover_shielding`（阻挡格挡在 NPC 与目标之间）
/// - 环带与障碍列表由调用方传入（读 MAP_RING_INNER/MAP_RING_OUTER 与关卡障碍列表；
///   摧毁后的障碍已从列表移除，其掩体点随之失效）
/// - 中央安全区内没有障碍 → 返回 None → 调用方保持直线推进/原地站定（冒烟机制不变）
fn pick_attack_cover(
    grid: &GridMap,
    npc: GridPos,
    target: GridPos,
    attack_range: f32,
    ring_inner: f32,
    ring_outer: f32,
    max_dist: u32,
    obstacles: &[MapObstacle],
) -> Option<GridPos> {
    let mut best: Option<(u32, u32, GridPos)> = None;
    for cover in find_cover_shielding(grid, npc, target, max_dist) {
        let (wx, wz) = grid_to_world(cover.pos);
        let d_origin = (wx * wx + wz * wz).sqrt();
        if d_origin < ring_inner || d_origin > ring_outer {
            continue;
        }
        // 掩体必须紧邻存活障碍盒（容差 GRID_CELL*2 覆盖"格中心到盒边"的最坏距离）
        let near_obstacle = obstacles.iter().any(|o| {
            (wx - o.x).abs() <= o.half_w + GRID_CELL * 2.0
                && (wz - o.z).abs() <= o.half_d + GRID_CELL * 2.0
        });
        if !near_obstacle {
            continue;
        }
        let (tx, tz) = grid_to_world(target);
        let dx = wx - tx;
        let dz = wz - tz;
        if dx * dx + dz * dz > attack_range * attack_range {
            continue;
        }
        let dist_t = target.manhattan(cover.pos);
        let better = match best {
            None => true,
            Some((bo, bd, _)) => {
                cover.openness < bo || (cover.openness == bo && dist_t > bd)
            }
        };
        if better {
            best = Some((cover.openness, dist_t, cover.pos));
        }
    }
    best.map(|(_, _, pos)| pos)
}

/// 按状态与战术推进单个 NPC：目标选择 → A* 寻路 → 移动（锯齿/躲避）→ 地形高度采样
fn advance_npc(
    npc: &mut Npc,
    state: NpcState,
    tactic: Tactic,
    target: &glam::Vec3,
    target_yaw: f32,
    grid: &GridMap,
    ring_inner: f32,
    ring_outer: f32,
    obstacles: &[MapObstacle],
    time: f32,
    dt: f32,
    stress: bool,
) {
    // 无路径（或已走完）时按状态 + 战术选择目标
    if npc.path.is_empty() || npc.path_index >= npc.path.len() {
        let goal = match state {
            NpcState::Chase => match tactic {
                // 突击/压制：直线逼近（压制手到射程边缘即转 Attack 站定）
                Tactic::Advance | Tactic::Suppress => world_to_grid(target.x, target.z),
                // 侧翼包抄：垂直轴向偏移 3 格（12m），id 奇偶定左右形成钳形
                Tactic::Flank => {
                    let target_g = world_to_grid(target.x, target.z);
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    let side = if npc.id % 2 == 0 { 1 } else { -1 };
                    flank_goal(grid, target_g, npc_g, side, FLANK_OFFSET)
                }
                // 偷袭绕背：玩家未面朝时绕大圈（20m 偏移）从背后逼近
                Tactic::Ambush => {
                    let target_g = world_to_grid(target.x, target.z);
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    ambush_goal(grid, target_g, npc_g, target_yaw, AMBUSH_OFFSET)
                }
                // 掩体跃进：逐掩体推进（只选比当前更靠近玩家的掩体）
                Tactic::CoverAdvance => {
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    let target_g = world_to_grid(target.x, target.z);
                    let cur = npc_g.manhattan(target_g);
                    match find_cover_points(grid, npc_g, COVER_MAX_DIST)
                        .into_iter()
                        .find(|c| c.dist < cur)
                    {
                        Some(cover) => cover.pos,
                        None => target_g,
                    }
                }
                // 掩体利用：障碍环带内选"距目标 ≤ 攻击距离"的遮挡掩体，先到掩体再开火；
                // 无可用掩体（中央安全区）→ 直线推进，保持站定/站定日志语义。
                // 压力模式（NPC-vs-NPC）：沿目标方向找遮挡掩体（NPC 穿越障碍带时利用），
                // 就近取第一个；环带过滤不适用（NPC 在环带外）。
                Tactic::CoverSeek => {
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    let target_g = world_to_grid(target.x, target.z);
                    if stress {
                        crate::engine::ai::find_cover_shielding(
                            grid,
                            npc_g,
                            target_g,
                            STRESS_COVER_MAX_DIST,
                        )
                        .first()
                        .map(|c| c.pos)
                        .unwrap_or(target_g)
                    } else {
                        pick_attack_cover(
                            grid,
                            npc_g,
                            target_g,
                            npc.attack_range,
                            ring_inner,
                            ring_outer,
                            COVER_MAX_DIST,
                            obstacles,
                        )
                        .unwrap_or(target_g)
                    }
                }
                // 低血量撤退：撤向最封闭且较远的遮挡掩体（阻挡格挡在 NPC 与玩家之间）
                Tactic::Retreat => {
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    let target_g = world_to_grid(target.x, target.z);
                    find_cover_shielding(grid, npc_g, target_g, COVER_MAX_DIST)
                        .first()
                        .map(|c| c.pos)
                        .unwrap_or(npc_g)
                }
                Tactic::Hold => world_to_grid(npc.position[0], npc.position[2]),
            },
            NpcState::Attack => {
                // 就近掩体站定（贴障碍簇）；无掩体原地（保持攻击站定日志供冒烟瞄准）
                let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                match find_cover_points(grid, npc_g, COVER_MAX_DIST).first() {
                    Some(cover) => cover.pos,
                    None => npc_g,
                }
            }
            NpcState::Patrol | NpcState::Idle => {
                // 确定性巡逻点：随 id 相位与时间缓慢旋转
                let angle = npc.id as f32 * 2.399 + (time / 8.0).floor() * 0.7;
                let r = 20.0 + npc.id as f32 * 3.0;
                world_to_grid(
                    npc.home[0] + r * angle.cos(),
                    npc.home[1] + r * angle.sin(),
                )
            }
        };
        let start = world_to_grid(npc.position[0], npc.position[2]);
        npc.path = find_path(grid, start, goal).unwrap_or_default();
        npc.path_index = 0;
    }

    // 躲避冷却/计时无条件递减（含 Attack 态，防冻结窗口；残留计时归零防"幽灵侧移"）
    npc.hit_cooldown = (npc.hit_cooldown - dt).max(0.0);
    npc.dodge_timer = (npc.dodge_timer - dt).max(0.0);

    // 爆炸冲击波推挤：覆盖本帧移动（指数衰减，约 0.25s 内衰减到 5%）
    if npc.knockback[0] != 0.0 || npc.knockback[1] != 0.0 {
        npc.position[0] += npc.knockback[0] * dt;
        npc.position[2] += npc.knockback[1] * dt;
        let decay = (-KNOCKBACK_DECAY * dt).exp();
        npc.knockback[0] *= decay;
        npc.knockback[1] *= decay;
        if npc.knockback[0].abs() < 0.05 && npc.knockback[1].abs() < 0.05 {
            npc.knockback = [0.0, 0.0];
        }
        npc.position[1] = terrain_height_at(npc.position[0], npc.position[2]);
        return;
    }

    // 攻击态原地站定（冒烟瞄准依据 `npc: #id stand`）
    if state == NpcState::Attack {
        npc.position[1] = terrain_height_at(npc.position[0], npc.position[2]);
        return;
    }

    // 受击/火力威胁后侧向弹开（垂直于 目标→NPC 方向，id 奇偶定左右）
    if npc.dodge_timer > 0.0 {
        let dx = npc.position[0] - target.x;
        let dz = npc.position[2] - target.z;
        let d = (dx * dx + dz * dz).sqrt().max(1e-4);
        let side = if npc.id % 2 == 0 { 1.0 } else { -1.0 };
        let step = npc.speed * dt;
        npc.position[0] += -dz / d * side * step;
        npc.position[2] += dx / d * side * step;
        npc.position[1] = terrain_height_at(npc.position[0], npc.position[2]);
        return;
    }

    let (tx, tz) = if npc.path_index < npc.path.len() {
        grid_to_world(npc.path[npc.path_index])
    } else {
        (npc.position[0], npc.position[2])
    };
    let dx = tx - npc.position[0];
    let dz = tz - npc.position[2];
    let d = (dx * dx + dz * dz).sqrt();
    if d < 1.0 {
        npc.path_index += 1;
    } else if d > 1e-4 {
        // 推进态锯齿机动：垂直前进方向横向摆动（被瞄准/火力威胁时幅度加大）
        let dxp = npc.position[0] - target.x;
        let dzp = npc.position[2] - target.z;
        let dist_p = (dxp * dxp + dzp * dzp).sqrt();
        let (mut mx, mut mz) = (dx / d, dz / d);
        if state == NpcState::Chase && dist_p < ZIGZAG_DIST && tactic != Tactic::Retreat {
            let amp = if npc.perception.under_fire || npc.perception.player_aiming {
                ZIGZAG_AMP_HIGH
            } else {
                ZIGZAG_AMP
            };
            let off = zigzag_offset(time, npc.id as u32, amp);
            mx += -dz / d * off;
            mz += dx / d * off;
            let mlen = (mx * mx + mz * mz).sqrt().max(1e-4);
            mx /= mlen;
            mz /= mlen;
        }
        let step = npc.speed * dt;
        npc.position[0] += mx * step;
        npc.position[2] += mz * step;
    }
    npc.position[1] = terrain_height_at(npc.position[0], npc.position[2]);
}
