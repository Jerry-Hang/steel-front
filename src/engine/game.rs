//! 游戏运行时中枢
//!
//! 把 weapons / ai / physics / ui / audio / network 模块接进主循环：
//! - 每帧 `update(dt, camera, fire)` 推进物理、武器、AI、音频、网络
//! - 渲染前由 main.rs 取 HUD quad 列表与光照 uniform
//!
//! 本文件只做模块间编排与少量胶水逻辑，具体算法仍留在各模块内。

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::audio::{AudioListener, AudioPlayer, AudioSource, Channel, SfxBank, SfxKind};
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
use super::geom::Shape;
use super::renderer::terrain_height_at;
// 子模块里写的是 `super::lighting::…`（这段代码原来就在 game.rs，`super` 指 engine）；
// 下沉一层后 `super` 变成 game ⇒ 这里再导出一份，让**搬走的字节保持原样**
// （判据 tools/refactor_move_check.py 因此不需要为它加归一化；renderer.rs 有同款说明）。
pub(crate) use super::lighting;
// 自由函数搬进子模块后，外部路径靠这条再导出维持不变（renderer.rs 同款做法）。
pub(crate) use ai_util::*;
pub(crate) use map_util::*;
pub(crate) use diag::*;
use super::window::{WINDOW_HEIGHT, WINDOW_WIDTH};
use super::weapon_data::{build_firearm, ALL_WEAPONS};
use super::weapons::{Grenade, Projectile, WeaponRack, GRENADE_FUSE_MAX, GRENADE_FUSE_MIN, GRENADE_SPEED};
use crate::ui::HudState;


/// AI 网格覆盖范围：128×128 格 × 4m = ±256m（与实例场/地形同域）
const GRID_CELL: f32 = 4.0;
const GRID_SIZE: usize = 128;
const GRID_HALF: f32 = GRID_CELL * (GRID_SIZE as f32) * 0.5;
/// NPC 数量
const NPC_COUNT: usize = 8;
/// NPC 命中球心高度（脚上 1.0m；hit_npc_index 的球心与 hit_height 公式共用此值）
const NPC_HIT_CENTER_Y: f32 = 1.0;
/// NPC 手榴弹出手高度（米，相对脚底 = 手的位置，不是脚底）。
/// 🔴 必须离地：`update_grenades` 的落地判据是 `y <= ground + 0.05`，从脚底出手的
/// 手榴弹在**高帧率下第一帧仍落在容差内**（出手后第一帧只上升 `vy*dt`，dt≤9.3ms 即
/// ≥108fps 时不足 5cm）⇒ 原地引爆，8m/120 伤的 AoE 把投掷者自己打死。
/// 见回归测试 `npc_grenade_does_not_detonate_on_release`。
const NPC_GRENADE_RELEASE_Y: f32 = 1.2;
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
/// 跳跃初速（m/s，~0.55m 跳高——真实二战士兵跳跃感，2026-08-15 从 4.6 调低去除"月球漫步"）
const JUMP_SPEED: f32 = 3.3;

/// 冲刺速度倍率。只在「站立 + 前进 + 未开镜 + 在地面」时生效，条件见 `GameState::sprinting`。
const SPRINT_MUL: f32 = 1.65;

/// 一次打药耗时（秒）：期间不能重复使用，HUD 显示进度。
const HEAL_TIME: f32 = 2.5;
/// 一次打药回复的生命值（战术射击的常见量级：约半管血）
const HEAL_AMOUNT: f32 = 45.0;

/// 玩家姿态（站 / 蹲 / 卧）。
///
/// **速度与视高只能从这里派生**，不许在别处再写一遍倍率：本仓历史上最贵的一类 bug 就是
/// 「同一个量有两套状态源」（见教训清单第 1 条）。要调数值就改这里的方法，一处生效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stance {
    #[default]
    Standing,
    Crouching,
    Prone,
}

/// 重力加速度（m/s²，19.6 = 2x 真实重力——FPS 手感偏好（下落干脆），配合低跳高）
const GRAVITY: f32 = 19.6;

/// 🔴 2026-09-13：**冲刺跳惯性的空中衰减率**（每秒的比例）。
/// 起跳时带走的水平速度按 `(1 - AIR_DRAG*dt)` 逐帧衰减，落地清零。
/// 取 0.6 的依据：`JUMP_SPEED = 3.3`、`GRAVITY = 19.6` ⇒ 滞空 ≈ `2*3.3/19.6 ≈ 0.34s`，
/// 衰减到 `(1-0.6*0.34) ≈ 80%` —— 冲刺（6.0×1.65 = 9.9 m/s）能多飞约 2.6m，
/// 足够跃过一条人行道或矮墙，又不会变成"滑翔"。
const AIR_DRAG: f32 = 0.6;

/// 🔴 2026-09-13：**抬脚上限（米）** —— 走到多高的障碍前可以直接迈上去。
/// 路缘 ~0.15、台阶 ~0.18、护栏 ~1.5、集装箱 ~2.6。
/// 取 0.45：路缘与台阶能上，护栏与集装箱必须跳 —— 这正是"跳"该有的意义。
/// 配合 `PlayerBody::support_height`（顶面支撑）与 `push_out_of_aabb` 的 Y 判据，
/// 玩家第一次能真的站到东西上面。
const PLAYER_STEP_UP: f32 = 0.45;

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
/// 掩体利用触发距离（米）：Chase 态距目标 ≤ 攻击距离 + 该值时先寻障碍环带掩体。
///
/// 🔴 2026-09-26 试过 20 → 32（想让掩护推进覆盖整段接近路线），**一轮实测没支持它**：
/// 掩体类占比 CoverAdvance 从 7.2% 掉到 4.4%、CoverSeek 持平 1.8%（单轮噪声级，但方向不对）
/// ⇒ 按"改动必须能被量出来"的纪律**回退到 20**。真正让掩体战术从 0% 变成有值的是
/// "掩体爬行者不被冲锋覆盖"那一条（见 `update_ai_npc` 里的注释），与这个半径无关。
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
/// 侧翼包抄偏移（格）。**硬约束：×`GRID_CELL` 必须明显小于 `attack_range`（12m）**。
///
/// 🔴 2026-09-25 由 3 改为 2：旧值 3 格 = **12m == 射程**，于是包抄手走到目的地时仍在射程外
/// 一步 ⇒ `state` 恒为 `Chase`、永不进 `Attack`，同时还反复换点。真机日志（`RV3D_AI_DIAG=1`）
/// 抓到它在这两个点之间走了整场：`tac=Flank goal=(14.0,2.0) → (2.0,14.0) → (14.0,2.0) …`，
/// NPC 离玩家 15–22m 来回，「第 1 波清不掉」的最后一块拼图。
/// 2 格 = 8m，留 4m 余量给玩家在格内的偏移与路径末端的落点误差
/// （判据 `flank_and_ambush_goals_land_inside_engage_range`）。
const FLANK_OFFSET: u32 = 2;
/// 偷袭绕背偏移（格）。与 `FLANK_OFFSET` 同一条约束（同样由 5 改为 2，理由见上）。
/// 「绕大圈」现在由寻路路线（障碍环带/楼群）提供，不再靠一个射程外的远目标点。
const AMBUSH_OFFSET: u32 = 2;
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
pub(crate) struct NetworkDemo {
    server: Server,
    client: Client,
    seq: u32,
    last_log: f32,
}

/// 游戏主状态机（开始菜单 → 游戏中 → 死亡/胜利/失败结算）
/// 开火模式（B 键循环切换）：单发 / 双发 / 三连发 / 连发
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FireMode {
    /// 单发：每次按下只打 1 发（狙击/精确射手默认手感）
    Semi,
    /// 双发：每次按下快速连打 2 发。比三连发省弹、比单发火力密度高，
    /// 是"点射武器"的常见档位。
    Burst2,
    /// 三连发：每次按下快速连打 3 发
    Burst3,
    /// 连发：按住持续以武器射速开火
    Auto,
}




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
    /// 寻路全空时直行机动（残局卡死修复）；目标世界坐标
    pub direct_goal: bool,
    pub direct_x: f32,
    pub direct_z: f32,
    /// 诊断用：本帧选定的**目标世界坐标**（路径目标格中心 / 直行兜底点 / 换位点）。
    ///
    /// 🔴 2026-09-25 实测加（未结案 #17）：`aidiag` 只能看到 `state=Chase`、`path=3/11`、
    /// 速度 3.9 m/s —— 一切正常，**却有一个 NPC 在两点之间来回走了一个半小时**（波次清不掉）。
    /// 那是 `Tactic::Flank` 的包抄点在"主导轴"翻转时**跳变**造成的：只要不把"它到底要去哪"
    /// 打出来，这个 bug 在日志里完全隐形。**行为无关，只读诊断字段。**
    pub last_goal: [f32; 2],
    /// 攻击态站定时长（火-机动交替打：站打几秒→换位）
    pub attack_timer: f32,
    /// 换位目标（Some=正在机动换位；到位后清空回到站打）
    pub reposition: Option<[f32; 2]>,
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
    /// `RV3D_AI_DIAG` 逐 NPC 节流状态：上一次打印所用的 (5s 桶 << 32 | id)。
    /// 必须是**每只 NPC 一份**——旧实现是一个全局 `AtomicU64` 只记最后一个 key，
    /// 两只以上卡住的 NPC 逐帧交替即恒真（§20.6 实测 300s 刷 16.4 万行）。
    /// `step_npc` 对每个 `npc` 是 `&mut` 独占（`par_for_each_mut`）⇒ 无需 atomic。
    aidiag_bucket: u64,
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






/// `RV3D_AI_DIAG=1` 时的计数：NPC 因为**自己站在阻挡格里**而被挪回可站立点的次数。
///
/// 🔴 2026-09-25 实测加：修掉"起点搬运"之后 `起点阻挡` 永远为 0（**自己的补丁把自己的
/// 测量变成恒 0 了**）—— 必须有一个**独立于寻路调用**的计数器来回答"NPC 到底有没有站在墙里"。
/// 1 Hz 由状态日志取走并清零。关了诊断时是一次 `Relaxed` 自增（可忽略）。
static NOTE_NPC_UNSTUCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `RV3D_AI_DIAG=1` 时的**移动归因**计数（未结案 #17 的定位工具）。
///
/// 🔴 2026-09-25 实测加：`aidiag: #id state=Chase pos=(…) path=7/21 wp_d=3.4` 这类日志只能
/// 证明"NPC 有路径、没在闪避"，**证明不了它这一帧到底动了没有**；而实测位置 10 秒几乎不变
/// （爬行 0.02 m/s vs 设定 4 m/s）。移动被抵消只有两条可能路径，两根计数器把它们分开：
///   - `NOTE_MOVE_UNDONE`：走了路径/直行、但**移动后被障碍 AABB 推回**（抵消 >50% 位移）；
///   - `NOTE_SEP_BIG`   ：**被邻居的分离力推开**，幅度 ≥ 半步（0.08m ≈ 25fps 下半帧位移）。
/// 1 Hz 由状态日志取走并清零；关了诊断时每条是一次 `Relaxed` 自增（可忽略）。
static NOTE_MOVE_STEP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NOTE_MOVE_UNDONE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NOTE_SEP_PUSHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NOTE_SEP_BIG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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



/// 封格判据：障碍盒与格子（4m×4m）的重叠面积 ≥ 该值（m²）才把这一格标成阻挡。
///
/// 🔴 2026-09-25 由 **0.0（碰到就封）** 改为 **1/3 格（≈5.33m²）**，理由与实测数据见
/// `block_obstacle_cells` 的文档。一句话：4m 的格子对这个 1:1 的世界太粗，
/// "碰到就封"会把**几何上并不相连的装饰件在网格里连成一道墙**，
/// 实测把玩家封在 24 格（城市）/ 4 格（defense_line）的孤岛里 ⇒ 波次永远清不掉。
///
/// 阈值取 1/3 而不是 1/2，是为了让 3×3m 的哨塔仍能封格（最实的格子 6.25m² = 39%），
/// 保住 `find_cover_points` 的掩体点判定；而 6×0.9 隔离墩（3.6m² = 22%）、
/// 1m 厚沙袋（4m² = 25%）、0.34m 护柱（1.4m²）都不再封格。
///
/// ⚠️ 只对**短件**生效；长件（≥ [`CELL_BLOCK_LONG_EXTENT_M`]）仍按保守规则封格。
const CELL_BLOCK_MIN_OVERLAP_M2: f32 = GRID_CELL * GRID_CELL / 3.0;

/// "长件"阈值（米）：任一水平方向 ≥ 该值的障碍按**保守规则**（碰到就封）建网。
///
/// 🔴 2026-09-25 补（覆盖率规则的第一版修正）：只按覆盖率封格时，1m 厚的沙袋环/矮墙
/// **不再封格** ⇒ A* 的路径直接穿墙 ⇒ NPC 撞上后只能贴墙滑 ⇒ 在**凹角**里来回磨死。
/// 真机实测（RV3D_AI_DIAG=1，defense_line 残局）：剩下两只卡在沙袋环的**内角**
/// （`#9 pos=(8.3,8.9) #10 pos=(-8.9,1.6)`，`wp_d` 恒定 3.0，逐秒位移 0.2m，
/// `occluded=true` ⇒ 进不了 Attack），`被障碍抵消` 却是 0 —— 因为滑动确实在动，只是原地打转。
///
/// 判据因此分成两类：
///   - **长件**（沙袋/矮墙/围墙/建筑，≥8m）：NPC 必须**绕着走**，先把所在格封住；
///   - **短件**（隔离墩 6m / 长椅 6m / 花坛 3.4m / 护柱 0.34m / 树 0.4m）：按覆盖率封格 ——
///     它们封整格才是"把 4m 格子放大成一面墙"的根源。
const CELL_BLOCK_LONG_EXTENT_M: f32 = GRID_CELL * 2.0;


/// 障碍种类：决定摆放形态与尺寸（渲染侧 marker 颜色由 main.rs 按 kind 映射）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObstacleKind {
    /// 墙体：1..=3 个中等盒子沿切线并排成墙（第一关基准形态）
    Wall,
    /// 大块：1..=2 个大尺寸独立方块（密度高、体型大）
    Block,
    /// 路障：2..=4 个长条薄墙（单簇更长、更稀疏）
    Barrier,
    /// 树木：场景装饰（安全环外），细长圆柱+树冠方块，可碰撞可击穿
    Tree,
    /// 建筑物：场景装饰（安全环外），大尺寸方块，可碰撞高耐久
    Building,
    /// 残骸：场景装饰（安全环外），矮小散落方块群
    Ruin,
}


/// 地图静态障碍盒（AABB：世界坐标中心 (x, y, z) + 半尺寸 half_w/half_h/half_d）。
/// 贴地障碍 y = half_h；城市建筑/树冠可抬升 y（如树冠 3.4m）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapObstacle {
    pub x: f32,
    pub z: f32,
    pub half_w: f32,
    pub half_d: f32,
    /// 盒中心高度（米；贴地障碍 = half_h）
    pub y: f32,
    /// 半高（米；贴地障碍默认 1.2 = 原 MAP_BLOCK_HEIGHT/2）
    pub half_h: f32,
    /// 障碍种类（第一关全部为 Wall；主题轮换见 theme_for_level）
    pub kind: ObstacleKind,
    /// 材质 tint 覆盖（城市手绘地图用：消防栓红/树冠绿/玻璃蓝等；None = 按种类默认色）
    pub tint: Option<[f32; 3]>,
    /// 血量上限（按种类，见 obstacle_max_hp）
    pub max_hp: f32,
    /// 当前血量（归 0 → 摧毁：从物理刚体/AI 网格/渲染 marker 中移除）
    pub hp: f32,
    /// 几何形状（**只管渲染模板**）。默认 [`Shape::Legacy`] = 旧行为（立方体，
    /// 绿色 tint 兜底成二十面体），所以既有构造点零改动即保持原画面。
    ///
    /// ⚠ 不参与碰撞：碰撞体始终是这条障碍的 AABB，与 shape 无关。
    /// 详见 `Shape::inscribed_radius_factor` 上那段"尚未接线"的说明。
    pub shape: Shape,
}


/// 程序化关卡布局：确定性（种子 = 关卡号），障碍全部位于中央安全环带之外
#[derive(Debug, Clone, Default)]
pub struct LevelMap {
    pub obstacles: Vec<MapObstacle>,
    /// **纯装饰几何**：只进渲染 marker 与路径追踪，不进刚体表、不进 AI 导航网格、
    /// 不可摧毁、不是掩体点。
    ///
    /// 为什么不复用 `obstacles` 加个 `collide: bool`：`hit_obstacle_index` 返回
    /// `world.bodies` 的下标，`damage_obstacle` 直接把它当 `map.obstacles` 的下标用
    /// （两表按下标一一对应、摧毁时同步 remove）。任何让两表错长的过滤都会把伤害
    /// 结算打到另一栋楼上——而且是静默打错，测试也测不出来。装饰件本来就和
    /// "障碍"是两种东西，分成两张表比在每个遍历点补 if 更不容易漏。
    ///
    /// 真建模必须这么做：一栋楼拆成结构体 + 挑檐 + 窗带 + 壁柱 + 女儿墙 + 屋顶设备
    /// 就有十几个部件，全部塞进物理会让弹道×障碍与玩家×障碍成本随部件数线性上涨，
    /// 而且屋顶空调机会在街面留下一圈隐形墙。
    pub decor: Vec<MapObstacle>,
    /// **GLB 世界道具摆放**（2026-09-03）：由 `engine::props::PropSet` 的网格下标 +
    /// 位姿描述，渲染走独立绘制路径，不进 `obstacles`/`decor` 两张盒表。
    ///
    /// 为什么单开一张表而不是给 `MapObstacle` 加个 mesh 字段：障碍表的下标被
    /// `world.bodies` 和伤害结算按位置复用（见 `decor` 字段的说明），任何让两表
    /// 错长的改动都会把伤害静默打到另一栋楼上。道具是另一种东西，就给它自己的表。
    ///
    /// 为什么这里的件几乎都是装饰：GLB 换掉的正是 `decor` 里那批"挑檐/窗带/壁柱/
    /// 屋顶设备"薄盒——它们在街对面读作一排悬挑板（缺陷 D11）。结构盒留在
    /// `obstacles` 里继续负责碰撞与耐久，所以画面重建完，物理一行没动。
    pub props: Vec<crate::engine::props::PropPlacement>,
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


/// 障碍种类对应的摆放风格：簇内盒数范围 + 盒子半尺寸范围 + 盒间间隙
#[derive(Debug, Clone, Copy)]
pub(crate) struct KindStyle {
    min_boxes: u32,
    max_boxes: u32,
    min_w: f32,
    max_w: f32,
    min_d: f32,
    max_d: f32,
    gap: f32,
}




/// 碰撞事件缓冲：监听者写入，Game 每帧 drain 取走
struct EventBuffer(Arc<Mutex<Vec<CollisionEvent>>>);


/// 网络远端玩家（服务器权威：位置/朝向/血量由服务器模拟并广播）
#[derive(Debug, Clone, Copy)]
pub struct NetPlayer {
    /// 客户端分配的 player id（Join-ack）
    pub id: u32,
    /// 世界位置
    pub pos: [f32; 3],
    /// 朝向（yaw/pitch）
    pub yaw: f32,
    pub pitch: f32,
    /// 血量（0 = 阵亡，服务器复活后回满）
    pub hp: f32,
    /// 是否存活
    pub alive: bool,
    /// 最近一次收到输入的时间（断线/超时判定）
    pub last_rx: f32,
    /// 开火节流（连发间隔）
    pub fire_accum: f32,
    /// 最近开火时刻（快照 firing 指示）
    pub last_fire: f32,
}

/// 弹着标记（子弹打在障碍表面留下的弹孔）的数量上限。
///
/// 环形缓冲：满了丢最旧的一条。弹孔池大小固定，不随游玩时长增长，
/// 也不会因为打多了就把实例槽位占满。
pub const IMPACT_MARK_MAX: usize = 192;
/// 弹着标记存活时长（秒）。
pub const IMPACT_MARK_LIFE: f32 = 30.0;
/// 生命末尾的收缩时长（秒）：最后这段里尺寸线性收到 0，避免"啪"地消失。
pub const IMPACT_MARK_FADE: f32 = 3.0;

/// 子弹在障碍表面留下的弹着标记（弹孔）。
///
/// 位置与法线来自**线段与刚体 AABB 的入口交点**，与子弹碰撞用的是同一份 slab 求交
/// （`Game::segment_aabb_entry`）。不能拿子弹当前位置当弹孔：高速弹一帧飞十几米，
/// 那个位置已经在墙**里面**了。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImpactMark {
    /// 弹孔中心（落在障碍表面）
    pub pos: [f32; 3],
    /// 表面法线（单位向量；AABB 面法线，轴向）
    pub normal: [f32; 3],
    /// 已存活时间（秒）
    pub age: f32,
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
    /// 医疗包库存（X 键使用；默认 2，上限 2）
    medkits: u32,
    medkits_max: u32,
    /// 打药剩余时间（秒）。>0 表示正在打药；归零那一帧一次性回血。
    heal_timer: f32,
    /// 在场投掷物（抛物线 + 引信计时）
    grenades_vec: Vec<Grenade>,
    /// 待施加到相机的后坐力（pitch/yaw 弧度，main.rs 每帧 drain 取走）
    pending_kick: (f32, f32),
    /// 第一人称玩家身体（WASD 移动 + 与演示刚体碰撞，y 每帧贴地形）
    player_body: PlayerBody,
    /// 移动输入标志（main.rs 转发 WASD；仅 Playing + FPS 生效）
    move_forward: bool,
    move_backward: bool,
    /// 跳跃请求（Space/Jump 绑定按下置位；落地清除）
    jump_pressed: bool,
    /// 玩家姿态。速度与视高的**唯一来源**，见 `Stance`。
    stance: Stance,
    /// 冲刺键（Shift）当前是否按住。是否真的在冲刺由 `sprinting()` 判定。
    sprint_held: bool,
    /// 跳跃垂直速度（m/s，>0 上升；落地归零）
    jump_vel: f32,
    /// 🔴 2026-09-13：**起跳时带走的水平速度**（冲刺跳的惯性）。
    /// 起跳瞬间由当时的有效速度写入，空中按 `AIR_DRAG` 衰减着继续推进
    /// （叠加在削弱后的空中控制之上），落地清零。
    /// 用户要求："在奔跑的时候，跳的时候会有向前的力，会直接跟过去一样越过去"。
    jump_hvel: glam::Vec3,
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
    /// 🔴 **当前正在结算的爆炸中心**（2026-09-15）：`spawn_explosion` 进入时写入，
    /// 供 `damage_npc` 拼"爆炸（x,z）击杀了…"那一行 —— 爆炸没有单一击杀者
    /// （一发 AoE 同时结算多个目标），报"某某杀的"会是编的，所以只报爆点位置。
    /// 只在 AoE 结算期间有意义；不在结算期间的值是上一次的残留，不参与任何判定。
    last_blast_center: [f32; 3],
    /// 爆炸震屏剩余时间（秒，>0 时相机叠加抖动偏移）
    shake_timer: f32,
    /// 爆炸震屏强度（世界位移米数，随剩余时间线性衰减）
    shake_strength: f32,
    /// 开火冷却剩余时间（秒）
    fire_cooldown: f32,
    /// 散布缩放（1.0 = 腰射全散布；main.rs 按 ADS 混合设置，开镜时缩小到 0.3）
    spread_scale: f32,
    /// 剔除用的眼位覆盖（2026-09-12 第④条修）。
    ///
    /// `npc_occluded` 原本硬取 `player_eye()`。正常玩法里相机**就是**玩家眼位 ⇒ 正确；
    /// 但 `RV3D_CAM` / `RV3D_NPC_CAM` 把调试相机移离玩家时，玩家仍在原点 ⇒
    /// **相机眼前的人被判为"从玩家位置看不到"而全部剔除**。
    /// 实测：关剔除前 `npc=288`（≈16 人），开后 `npc=4590`（255 人 × 18 段）。
    ///
    /// `main.rs` 每帧按"相机与玩家眼位的距离"写它：**正常玩法下恒为 `None`**（行为不变），
    /// 只有调试相机偏离 > 1m 时才生效。**这样取证不再需要 `RV3D_NO_NPC_CULL=1` 那条 workaround。**
    pub cull_eye_override: Option<glam::Vec3>,
    /// `target_occlusion` 的缓存与年龄（2026-09-12 第 102 轮）。
    /// 该节占 `ai_us` 的 18–39%（244–519µs，第 50 轮实测），却是每帧为全部 NPC 做的
    /// 线段-AABB 扫描；遮挡关系在相邻帧之间几乎不变，故每 `OCCLUSION_REFRESH` 帧重算一次。
    /// `occl_cache_age == 0` 表示"该重算了"。
    pub occl_cache: Vec<bool>,
    pub occl_cache_age: u32,
    /// 遮挡剔除自计时（累计微秒 / 调用次数），只有 `RV3D_CULL_DIAG=1` 时累加；
    /// `main.rs` 每秒读一次并清零（`Cell` = 只读方法也能累加）。
    pub occl_us: std::cell::Cell<u64>,
    pub occl_calls: std::cell::Cell<u64>,
    /// 渲染用「玩家看得见否」缓存：逐槽位 `(npc id, 可见)` —— 带 id 是为了在**下标前移**
    /// （有人死亡）时立刻发现错位，见 `refresh_npc_visibility`。`npc_vis_scans` = 累计重算次数。
    npc_vis: Vec<(usize, bool)>,
    npc_vis_frame: u32,
    pub npc_vis_scans: std::cell::Cell<u64>,
    /// 开火模式（B 键循环切换）
    fire_mode: FireMode,
    /// 连发热量 0..1：连续射击累积，压制枪口上扬；停火后衰减
    auto_heat: f32,
    /// NPC 受击闪白：id → 剩余秒数（命中瞬间置 0.15，衰减；渲染侧混合白色反馈）
    npc_hit_flash: std::collections::HashMap<usize, f32>,
    /// 本帧命中点（世界坐标；main.rs 读取后生成命中火花粒子，每帧清空）
    hit_points: Vec<[f32; 3]>,
    /// 弹着标记（弹孔）：子弹打在障碍表面留下的痕迹，`main.rs` 每帧转成 WorldMarker 绘制。
    /// 只在障碍上留痕（树冠/道具那类球体不留），上限 [`IMPACT_MARK_MAX`]。
    impact_marks: Vec<ImpactMark>,
    /// 本帧命中伤害值（HUD 伤害飘字；与 hit_points 一一对应，每帧清空）
    hit_damages: Vec<f32>,
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
    /// `RV3D_AI_DIAG=1` 专用：上一秒的 NPC 位置快照（id, x, z）+ 对应时刻，用来算**每只 NPC
    /// 的真实速度**（位移 / Δt）。未结案 #17 的定位工具：`state=Chase` + 有路径 + 不在闪避
    /// 仍然可能**原地不动**，只有"一秒钟走了几米"能把这件事量化；逐帧打印会刷屏 ⇒ 每秒聚合
    /// 一次（平均速度 / 停滞只数 / 最慢三只）。不诊断时它恒为空，不占内存也不改行为。
    ai_diag_prev: Vec<(u32, f32, f32)>,
    ai_diag_prev_t: f32,
    /// 同步冲锋滞回状态（开启后需 <60% 才取消）
    charge_active: bool,
    /// NPC 数量缩放（RV3D_NPC_SCALE，默认 1.0；压测多人对战压力场景用）
    npc_scale: f32,
    /// 压力模式（RV3D_STRESS_AI）：红蓝各 `stress_sides` 名 NPC 大战场对抗
    stress: bool,
    /// 压力模式每边 NPC 数量（默认 64 → 64v64）
    stress_sides: usize,
    /// 指挥体系（压力模式）：红蓝各一营（三三制 营连排班，见 ai_command.rs）
    command: Option<(crate::engine::ai_command::Army, crate::engine::ai_command::Army)>,
    /// LLM 指挥官（RV3D_LLM 启用）：战役级决策（红/蓝各一独立上下文窗口）
    llm: Option<crate::llm_cmd::LlmCommander>,
    /// 玩家无敌（数据收集/演示用：RV3D_INVINCIBLE=1 或 RV3D_LLM 启用时自动开）
    player_invincible: bool,
    /// 兵力耗尽后的下一轮重置时刻（-1 = 未触发）
    round_reset_at: f32,
    /// 本轮获胜方（兵力先耗尽方判负）
    round_winner: Option<Team>,
    /// 本轮开始时刻（超时判胜用；-1 = 未开战）
    round_started_at: f32,
    /// 本轮红/蓝累计阵亡（指挥军情 kills 字段）
    round_kills_red: u32,
    round_kills_blue: u32,
    /// 指挥军情日志节流
    command_log_at: f32,
    /// 压力模式对抗轮次（一方团灭补员后 +1）
    stress_round: u32,
    /// 并行 AI 更新开关（RV3D_AI_PARALLEL=off 关闭，可串行 A/B 对比）
    ai_parallel: bool,
    /// 性能探针：本帧各阶段耗时（µs，1Hz 日志输出，定位 CPU 侧瓶颈）
    stage_physics_us: u64,
    stage_ai_us: u64,
    stage_audio_us: u64,
    stage_net_us: u64,
    /// `ai_us` 的**分项**（未结案 #25 的"AI 到底花在哪"）：那一格其实是整段玩法
    /// （投掷物 + AI + 波次 + 据点）的合计 ⇒ 不拆开就无法判断优化该往哪打。
    /// 本帧值 + 本秒累计值（`RV3D_AI_DIAG=1` 时随 aidiag 行每秒打一条）。
    stage_proj_us: u64,
    stage_ai_only_us: u64,
    stage_wave_us: u64,
    stage_obj_us: u64,
    acc_proj_us: u64,
    acc_ai_only_us: u64,
    acc_wave_us: u64,
    acc_obj_us: u64,
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
    /// 音频播放器（Windows waveOut 真实发声；无设备时静默降级，混音链路恒运行）
    audio: AudioPlayer<crate::audio_out::DefaultSink>,
    /// 音频采样率
    audio_sample_rate: u32,
    /// 网络环回演示（默认关闭，RV3D_NET=1 启用）
    net_demo: Option<NetworkDemo>,
    /// 服务器模式（RV3D_NET=server）：权威模拟 + 每 tick 广播快照
    pub net_server: Option<Server>,
    /// 网络远端玩家（服务器权威模拟；客户端模式时为空，远端实体由快照消费）
    net_players: Vec<NetPlayer>,
    /// 客户端模式（RV3D_NET=client）：每 tick 上报输入 + 快照插值缓冲
    pub net_client: Option<Client>,
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
    /// 地图代号：每次 `apply_level` 自增。渲染层用它判断"地图换了，道具几何要重传"，
    /// 避免每帧重做一次百万级顶点的 CPU 合并（`Renderer::set_props` 的调用条件）。
    map_generation: u64,
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
    /// 上次 `npcpos:` 时间（频率由 `RV3D_NPC_POS_HZ` 决定，见 `npc_pos_period`）
    last_npcpos_log: f32,
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
pub(crate) struct AiStepCtx<'a> {
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
    /// 班指挥目标点（压力模式：未接敌战士按班目标编队推进；None = 逐人战术照旧）
    squad_wps: &'a [Option<[f32; 2]>],
    /// 观战模式（玩家无敌）：NPC 无可见敌人时的兜底目标 = 敌方重心（不再锁玩家造成火力浪费）
    spectator: bool,
    /// 目标**已知**（2026-09-23，未结案 #17 的修法②）：进攻方从出生起就知道要打哪。
    /// 普通波次/防守波为 `true`（玩家是它的任务目标），压力模式为 `false`
    /// （红蓝对抗有自己的选目标逻辑 `pick_stress_targets`），开始菜单游走也为 `false`。
    /// ⚠️ 它**不是**开火许可：开火仍要求 `enemy_visible`（视距 + 遮挡），
    /// 见 `ai.rs::NpcPerception::target_known` 与 `NpcStateMachine::update`。
    target_known: bool,
    fallback_targets: &'a [[f32; 3]],
    /// 每 NPC 的"目标是否被几何体挡住"。`true` = 挡住。
    ///
    /// 这是"隔墙掉血"的正解。此前 `enemy_visible = dist < sight` **只看距离、零视线检测**，
    /// 所以一堵楼墙完全不影响 AI：NPC 隔着整栋建筑持续对你输出，而你既看不到他、
    /// 也躲不掉——用户报的"穿墙/透视"里最严重的一条其实是这个逻辑洞，不是几何。
    /// 空 slice = 无数据（保持旧行为），供单测与未接线路径使用。
    target_occluded: &'a [bool],
}





/// 渲染用「玩家看得见否」缓存的重算周期（帧）—— 纯常量，判据见
/// `npc_visibility_cache_is_staggered` 与 `npc_visibility_refreshes_within_the_window`。
///
/// **实测依据（2026-09-26）**：255 NPC / 1240 障碍的压力场景，`cull-diag` 中位
/// **102 ms/s**（≈51k 次调用/s、每次约 2 µs、每帧 510 次）⇒ 在 98 fps（帧 5.5 ms）下
/// **约占 18% 的帧预算**。而遮挡关系在相邻帧之间几乎不变 —— AI 那边早就为此加了
/// `OCCLUSION_REFRESH` 缓存（该节曾占 `ai_us` 的 18–39%），渲染这条路一直没有。
///
/// 取 4（≈40ms @100fps）：这是**画面语义**，刷新窗口必须小到看不出；分摊刷新（每个 NPC
/// 落在固定槽位）保证任何时刻只有 1/4 的 NPC 数据是旧的，而不是整场同时跳变。
/// **把 N 设成 1 = 每帧全量重算**（只剩"两条调用路共用一份结果"的去重收益、零额外延迟）
/// —— 想用最保守的语义时改这一个常量即可。
const NPC_VIS_REFRESH_FRAMES: u32 = 4;








/// 🔴 **伤害来源**（2026-09-15）——kill feed 此前只报"谁死了"，从不报"谁杀的"。
///
/// 缺口不在渲染层而在**结算层**：`damage_npc(idx, dmg)` 根本不携带来源，
/// 所以 feed 只能写成"击杀 蓝方 #174"。全仓生产代码只有 2 处调它
/// （玩家弹命中 / 爆炸 AoE），改造面很小，于是显式加参数而不是用
/// "记住上一发是谁打的"这类隐式状态（那种写法会在多来源同帧交错时静默归错人）。
///
/// ⚠ **只列真正会走到这里的来源**：NPC 互射与联机击杀走的是各自的路径
/// （`apply_npc_combat` / 网络命中），它们本来就手上有击杀者，直接用 `kill_line` 拼。
/// 为"接上变体"而把枚举塞进那些路径，只会多出永不构造的变体（本仓 0 警告是红线）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DamageSource {
    /// 玩家（第一人称射击）
    Player,
    /// 无归属：爆炸/冲击波 —— 同一发 AoE 会同时结算多个目标，
    /// 报"某个人杀的"会是编的。爆炸单独成句（见 `blast_kill_line`）。
    Blast,
}

/// feed 文本里"击杀者"的两种形态（拼装只在 `kill_line` 一处，避免三处格式漂移）。
/// ⚠ **没有"无归属"这一档** —— 爆炸走 `blast_kill_line` 自己成句，
/// 不经过这里（加了就会是个永不构造的变体）。
pub(crate) enum KillerLabel {
    /// 玩家自己
    You,
    /// 有名字/编号的主体（如"红方 #12"）
    Named(String),
}



/// `RV3D_AI_DIAG` 的逐 NPC 节流桶宽（秒）。
const AIDIAG_BUCKET_SECS: f32 = 5.0;


impl Game {





































































































































}




/// NPC 身体半径（米）：与 `resolve_circle_obstacles` 的推开半径、`world_to_grid` 的
/// 「身体所在格」判据共用同一个值（别在别处再写 0.45）。
const NPC_BODY_RADIUS: f32 = 0.45;



/// 网络远端玩家快照 id 基址（与 NPC id 空间隔离：100000+）
/// 快照里远端玩家的 id 区（`crate::net::NET_PLAYER_BASE` 的同源定义；改一处必须改两处
/// —— 现在两处是同一个常量，见下面的 `use`）。
const NET_PLAYER_BASE: u32 = crate::net::NET_PLAYER_BASE;



#[cfg(test)] mod tests;

// 子模块（见 docs/refactor-plan.md）
mod types;

// 子模块（见 docs/refactor-plan.md）
mod net;

// 子模块（见 docs/refactor-plan.md）
mod session;

// 子模块（见 docs/refactor-plan.md）
mod weapons;

// 子模块（见 docs/refactor-plan.md）
mod npc_ai;

// 子模块（见 docs/refactor-plan.md）
mod projectiles;

// 子模块（见 docs/refactor-plan.md）
mod waves;

// 子模块（见 docs/refactor-plan.md）
mod collisions;

// 子模块（见 docs/refactor-plan.md）
mod player;

// 子模块（见 docs/refactor-plan.md）
mod ai_util;

// 子模块（见 docs/refactor-plan.md）
mod map_util;

// 子模块（见 docs/refactor-plan.md）
mod diag;

