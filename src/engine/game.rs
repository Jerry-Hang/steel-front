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
use super::window::{WINDOW_HEIGHT, WINDOW_WIDTH};
use super::weapon_data::{build_firearm, ALL_WEAPONS};
use super::weapons::{Grenade, Projectile, WeaponRack, GRENADE_FUSE_MAX, GRENADE_FUSE_MIN, GRENADE_SPEED};
use crate::ui::HudState;

/// 阵营显示名（**只给 kill feed 用**，2026-09-15 中文化）
///
/// 改前是 `"RED"` / `"BLUE"`。全仓只有 `push_kill` 的两处调用在用它
/// （**不在日志、不在网络协议里** —— 这一点在改之前用 `rg 'team_name\('` 确认过），
/// 所以这里的字符串只影响玩家看到的那一行。
///
/// **为什么中文化是低风险的**：`ui.rs::render_text` 早就有 CJK 分支
/// （`is_cjk(ch)` → `engine::font_cjk` 的 12×12 点阵），而 HUD 的
/// `"OBJECTIVE 歼灭敌人 {}/{}"` 一直就是这么渲染的 ⇒ **路径是通的，缺的只是字符串**。
fn team_name(t: Team) -> &'static str {
    match t {
        Team::Red => "红方",
        Team::Blue => "蓝方",
    }
}

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


/// 由**射速**派生出这把武器支持的档位表。用户要求"不同武器支持不同档位"，
/// 但**不逐武器手写 35 份表** —— 手写表就是 35 个将来会分叉的地方，而本仓最贵的
/// 一类 bug 正是"同一个量有两套来源"。派生规则的阈值取自真实连发扳机的分布：
///
/// - `rpm < 200`：栓动狙击 / 泵动霰弹 —— **只有单发**，这类枪没有点射档
/// - `200 ≤ rpm < 550`：半自动步枪 / 精确射手 —— 单发 + 双发 + 三连发
/// - `rpm ≥ 550`：突击步枪 / 冲锋枪 —— 四档全给
pub fn fire_modes_for(rpm: f32) -> &'static [FireMode] {
    const SEMI: &[FireMode] = &[FireMode::Semi];
    const BURST: &[FireMode] = &[FireMode::Semi, FireMode::Burst2, FireMode::Burst3];
    const FULL: &[FireMode] = &[FireMode::Semi, FireMode::Burst2, FireMode::Burst3, FireMode::Auto];
    // 用 `!(rpm >= 200.0)` 而不是 `rpm < 200.0`，这样 NaN 落进最保守的"只有单发"
    if !(rpm >= 200.0) {
        SEMI
    } else if rpm < 550.0 {
        BURST
    } else {
        FULL
    }
}

/// 在**支持的档位集合**里取下一个，跳过不支持的。抽成纯函数是为了能直接测
/// "栓动狙击按 B 键不会切到连发"—— 否则得先构造一把特定武器才能覆盖到。
/// 一圈都找不到就原样返回（支持集合为空或只有当前档时不该崩也不该乱跳）。
pub fn next_supported_fire_mode(current: FireMode, supported: &[FireMode]) -> FireMode {
    let mut m = current;
    for _ in 0..4 {
        m = m.next();
        if supported.contains(&m) {
            return m;
        }
    }
    current
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

/// 把一个障碍盒"够格"的格子标成阻挡，返回**新封的格数**。这是导航网格的**唯一建网规则**。
///
/// 判据 = 障碍 AABB 与该格（`GRID_CELL` 见方）的**重叠面积** ≥ [`CELL_BLOCK_MIN_OVERLAP_M2`]。
/// 逐格算 AABB∩格 的精确面积（不是"包围盒范围全封"），因为 4m 的格子对这个 1:1 的世界太粗：
/// 一件 6×0.9m 的中央隔离墩用"碰到就封"会封掉 3×2 = 6 格（96m²，实际占地 5.4m²），
/// 0.34m 的护柱封掉整整一格（16m²）—— 于是**几何上并不相连的装饰件在网格里连成一道墙**。
///
/// 实测后果（`reachable_mask` + 临时探针，数字见 `docs/PROGRESS.md` 2026-09-25 节）：
/// 程序化城市地图上玩家出生的十字路口被隔离墩/护柱/树/消防栓围成 **24 格的孤岛**
/// （全图可通行 13165 格），出生环 64 个采样点**一个都到不了玩家**；defense_line 的
/// 沙袋环同理（玩家所在连通域只剩 **4 格**）⇒ NPC 出生即在另一个连通域 ⇒ 永远走不到玩家 ⇒
/// `update_waves` 等不到 `npcs.is_empty()` ⇒ **波次永远清不掉**（未结案 #17 的真根因）。
///
/// 调用方：`apply_level`（生产）与单测的建网；**不许再写第二套循环**。
fn block_obstacle_cells(grid: &mut GridMap, ob: &MapObstacle) -> usize {
    let g0 = world_to_grid(ob.x - ob.half_w, ob.z - ob.half_d);
    let g1 = world_to_grid(ob.x + ob.half_w, ob.z + ob.half_d);
    let (hx, hz) = (GRID_CELL * 0.5, GRID_CELL * 0.5);
    // 长件（沙袋/矮墙/围墙/建筑）保守封格：NPC 必须绕着走，见 `CELL_BLOCK_LONG_EXTENT_M`
    let long = ob.half_w.max(ob.half_d) * 2.0 >= CELL_BLOCK_LONG_EXTENT_M;
    let mut newly_blocked = 0usize;
    for gx in g0.x..=g1.x {
        for gz in g0.y..=g1.y {
            let pos = GridPos::new(gx, gz);
            if !grid.in_bounds(pos) {
                continue;
            }
            let (cx, cz) = grid_to_world(pos);
            // AABB ∩ 格的逐轴重叠长度（≤0 表示该轴不重叠）；障碍比格宽时按格宽封顶
            let ox = (hx + ob.half_w - (ob.x - cx).abs()).min(GRID_CELL);
            let oz = (hz + ob.half_d - (ob.z - cz).abs()).min(GRID_CELL);
            if ox <= 0.0 || oz <= 0.0 {
                continue;
            }
            if !long && ox * oz < CELL_BLOCK_MIN_OVERLAP_M2 {
                continue;
            }
            if grid.is_passable(pos) {
                grid.block(pos);
                newly_blocked += 1;
            }
        }
    }
    newly_blocked
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
    /// 树木：场景装饰（安全环外），细长圆柱+树冠方块，可碰撞可击穿
    Tree,
    /// 建筑物：场景装饰（安全环外），大尺寸方块，可碰撞高耐久
    Building,
    /// 残骸：场景装饰（安全环外），矮小散落方块群
    Ruin,
}

/// 障碍基础血量：按种类区分（墙 150 / 大块 300 / 路障 100）。
/// M1 步枪单发 25 伤害 → 6/12/4 发击穿；hp 归 0 即摧毁，从碰撞/阻挡/渲染中移除。
fn obstacle_max_hp(kind: ObstacleKind) -> f32 {
    match kind {
        ObstacleKind::Wall => 150.0,
        ObstacleKind::Block => 300.0,
        ObstacleKind::Barrier => 100.0,
        ObstacleKind::Tree => 60.0,
        ObstacleKind::Building => 500.0,
        ObstacleKind::Ruin => 120.0,
    }
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
        ObstacleKind::Tree => KindStyle {
            min_boxes: 1,
            max_boxes: 1,
            // 树干半宽收窄到 0.15..0.28（整宽 0.3..0.56m）：物理刚体与渲染 marker 同源
            // （同一 half_w/half_d），视觉/碰撞一致收窄 → 玩家可贴近树干（半径 0.5m 外即可绕行），
            // 不再有“看着细、撞着粗”的路障感。RNG 消耗顺序不变 → 摆放位置不变。
            min_w: 0.15,
            max_w: 0.28,
            min_d: 0.15,
            max_d: 0.28,
            gap: 1.5,
        },
        ObstacleKind::Building => KindStyle {
            min_boxes: 1,
            max_boxes: 1,
            min_w: 6.0,
            max_w: 9.0,
            min_d: 5.0,
            max_d: 7.0,
            gap: 2.0,
        },
        ObstacleKind::Ruin => KindStyle {
            min_boxes: 2,
            max_boxes: 4,
            min_w: 0.8,
            max_w: 2.0,
            min_d: 0.6,
            max_d: 1.5,
            gap: 0.8,
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
            let mut ob = MapObstacle::new(theme.kind, cx + tx * off, cz + tz * off, half_w, half_d);
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

    // ---- 场景装饰（安全环外）：树木 / 建筑物 / 残骸，丰富战场外观 ----
    // 装饰放在 ring_outer 之外 20-60m 的环形带，避开战斗区（冒烟站定/弹道不受影响）；
    // 数量 10-16 个，确定性 LCG 派生；参与碰撞（可击穿，耐久见 obstacle_max_hp）。
    let deco_kinds = [ObstacleKind::Tree, ObstacleKind::Building, ObstacleKind::Ruin];
    let deco_count = 10 + (map_lcg_unit(&mut state) * 6.0) as usize;
    for _ in 0..deco_count {
        let kind = deco_kinds[((map_lcg_unit(&mut state) * 3.0) as usize) % 3];
        let style = kind_style(kind);
        let n_boxes = style.min_boxes as usize
            + (map_lcg_unit(&mut state) * (style.max_boxes - style.min_boxes + 1) as f32) as usize;
        let angle = map_lcg_unit(&mut state) * tau;
        let (dirx, dirz) = (angle.cos(), angle.sin());
        let half_w = style.min_w + map_lcg_unit(&mut state) * (style.max_w - style.min_w);
        let half_d = style.min_d + map_lcg_unit(&mut state) * (style.max_d - style.min_d);
        let dist = theme.ring_outer + 20.0 + map_lcg_unit(&mut state) * 40.0;
        let cx = dirx * dist;
        let cz = dirz * dist;
        let (tx, tz) = (-dirz, dirx);
        for i in 0..n_boxes {
            let off = (i as f32 - (n_boxes as f32 - 1.0) * 0.5) * (half_w * 2.0 + style.gap);
            let ob = MapObstacle::new(kind, cx + tx * off, cz + tz * off, half_w, half_d);
            let mut placed = true;
            for o in &obstacles {
                if (ob.x - o.x).abs() < ob.half_w + o.half_w
                    && (ob.z - o.z).abs() < ob.half_d + o.half_d
                {
                    placed = false;
                    break;
                }
            }
            if placed {
                obstacles.push(ob);
            }
        }
    }
    LevelMap { obstacles, decor: Vec::new(), props: Vec::new() }
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

/// `RV3D_AI_DIAG=1`：NPC 停在非 Attack 态时每 5s 打一行"为什么"（未结案 #17 的定位工具，
/// 见 `step_npc` 里的调用点；默认关 ⇒ 生产行为与日志量不变）。
fn ai_diag() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("RV3D_AI_DIAG").is_ok_and(|v| v == "1" || v == "on" || v == "true")
    })
}

/// `RV3D_NPC_POS=1`：每秒给**每只** NPC 打一行机器可读位置 `npcpos: #id x y z state`，
/// 供注入 harness 跟踪**活靶**（`npc: #N stand` 只在进 Attack 那一刻打一次，移动靶全程打空）。
/// 与 `RV3D_AI_DIAG` 分开：harness 只要位置，不需要那一堆 AI 归因统计。
fn npc_pos_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("RV3D_NPC_POS").is_ok_and(|v| v == "1" || v == "on" || v == "true")
    })
}

/// 换弹动作包络（纯函数，可单测）：`p` = 换弹进度 0→1（刚按下 → 完成），返回 `[0, 1]`，
/// **两端位移为 0、中点为 1**。
///
/// 与切枪用同一套 `sin(π·p)`：**两端取 0** ⇒ 枪不会在换弹开始/结束那一帧"啪"地跳一下
/// （本仓枪模历史上的残影/抖动都出自位置跳变）。
/// ⚠️ 这条包络**两端速度最大**（导数 = π·cos(πp)，在 p=0/1 处为 ±π），所以它是
/// "很快沉下去、再抬回来"，而不是缓入缓出 —— 想要缓入缓出得用 `sin²` 或 smoothstep，
/// 但那会让中段变慢、看起来像卡顿。**别把这条写成"速度也连续"。**
/// `main.rs` 传的是 `1 - hud.reload_progress`（该进度是 1→0 递减的）。
pub fn reload_envelope(p: f32) -> f32 {
    (std::f32::consts::PI * p.clamp(0.0, 1.0)).sin()
}

/// `RV3D_CULL_DIAG=1`：把「NPC 遮挡剔除」的 CPU 成本按秒量出来（默认关；关着时每个调用
/// 只多一次已缓存的 bool 比较）。
///
/// **为什么要这把尺子**：`npc_occluded` 每帧对**每个** NPC 做 2 条线段 × `world.bodies`
/// 的 AABB 扫描（城市图 1240 个障碍）⇒ 255 人一帧约 63 万次相交测试，而 `main.rs` 里
/// 它被**调两遍**（上屏列表 + 枪口焰筛选）⇒ 一帧约 126 万次。这个数量级**不许靠推理**：
/// 先量出来（教训 20 / 25），再决定要不要缓存或换宽相。
pub fn cull_diag_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("RV3D_CULL_DIAG").is_ok_and(|v| v == "1" || v == "on" || v == "true")
    })
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

/// `npcpos:` 的发送周期（秒）—— 纯函数，可单测。
///
/// `RV3D_NPC_POS_HZ`（默认 1）决定频率，夹在 `[1, 30]`：小于 1 没意义，大于 30 只会刷屏。
/// 🔴 2026-09-25 加：注入 harness 打移动靶时，位置样本的**年龄**直接决定命中率 ——
/// 1 Hz 意味着它可能瞄一个 1 秒前的位置（NPC 4–5 m/s ⇒ 差出好几米）。
/// 这条旋钮让 harness 能按需取到 ~10 Hz 的新鲜位置，而默认仍是原来的 1 Hz（不改变既有日志量）。
fn npc_pos_period(hz_env: Option<u32>) -> f32 {
    let hz = hz_env.unwrap_or(1).clamp(1, 30);
    1.0 / hz as f32
}

/// `RV3D_NPC_POS_HZ`（只读一次；非法值视为缺省 = 1 Hz）。
fn npc_pos_hz() -> Option<u32> {
    static HZ: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *HZ.get_or_init(|| std::env::var("RV3D_NPC_POS_HZ").ok().and_then(|v| v.parse::<u32>().ok()))
}

/// 诊断通道的刷新周期（秒）：`RV3D_NPC_POS=1` 时按 `RV3D_NPC_POS_HZ`，否则保持 1 Hz。
///
/// 🔴 2026-09-25：`main.rs` 的 `cam:` 行（yaw/pitch）**也**要用它。这是当天最重要的一个发现：
/// 注入 harness 的瞄准环是拿 `cam:` 行做**回读**的（注入一像素 → 读回当前角度 → 再算误差），
/// 而 `cam:` 原本 1 Hz，瞄准环每轮只 sleep 0.5s ⇒ **常常读到同一行**（旧角度）⇒ 把同一个
/// 修正量**再注入一次** ⇒ 过冲 / 假装收敛。弹道埋点正是这么露的马脚：过期弹 117/118
/// 差最近的人 **2m 以上**（12m 交火距离上那是 ~10° 的偏差，只可能是回读失灵）。
/// 判据：同频后 `RV3D_PROJ_DIAG` 的 `>2m` 桶必须显著下降（PROGRESS §21.22）。
pub fn diagnostic_period_secs() -> f32 {
    if npc_pos_log() {
        npc_pos_period(npc_pos_hz())
    } else {
        1.0
    }
}

/// `RV3D_PROJ_DIAG=1`：弹道诊断通道（弹丸去向 / 过期分桶 / 每枪瞄得准不准）。
fn proj_diag_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RV3D_PROJ_DIAG").as_deref() == Ok("1"))
}

/// step_npc 解析"本 NPC 这一帧的目标位置"的唯一规则。
/// 遮挡预计算必须与决策走同一条分支，否则会出现"按玩家算遮挡、按 NPC 行动"的错位。
fn resolve_ai_target(
    index: usize,
    stress: bool,
    spectator: bool,
    targets: &[Option<(usize, [f32; 3], f32)>],
    fallback_targets: &[[f32; 3]],
    player: &glam::Vec3,
) -> glam::Vec3 {
    match (stress, targets.get(index).copied().flatten()) {
        (true, Some((_, tp, _))) => glam::Vec3::new(tp[0], 0.0, tp[2]),
        (true, None) if spectator => {
            let f = fallback_targets.get(index).copied().unwrap_or([0.0, 0.0, 0.0]);
            glam::Vec3::new(f[0], 0.0, f[2])
        }
        _ => *player,
    }
}

/// 逐 NPC 计算"目标是否被障碍挡住"。
///
/// 采样规则与渲染剔除 [`Game::npc_occluded`] 严格一致：躯干中心与头/肩两条线段**全部**
/// 被挡才算挡住——低掩体后露出上半身的士兵仍然可见，避免"我能看到他却打不到我"这种
/// 新的不一致。
///
/// 成本：每 NPC 2 条线段，先用线段 XY 包围盒粗筛再跑 slab 测试。全城障碍约 1100 件时，
/// 粗筛把每次查询压到几十个候选盒，远好于对 `world.bodies` 做全量线性扫描。
fn target_occlusion(
    npcs: &[Npc],
    bodies: &[Body],
    stress: bool,
    spectator: bool,
    targets: &[Option<(usize, [f32; 3], f32)>],
    fallback_targets: &[[f32; 3]],
    player: &glam::Vec3,
) -> Vec<bool> {
    let samples = [NPC_HIT_CENTER_Y, 1.7];
    // 成本结构（第 50 轮实测本节占 `ai_us` 的 18–39%，即 240–520µs）：
    // 对**每一个 NPC × 2 个采样**都遍历**全部 bodies**（约 1100 件）——
    // 255 × 2 × 1100 ≈ **56 万次迭代/帧**。注释里说的"把候选盒压到几十个"
    // 是**粗筛之后**的收益，**粗筛本身仍是 O(bodies)**。
    //
    // 🔴 第 52 轮试过并把 body AABB 摊平成一维数组（想省掉 `aabb()` 构造与间接寻址）——
    // **实测无收益**（238µs vs 基线 252µs，在噪声内），却每帧多一次 1100 元素分配。
    // **已回退**：成本不在访问方式，而在**迭代次数本身**。
    // ⇒ 真正的修法是**算法级**的：给 bodies 建粗相位空间索引，把每 NPC 的候选盒
    //    从"全部 1100 件"降到"视野段覆盖的几个格子"，而不是继续在常数因子上抠。
    npcs.iter().enumerate()
        .map(|(index, npc)| {
            let t = resolve_ai_target(index, stress, spectator, targets, fallback_targets, player);
            let (ax, ay, az) = (npc.position[0], npc.position[1] + 1.4, npc.position[2]);
            // 线段包围盒粗筛（x/z/y 三轴都裁一遍，把候选盒压到几十个）
            let (minx, maxx) = (ax.min(t.x), ax.max(t.x));
            let (minz, maxz) = (az.min(t.z), az.max(t.z));
            let mut all_blocked = true;
            for h in samples {
                let ty = t.y + h;
                let (tmin, tmax) = (ay.min(ty), ay.max(ty));
                let blocked = bodies.iter().any(|body| {
                    let a = body.aabb();
                    if a.max.x < minx || a.min.x > maxx || a.max.z < minz || a.min.z > maxz
                        || a.max.y < tmin || a.min.y > tmax
                    {
                        return false;
                    }
                    Game::segment_hits_aabb(ax, ay, az, t.x, ty, t.z, &a)
                });
                if !blocked {
                    all_blocked = false;
                    break;
                }
            }
            all_blocked
        })
        .collect()
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
enum DamageSource {
    /// 玩家（第一人称射击）
    Player,
    /// 无归属：爆炸/冲击波 —— 同一发 AoE 会同时结算多个目标，
    /// 报"某个人杀的"会是编的。爆炸单独成句（见 `blast_kill_line`）。
    Blast,
}

/// feed 文本里"击杀者"的两种形态（拼装只在 `kill_line` 一处，避免三处格式漂移）。
/// ⚠ **没有"无归属"这一档** —— 爆炸走 `blast_kill_line` 自己成句，
/// 不经过这里（加了就会是个永不构造的变体）。
enum KillerLabel {
    /// 玩家自己
    You,
    /// 有名字/编号的主体（如"红方 #12"）
    Named(String),
}

/// feed 里一行"某人击杀某人"的文本（`cause` 是武器/爆炸说明，可空）。
/// **三处调用点（`damage_npc` / NPC 互射 / 联机）共用它** ——
/// 免得三处各写一遍格式、越改越不一致（此前正是三处各写一遍）。
fn kill_line(killer: KillerLabel, victim: &str, cause: &str) -> String {
    match killer {
        KillerLabel::You => format!("你击杀了{victim}{cause}"),
        KillerLabel::Named(name) => format!("{name} 击杀了{victim}{cause}"),
    }
}

/// 爆炸击杀单独成句：**不冒充有击杀者**。
fn blast_kill_line(center: [f32; 3], victim: &str) -> String {
    format!("爆炸（{:.0},{:.0}）击杀了{victim}", center[0], center[2])
}

/// `RV3D_AI_DIAG` 的逐 NPC 节流桶宽（秒）。
const AIDIAG_BUCKET_SECS: f32 = 5.0;

/// 判断"这只 NPC 本帧该不该打一行 `aidiag`"，并把节流状态写回**它自己的**槽位。
///
/// 存在理由（2026-10-02）：旧实现是 `step_npc` 里的一个 `static SEEN: AtomicU64`，
/// 只记"上一个打印过的 key"。键里**确实**含 id，但全局只有一个槽 ⇒
/// 两只以上卡住的 NPC 逐帧交替时，每次都"与上次不同"，判据恒真，
/// 于是每帧两只各打一行（§20.6 实测 300s 刷 16.4 万行，
/// 而那行注释一直声称"每个卡住的 NPC 每 5s 一行"）。
/// ⇒ **诊断工具在它最该工作的场景（多只同时卡住）里把自己刷成了噪声。**
///
/// 状态改为随 `Npc` 携带：`step_npc` 对每个 `npc` 是 `&mut` 独占
/// （`par_for_each_mut` 切片）⇒ **不需要 atomic**。
/// 判据：`aidiag_throttle_is_per_npc`（正）与 `aidiag_single_shared_slot_would_flood`
/// （把旧实现的错误行为钉成特征测试，见其注释）。
fn aidiag_due(last_key: &mut u64, time: f32, id: usize) -> bool {
    let bucket = (time / AIDIAG_BUCKET_SECS) as u64;
    let key = (bucket << 32) | (id as u64 & 0xFFFF_FFFF);
    if *last_key == key {
        return false;
    }
    *last_key = key;
    true
}

impl Game {













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
            pt_enable: self.hud.pt_enable,
            pt_exposure: self.hud.pt_exposure,
            quality: self.hud.quality_index as u32,
        }
    }
























    /// 尝试开火（受射速冷却限制）。`origin`/`direction` 来自相机；返回是否真的开火。
    /// 内部单发：不做冷却检查，直接扣弹发射并结算后坐（含连发热量压制）/音效/日志。
    /// 返回是否发射成功。冷却与连发节奏由 fire / fire_burst 控制。
    fn fire_shot(&mut self, origin: [f32; 3], direction: [f32; 3], from_player: bool) -> bool {
        let active_spec = ALL_WEAPONS
            .iter()
            .find(|s| s.name_zh == self.weapons.active_name());
        // V3.0 散射密度（MOA → 弧度）：半角半径 × spread_scale（开镜缩小散布）。
        // 均匀矩形扰动 ±half；霰弹全部 8 弹丸共用同一散布锥。
        let moa_half = active_spec
            .map(|s| s.spread_moa * 0.000_291_f32 * 0.5 * self.spread_scale)
            .unwrap_or(0.0);
        let fire_dir = if moa_half > 0.0 {
            let seed = self.shots.wrapping_mul(2_654_435_761) + 17;
            let r1 = ((seed % 10_000) as f32 / 10_000.0) - 0.5;
            let r2 = (((seed.wrapping_mul(4_052_877)) % 10_000) as f32 / 10_000.0) - 0.5;
            Self::spread_direction(direction, r1 * 2.0 * moa_half, r2 * 2.0 * moa_half)
        } else {
            direction
        };
        match self.weapons.active_firearm().try_fire(origin, fire_dir) {
            Some(mut projectile) => {
                if from_player {
                    projectile = projectile.with_player();
                }
                // 连发热量：连续射击累积（0..1），压制后续枪口上扬——前几发上扬大，
                // 连发后期上扬趋于稳定（模拟控枪）；停火后 update 中按速率衰减
                let heat = self.auto_heat;
                self.auto_heat = (self.auto_heat + 0.22).min(1.0);
                let recoil_comp = 1.0 - heat * 0.55;
                let (kick_pitch, kick_yaw) = self.weapons.active_firearm_ref().current_kick();
                self.pending_kick.0 += kick_pitch * recoil_comp;
                self.pending_kick.1 += kick_yaw * recoil_comp;
                self.projectiles.push(projectile);
                // 霰弹：每发 8 弹丸，只消耗 1 发弹药；其余弹丸带确定性锥形散布（±2.8°）
                if let Some(spec) = active_spec {
                    if spec.pellets > 1 {
                        let weapon = self.weapons.active_firearm_ref().weapon_ref();
                        for k in 1..spec.pellets {
                            let seed = self.shots * 31 + (k as u64) * 7;
                            let r1 = ((seed * 2_654_435_761) % 10_000) as f32 / 10_000.0;
                            let r2 = ((seed * 4_052_877 % 10_000) + 1) as f32 / 10_000.0;
                            let dir = Self::spread_direction(
                                fire_dir,
                                (r1 - 0.5) * 2.0 * moa_half.max(0.000_5),
                                (r2 - 0.5) * 2.0 * moa_half.max(0.000_5),
                            );
                            let mut sp = weapon.fire(origin, dir);
                            if from_player {
                                sp = sp.with_player();
                            }
                            self.projectiles.push(sp);
                        }
                    }
                }
                self.shots += 1;
                // 🔴 2026-09-25：**这一枪瞄得准不准**（`RV3D_PROJ_DIAG=1`）——与角度最小的
                // NPC 胸口之间的夹角。这是"空放"归因的最后一格：夹角普遍很小 ⇒ 子弹在飞、
                // 是**命中体/时序**问题；夹角普遍很大 ⇒ 目标根本不在准星附近（harness 侧）。
                if from_player && proj_diag_on() {
                    let mut best = (f32::MAX, -1i32, 0.0f32);
                    for n in &self.npcs {
                        let v = [
                            n.position[0] - origin[0],
                            n.position[1] + 1.2 - origin[1],
                            n.position[2] - origin[2],
                        ];
                        let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                        if d < 1e-3 {
                            continue;
                        }
                        let cos = (v[0] * fire_dir[0] + v[1] * fire_dir[1] + v[2] * fire_dir[2]) / d;
                        let ang = cos.clamp(-1.0, 1.0).acos().to_degrees();
                        if ang < best.0 {
                            best = (ang, n.id as i32, d);
                        }
                    }
                    if best.1 >= 0 {
                        log::info!(
                            "shot-aim: npc=#{} ang={:.1}deg dist={:.0}m",
                            best.1,
                            best.0,
                            best.2
                        );
                    }
                }
                // 程序化枪声：按武器类别选音色（步枪/冲锋/狙击/机枪/霰弹/手枪），
                // 带确定性音量抖动（0.95..=1.0）避免机械重复
                let shot_scale = 0.95 + 0.05 * ((self.shots % 5) as f32 / 4.0);
                let shot_params = match active_spec.map(|s| s.sound) {
                    Some(crate::engine::weapon_data::SoundKind::Smg) => crate::audio::THOMPSON_SHOT,
                    Some(crate::engine::weapon_data::SoundKind::Sniper) => crate::audio::SNIPER_SHOT,
                    Some(crate::engine::weapon_data::SoundKind::Lmg) => crate::audio::LMG_SHOT,
                    Some(crate::engine::weapon_data::SoundKind::Shotgun) => crate::audio::SHOTGUN_SHOT,
                    Some(crate::engine::weapon_data::SoundKind::Pistol) => crate::audio::PISTOL_SHOT,
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

    /// 单发开火（受冷却与切枪计时约束）；返回是否发射。
    /// 内部路径（AI/网络/测试）发射的弹不带玩家标记（无友军伤害豁免）。
    pub fn fire(&mut self, origin: [f32; 3], direction: [f32; 3]) -> bool {
        if self.fire_cooldown > 0.0 {
            return false;
        }
        // 切枪计时中禁止开火（纯计时器，无动画）
        if self.weapons.is_switching() {
            return false;
        }
        let ok = self.fire_shot(origin, direction, false);
        if ok {
            self.fire_cooldown = self.weapons.active_firearm_ref().fire_interval();
        }
        ok
    }

    /// 玩家单发开火（main.rs 调用）：弹带玩家标记，命中友军（同阵营 NPC）穿身而过不造成伤害
    pub fn fire_player(&mut self, origin: [f32; 3], direction: [f32; 3]) -> bool {
        if self.fire_cooldown > 0.0 {
            return false;
        }
        if self.weapons.is_switching() {
            return false;
        }
        let ok = self.fire_shot(origin, direction, true);
        if ok {
            self.fire_cooldown = self.weapons.active_firearm_ref().fire_interval();
        }
        ok
    }

    /// 连发（Burst2/Burst3 共用这一条路径）：无视冷却快速连打 `rounds` 发，
    /// 之后强制冷却 `rounds ×` 间隔；返回实际发射数（弹匣打空即停）。
    /// 切枪计时中返回 0。`rounds` 由调用方给（`FireMode::burst_rounds()`）。
    /// `from_player` = true 时弹带玩家标记（友军豁免）；AI/网络侧传 false。
    ///
    /// 🔴 2026-09-23 复查：此前 `fire_burst` 与 `fire_burst_player` 是**两份逐行重复的函数体**
    /// （只差 `fire_shot(.., false)` / `fire_shot(.., true)`），而前者**没有任何调用方** ——
    /// 它靠一句 `#[allow(dead_code)]` 压着，旧注释还写着"AI/网络/测试用"（照它去找调用方会白找）。
    /// 现在合成一条路径 + 一个薄入口：重复没了，`from_player` 的语义写在签名上，
    /// 那条 `#[allow(dead_code)]` 也一并删掉（它能被删掉本身就是"没有重复"的证明）。
    pub fn fire_burst(
        &mut self,
        origin: [f32; 3],
        direction: [f32; 3],
        rounds: u32,
        from_player: bool,
    ) -> u32 {
        if self.weapons.is_switching() {
            return 0;
        }
        let mut n = 0u32;
        let rounds = rounds.max(1);
        for _ in 0..rounds {
            if self.fire_shot(origin, direction, from_player) {
                n += 1;
            } else {
                break;
            }
        }
        if n > 0 {
            self.fire_cooldown = self.weapons.active_firearm_ref().fire_interval() * rounds as f32;
        }
        n
    }

    /// 玩家三连发（main.rs 调用）：= [`Game::fire_burst`] 带玩家标记（友军豁免）
    pub fn fire_burst_player(&mut self, origin: [f32; 3], direction: [f32; 3], rounds: u32) -> u32 {
        self.fire_burst(origin, direction, rounds, true)
    }

    /// 设置散布缩放（main.rs 每帧按 ADS 混合更新：1.0 腰射 → 0.3 开镜）
    pub fn set_spread_scale(&mut self, scale: f32) {
        self.spread_scale = scale.clamp(0.1, 1.0);
    }

    /// 循环切换开火模式（B 键）：单发 → 三连发 → 连发
    /// X 键：开始打药。满血 / 没药 / 已在打药时**不消耗**（避免误按白扔一个包）。
    pub fn use_medkit(&mut self) {
        // 生命值放在 HudState（`hud.health` / `hud.max_health`），这是玩家血量的唯一来源 ——
        // 不要再往 Game 上加一个 hp 字段，那就是两套状态源。
        if self.heal_timer > 0.0 || self.medkits == 0 || self.hud.health >= self.hud.max_health {
            return;
        }
        self.medkits -= 1;
        self.heal_timer = HEAL_TIME;
        log::info!(
            "heal: 开始打药（剩余 {} 个，当前 hp={:.0}）",
            self.medkits,
            self.hud.health
        );
    }

    /// 打药进度 0..=1（未打药为 0；HUD 用它画进度）
    pub fn heal_progress(&self) -> f32 {
        if self.heal_timer <= 0.0 {
            0.0
        } else {
            (1.0 - self.heal_timer / HEAL_TIME).clamp(0.0, 1.0)
        }
    }

    /// 打药推进：计时归零**那一帧**一次性回血。
    /// 不做逐帧回血 —— 那样 HUD 没有明确的"完成"时刻，测试也不好断言。
    fn update_heal(&mut self, dt: f32) {
        if self.heal_timer <= 0.0 {
            return;
        }
        self.heal_timer -= dt;
        if self.heal_timer <= 0.0 {
            self.heal_timer = 0.0;
            let before = self.hud.health;
            self.hud.health = (before + HEAL_AMOUNT).min(self.hud.max_health);
            log::info!("heal: 完成 hp {:.0} -> {:.0}", before, self.hud.health);
        }
    }

    pub fn cycle_fire_mode(&mut self) {
        let modes = self.supported_fire_modes();
        self.fire_mode = next_supported_fire_mode(self.fire_mode(), modes);
        log::info!("weapons: 开火模式切换为 {}", self.fire_mode().label());
    }

    /// 当前开火模式
    /// 当前武器支持的档位表（由射速派生，见 `fire_modes_for`）
    pub fn supported_fire_modes(&self) -> &'static [FireMode] {
        let interval = self.weapons.active_firearm_ref().fire_interval();
        let rpm = if interval > 0.0 { 60.0 / interval } else { 0.0 };
        fire_modes_for(rpm)
    }

    /// 当前**生效**档位：始终夹进本武器支持的集合里。
    ///
    /// 故意做成"读的时候派生"而不是"切枪时重置一个字段"：后者需要一个切枪钩子，
    /// 一旦漏挂就会留下"拿着栓动狙击却还开着连发"的状态 —— 那正是两套状态源。
    /// 腰射准星扩散量 0..=1（HUD 按它缩放十字）。
    ///
    /// 2026-09-12 第⑤条：**这是本仓第一次给玩家"散布反馈"** ——
    /// 在此之前准星是固定 8px 半长，玩家读不出自己当前的散布状态，
    /// 而移动/姿态/连发恰恰是散布的三个主要来源。
    /// 三个输入量全都已经存在（`stance` / `sprinting()` / `fire_cooldown`），
    /// 所以这里不新增状态，只做一次纯函数式的合成。
    pub fn crosshair_spread(&self) -> f32 {
        // 站姿基准 0.30；蹲/趴收拢（更稳），冲刺张开（最不稳）
        let stance = match self.stance {
            Stance::Standing => 0.30,
            Stance::Crouching => 0.18,
            Stance::Prone => 0.10,
        };
        let sprint = if self.sprinting() { 0.25 } else { 0.0 };
        // 开火后坐期：fire_cooldown 是剩余秒数，按它线性张开，封顶 0.45
        let fire = (self.fire_cooldown * 3.0).clamp(0.0, 0.45);
        let base = (stance + sprint + fire).clamp(0.08, 1.0);
        // 🔴 2026-09-12 第⑤条补：**必须乘 `spread_scale`**。
        // `main.rs` 每帧按开镜混合度写它（`1.0 - ads_blend*0.7`，即开镜后弹道收拢 70%），
        // 而 `fire_dir` 的散射半角也乘了它。初版忘了这一项 ⇒
        // **玩家开镜后弹道已经收拢，准星却纹丝不动** —— 准星与实际散布不一致。
        (base * self.spread_scale).clamp(0.08, 1.0)
    }

    pub fn fire_mode(&self) -> FireMode {
        let modes = self.supported_fire_modes();
        if modes.contains(&self.fire_mode) {
            self.fire_mode
        } else {
            modes[0]
        }
    }

    /// 累计命中数（供 UI / 日志）
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// NPC 是否被障碍物完全遮挡：玩家眼位 → NPC 的采样点（身体中心 +1.0m 与头部 +1.7m）
    /// 的线段与任一障碍 AABB 相交。两处都被挡才算完全遮挡；任一处可见（如从矮墙
    /// 上方露出头/肩）即不遮挡——修复"隔墙透视"同时避免"半身可见却消失"。
    ///
    /// ⚠️ `RV3D_CULL_DIAG=1` 时这里会计时（见 `cull_diag_on`），用于**先量再改**：
    /// 一次调用最坏要扫 2 × `world.bodies.len()` 个 AABB，而 `main.rs` 每帧对全部 NPC
    /// 调它**两遍**。
    pub fn npc_occluded(&self, idx: usize) -> bool {
        if !cull_diag_on() {
            return self.npc_occluded_untimed(idx);
        }
        let t0 = std::time::Instant::now();
        let r = self.npc_occluded_untimed(idx);
        self.occl_us.set(self.occl_us.get() + t0.elapsed().as_micros() as u64);
        self.occl_calls.set(self.occl_calls.get() + 1);
        r
    }

    fn npc_occluded_untimed(&self, idx: usize) -> bool {
        let Some(n) = self.npcs.get(idx) else {
            return false;
        };
        let eye = self.cull_eye_override.unwrap_or_else(|| self.player_eye());
        for h in [NPC_HIT_CENTER_Y, 1.7] {
            let (ax, ay, az) = (eye.x, eye.y, eye.z);
            let (bx, by, bz) = (n.position[0], n.position[1] + h, n.position[2]);
            let blocked = self.world.bodies.iter().any(|body| {
                Self::segment_hits_aabb(ax, ay, az, bx, by, bz, &body.aabb())
            });
            if !blocked {
                return false;
            }
        }
        true
    }

    /// 刷新渲染用的「玩家看得见否」缓存（分摊刷新，见 `NPC_VIS_REFRESH_FRAMES`）。
    ///
    /// 每个 NPC 有一个固定槽位：只有 `(frame + i) % N == 0` 的那些这一帧才真做遮挡测试
    /// ⇒ 每帧重算 **1/N** 的 NPC，而每个 NPC 的判定最多旧 N−1 帧（≈40ms @100fps）。
    /// 分摊（而不是"每 N 帧全体重算一次"）是为了让过时数据**散在几个 NPC 上**，
    /// 不会整场一起跳变 —— 视觉上只是一两个士兵晚 40ms 才消失。
    ///
    /// 🔴 槽位里存的是 **`(npc.id, 可见)` 而不是单一个 bool**：NPC 死亡会把后面所有元素
    /// 的**下标前移**（`npcs.retain` / `swap_remove`），只按下标缓存 ⇒ 死一个人之后，
    /// 后面每个人的标志都变成了**前一个人的**（最多错 3 帧）。带上 id 就能一眼看出错位：
    /// id 不符 ⇒ 无论槽位轮没轮到，当帧立刻重算（这是"精确性"那一边，不受分摊限制）。
    ///
    /// 新出现的 NPC 一律**先按可见处理**（fail-open）：宁可多画一个也不让新兵隐形，
    /// 它的第一次刷新最迟 N−1 帧后到。
    pub fn refresh_npc_visibility(&mut self) {
        let n = self.npcs.len();
        if self.npc_vis.len() != n {
            self.npc_vis.resize(n, (usize::MAX, true));
        }
        let shift = self.npc_vis_frame % NPC_VIS_REFRESH_FRAMES;
        for i in 0..n {
            let due = (i as u32 + shift) % NPC_VIS_REFRESH_FRAMES == 0;
            // 下标错位（有人死了）：立刻重算，不等自己的槽位
            let shifted = self.npc_vis[i].0 != self.npcs[i].id;
            if !due && !shifted {
                continue;
            }
            self.npc_vis[i] = (self.npcs[i].id, !self.npc_occluded(i));
            self.npc_vis_scans.set(self.npc_vis_scans.get() + 1);
        }
        self.npc_vis_frame = self.npc_vis_frame.wrapping_add(1);
    }

    /// 上一帧 [`Game::refresh_npc_visibility`] 的结果（`true` = 玩家看得见）。
    /// 索引越界返回 `true`（fail-open，理由同上）。
    pub fn npc_visibility_flags(&self) -> Vec<bool> {
        // 每帧一次（255 个 bool）：把 `(id, flag)` 摊平成调用方要的形状。
        // 就地返回 `&[bool]` 需要第二份并行数组，收益不值得（一次 255 字节的拷贝 ≈ 几十纳秒）。
        self.npc_vis.iter().map(|(_, v)| *v).collect()
    }

    /// 取走本帧开火累计的后坐力（pitch/yaw 弧度），由 main.rs 施加到相机
    pub fn drain_kick(&mut self) -> (f32, f32) {
        let kick = self.pending_kick;
        self.pending_kick = (0.0, 0.0);
        kick
    }

    /// 是否处于开火后坐期（fire_cooldown > 0 = 刚开火，枪模后坐动画用）
    #[allow(dead_code)] // 历史接口：枪模后坐已改为一次性脉冲（main.rs 用 last_shot_at）
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
            self.player_body.pos.y + self.stance.eye_height(),
            self.player_body.pos.z,
        )
    }

    /// 转发 WASD 按键状态（FPS 玩家移动；仅 Playing + 第一人称生效）
    /// C 键：站立 ↔ 下蹲（卧倒时按下先回到站立）。日志落在姿态真正变化时。
    pub fn toggle_crouch(&mut self) {
        let next = if self.stance == Stance::Crouching {
            Stance::Standing
        } else {
            Stance::Crouching
        };
        self.stance = next;
        log::info!("stance: {:?} eye={:.2}m", next, next.eye_height());
    }

    /// Z 键：站立 ↔ 卧倒（下蹲时按下直接转卧倒）。
    pub fn toggle_prone(&mut self) {
        let next = if self.stance == Stance::Prone {
            Stance::Standing
        } else {
            Stance::Prone
        };
        self.stance = next;
        log::info!("stance: {:?} eye={:.2}m", next, next.eye_height());
    }

    /// 冲刺键（Shift）按住状态
    pub fn set_sprint(&mut self, on: bool) {
        self.sprint_held = on;
    }


    /// 此刻是否真的在冲刺：按住 Shift **且** 正在前进、非后退、站立、未开镜、在地面。
    /// HUD/视场角可以据此变化；速度倍率在 `move_first_person` 里消费。
    pub fn sprinting(&self) -> bool {
        self.sprint_held
            && self.move_forward
            && !self.move_backward
            && self.stance == Stance::Standing
            && !self.hud.ads
            && self.jump_vel == 0.0
    }

    pub fn set_movement(&mut self, forward: bool, backward: bool, left: bool, right: bool) {
        self.move_forward = forward;
        self.move_backward = backward;
        self.move_left = left;
        self.move_right = right;
    }

    /// 当前激活武器键名（weapon_data::WeaponSpec::key，第一人称枪模选择用）
    pub fn active_weapon_key(&self) -> &'static str {
        ALL_WEAPONS
            .get(self.weapons.active_index())
            .map(|s| s.key)
            .unwrap_or("hk416")
    }

    /// 诊断用：把 npc[0] 放到指定位置并固定（弹道隔离实验，RV3D_DIAG_NPC_FRONT）
    pub fn diag_place_npc(&mut self, pos: [f32; 3]) {
        if !self.npcs.is_empty() {
            self.npcs[0].position = pos;
            self.npcs[0].speed = 0.0;
            self.npcs[0].state_machine = NpcStateMachine::new();
        }
    }

    /// 本帧命中点列表（main.rs 生成命中火花后清空）
    pub fn take_hit_points(&mut self) -> Vec<[f32; 3]> {
        std::mem::take(&mut self.hit_points)
    }

    /// 本帧命中伤害值（与命中点一一对应；HUD 伤害飘字）
    pub fn take_hit_damages(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.hit_damages)
    }

    /// 弹着标记（弹孔）当前集合；`main.rs` 每帧读它生成渲染实例。
    pub fn impact_marks(&self) -> &[ImpactMark] {
        &self.impact_marks
    }

    /// 追加一枚弹着标记（`RV3D_NO_DECALS=1` 整体关闭，供同机位 A/B 当对照组）。
    fn push_impact_mark(&mut self, pos: [f32; 3], normal: [f32; 3]) {
        // 环境变量只解析一次：本函数虽然只在命中的那几帧被调用，
        // 但 `env::var` 带锁 + 扫环境表，没有必要每次付（与 WorldMarker::for_obstacle 同一处理）。
        static DISABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *DISABLED.get_or_init(|| std::env::var("RV3D_NO_DECALS").is_ok()) {
            return;
        }
        if self.impact_marks.len() >= IMPACT_MARK_MAX {
            self.impact_marks.remove(0);
        }
        self.impact_marks.push(ImpactMark {
            pos,
            normal,
            age: 0.0,
        });
    }

    /// NPC 受击闪白剩余强度（0..1；0 = 无闪白）。渲染侧按此混合白色 tint。
    pub fn npc_flash(&self, id: usize) -> f32 {
        self.npc_hit_flash
            .get(&id)
            .map(|t| (t / 0.15).clamp(0.0, 1.0))
            .unwrap_or(0.0)
    }

    /// 切换武器（数字键/命令窗口/滚轮）：切换到指定槽位；切枪计时中忽略重复切换。
    /// 越界输入优雅回退：记录日志并忽略，不 panic。
    pub fn switch_weapon(&mut self, index: usize) {
        // 槽位越界防御（WeaponRack::len 暴露武器数量）
        if index >= self.weapons.len() {
            log::warn!(
                "weapons: 切枪回退——槽位 {} 越界（共 {} 槽），忽略本次切换",
                index,
                self.weapons.len()
            );
            return;
        }
        let prev = self.weapons.active_index();
        self.weapons.switch_to(index);
        self.log_switch_if_changed(prev, "切枪");
    }

    /// 切枪日志（两条路共用）：**只在真的换成了才打**，且 `tag` 区分是哪条路。
    ///
    /// 🔴 2026-09-26 补：滚轮那条路（`cycle_weapon`）原来**绕过这里直接调 rack**，
    /// 于是滚轮切枪在日志里**完全隐形** —— 当天写 `scripts/run_weapon_probe.ps1` 时
    /// 就据此得出了"滚轮没生效"的结论，其实是我的尺子量不到（教训 27 的又一例：
    /// **量不到 ≠ 现象不存在**）。现在两条路都留痕且标签不同，探针能分别计数。
    fn log_switch_if_changed(&self, prev: usize, tag: &str) {
        let now = self.weapons.active_index();
        if now != prev {
            log::info!(
                "weapons: {} {} -> {} ({})",
                tag,
                prev,
                now,
                self.weapons.active_name()
            );
        }
    }

    /// 循环切换武器（滚轮向上 = 下一把，向下 = 上一把；末尾回到 0 / 开头回到末尾）
    pub fn cycle_weapon(&mut self, delta: i32) {        let prev = self.weapons.active_index();
        if delta > 0 {
            self.weapons.switch_next();
        } else if delta < 0 {
            self.weapons.switch_prev();
        }
        self.log_switch_if_changed(prev, "滚轮切枪");
    }

    /// 🔴 **切枪动画进度** 0..=1（0 = 刚换手、枪在最低点；1 = 抬回瞄准位）。
    ///
    /// 2026-09-15 加：切枪此前只有"禁止开火"的计时器，**没有任何动作** ——
    /// 按下数字键后枪是直接跳变的。枪模（`main.rs::fp_gun_matrix`）每帧读这个值
    /// 做下坠/侧转。取不到武器（空架）时返回 1.0（= 无位移），与"没在切枪"同义。
    pub fn weapon_switch_progress(&self) -> f32 {
        self.weapons.switch_progress()
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
        self.medkits = self.medkits_max;
        self.heal_timer = 0.0;
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
            // 班长（指挥体系）投掷倾向更高（15%），其余战士维持 8%
            let is_leader = self.command.as_ref().map(|cmd| match npc.team {
                Team::Red => cmd.0.is_leader(npc.id),
                Team::Blue => cmd.1.is_leader(npc.id),
            }).unwrap_or(false);
            let h = (npc.id as u64 * 31 + self.frame_no as u64 * 7) % 100;
            if h >= if is_leader { 15 } else { 8 } {
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
            // 🔴 出手点必须抬到离地高度：脚底出手 = 高帧率下第一帧就被判"落地" → 自爆
            // （机理与现场证据见 `NPC_GRENADE_RELEASE_Y` 定义处）
            let origin = [
                npc.position[0],
                npc.position[1] + NPC_GRENADE_RELEASE_Y,
                npc.position[2],
            ];
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
    /// 玩家跳跃请求（main.rs 按 Jump 绑定置位；落地后清除）
    pub fn jump_requested(&mut self, jump: bool) {
        self.jump_pressed = jump;
    }

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
        // 🔴 2026-09-13：**冲刺起跳保留水平动量**（用户要求："在奔跑的时候，跳的时候会有
        // 向前的力，会直接跟过去一样越过去"）。
        //
        // 原实现是纯"位置式"移动：每帧只按 `速度 × dt` 沿**当前输入方向**挪一下，
        // 不保存任何水平速度。于是起跳那一瞬间水平速度就没了 —— 而且空中还要再乘
        // `air_factor = 0.5`，结果是"原地直上直下"，冲刺跳跃完全过不去障碍。
        //
        // 现在：起跳瞬间把**当时的有效水平速度**存进 `jump_hvel`，空中把它按 `AIR_DRAG`
        // 衰减着继续推进（叠加在削弱后的空中控制之上），落地清零。
        // 这样冲刺跳能靠惯性跃过缺口，普通走跳几乎看不出来（速度本来就小）。
        if len > 1e-4 {
            // 空中控制衰减：跳跃中水平移动减半（真实物理——空中无法急转弯）
            let air_factor = if self.jump_vel != 0.0 { 0.5 } else { 1.0 };
            // 开镜减速：举枪瞄准移动 -35%（ADS 重量感；hud.ads 由 main.rs 每帧同步）
            let ads_factor = if self.hud.ads { 0.65 } else { 1.0 };
            let sprint_mul = if self.sprinting() { SPRINT_MUL } else { 1.0 };
            let step = (PLAYER_SPEED
                * self.stance.speed_mul()
                * sprint_mul
                * ads_factor
                * air_factor
                * dt)
                .min(0.5);
            // 惯性位移：与输入方向无关，纯粹把起跳时带走的速度延续下去
            let (ix, iz) = (self.jump_hvel.x * dt, self.jump_hvel.z * dt);
            let (mx, mz) = self
                .player_body
                .try_move(&self.world, dx / len * step + ix, dz / len * step + iz);
            let moved = (mx * mx + mz * mz).sqrt();
            if moved > 0.01 && self.time - self.footstep_timer >= FOOTSTEP_INTERVAL {
                self.footstep_timer = self.time;
                // 程序化脚步：短促宽带噪声，交替强弱（0.8 / 1.0）确定性变化
                let step_scale = 0.8 + 0.2 * ((self.time * 2.0) as u32 % 2) as f32;
                let pos = self.player_pos();
                self.audio.synth_mut().play_footstep(pos, step_scale);
            }
        } else if self.jump_hvel.length_squared() > 1e-6 {
            // 无输入但空中有惯性：仍然往前滑（松手也越得过去）
            let (ix, iz) = (self.jump_hvel.x * dt, self.jump_hvel.z * dt);
            let _ = self.player_body.try_move(&self.world, ix, iz);
        }
        // 惯性衰减（空中每帧），落地时由下面的贴地分支清零
        if self.jump_vel != 0.0 {
            let k = (1.0 - AIR_DRAG * dt).max(0.0);
            self.jump_hvel.x *= k;
            self.jump_hvel.z *= k;
        }
        // 垂直运动：跳跃（Jump 键按下且在地面 → 初速）+ 重力 + 落地贴地
        //
        // 🔴 2026-09-13：站立面 = **地形高度 与 障碍物顶面 的较大者**。
        // 配套改动在 `physics.rs`：`push_out_of_aabb` 现在带 Y 判据（站在盒子上方不再被
        // 水平推开），所以必须有东西接住玩家 —— 否则会直接穿进盒子里掉下去。
        // 这条接线同时给了"能站到集装箱/掩体/矮墙上"这个此前完全不存在的能力。
        let terrain = terrain_height_at(self.player_body.pos.x, self.player_body.pos.z);
        let support = self.player_body.support_height(&self.world, PLAYER_STEP_UP);
        let ground = terrain.max(support);
        let on_ground = self.player_body.pos.y <= ground + 0.05 && self.jump_vel <= 0.0;
        // 卧倒不能起跳（蹲姿可以）
        if self.jump_pressed && on_ground && self.stance != Stance::Prone {
            self.jump_vel = JUMP_SPEED;
            // 🔴 起飞瞬间把**当前有效水平速度**带进惯性（见上面 jump_hvel 的注释）。
            // 输入方向归一化：斜着冲刺不会比直着冲刺飞得更远。
            if len > 1e-4 {
                let sprint_mul = if self.sprinting() { SPRINT_MUL } else { 1.0 };
                let ads_factor = if self.hud.ads { 0.65 } else { 1.0 };
                let v = PLAYER_SPEED * self.stance.speed_mul() * sprint_mul * ads_factor;
                self.jump_hvel = glam::Vec3::new(dx / len * v, 0.0, dz / len * v);
            } else {
                self.jump_hvel = glam::Vec3::ZERO; // 原地起跳：没有惯性
            }
            let jpos = self.player_pos();
            self.audio.synth_mut().play_footstep(jpos, 1.0);
        }
        if self.jump_vel != 0.0 {
            self.jump_vel -= GRAVITY * dt;
            self.player_body.pos.y += self.jump_vel * dt;
            if self.player_body.pos.y <= ground {
                self.player_body.pos.y = ground;
                self.jump_vel = 0.0;
                self.jump_hvel = glam::Vec3::ZERO; // 落地：惯性结束
            }
        } else {
            self.player_body.pos.y = ground;
            self.jump_hvel = glam::Vec3::ZERO;
        }
        self.player_body.grounded = self.jump_vel <= 0.0;
        // 跳跃结束后清除请求（避免长按连续起跳；重置时也清）
        if on_ground && self.jump_pressed {
            self.jump_pressed = false;
        }
    }

    /// 投射物推进 + 碰撞检测：物理刚体/球体命中即销毁；NPC 命中扣血，hp≤0 移除并计分。
    ///
    /// `allow_kills = false`（GameOver 冻结）：投射物照常飞行/到期，但不判定任何命中。
    fn update_projectiles(&mut self, dt: f32, allow_kills: bool) {
        // 弹道诊断（RV3D_PROJ_DIAG=1 时启用，节流 2s）：默认关闭避免生产日志噪音
        static LAST_PROJ_LOG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // 弹丸**去向**计数（2026-09-25 加）：harness 只有 `hits` 一个数，打了多少发、
        // 有多少打在掩体上、多少飞没了**都看不见** ⇒ "改瞄法到底有没有用"无法判定。
        // 三个计数器在下面三个结算分支里各加一，随 2s 诊断行一起打印并清零。
        static PROJ_OBSTACLE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static PROJ_NPC: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static PROJ_EXPIRED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // 过期弹**差多远**（分桶）：<0.5m = 差一点点（瞄点/抖动），0.5–2m = 接近，
        // >2m = 根本不是瞄着它飞的（瞄错目标/几何）。判"该怎么修"全靠这三格。
        static PROJ_MISS_NEAR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static PROJ_MISS_MID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static PROJ_MISS_FAR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let diag = proj_diag_on();
        if diag {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let last = LAST_PROJ_LOG.load(std::sync::atomic::Ordering::Relaxed);
            if now_ms - last > 2000 {
                LAST_PROJ_LOG.store(now_ms, std::sync::atomic::Ordering::Relaxed);
                use std::sync::atomic::Ordering::Relaxed;
                let obs = PROJ_OBSTACLE.swap(0, Relaxed);
                let npc = PROJ_NPC.swap(0, Relaxed);
                let exp = PROJ_EXPIRED.swap(0, Relaxed);
                let m_near = PROJ_MISS_NEAR.swap(0, Relaxed);
                let m_mid = PROJ_MISS_MID.swap(0, Relaxed);
                let m_far = PROJ_MISS_FAR.swap(0, Relaxed);
                let first = self.projectiles.first().map(|p| {
                    format!(
                        "pos=({:.0},{:.0},{:.0}) dist={:.0}m",
                        p.position[0],
                        p.position[1],
                        p.position[2],
                        p.distance_traveled()
                    )
                });
                let nearest_npc = self
                    .npcs
                    .iter()
                    .map(|n| {
                        let dx = n.position[0] - self.player_body.pos.x;
                        let dz = n.position[2] - self.player_body.pos.z;
                        (dx * dx + dz * dz).sqrt()
                    })
                    .fold(f32::MAX, f32::min);
                log::info!(
                    "proj-diag: alive={} first=[{}] nearest_npc={:.0}m 去向(2s) npc={} obstacle={} expired={} | 过期弹离最近 NPC: <0.5m={} 0.5-2m={} >2m={}",
                    self.projectiles.len(),
                    first.unwrap_or_else(|| "none".to_string()),
                    nearest_npc,
                    npc,
                    obs,
                    exp,
                    m_near,
                    m_mid,
                    m_far
                );
            }
        }
        for p in self.projectiles.iter_mut() {
            if p.is_alive() {
                p.update(dt);
                // 只诊断时算：这发弹离**最近 NPC 胸口**有多近（过期分桶用）。
                // 字段级借用互不相交（projectiles 可变 / npcs 只读），不必克隆。
                if diag {
                    let mut best = f32::MAX;
                    for n in &self.npcs {
                        let dx = n.position[0] - p.position[0];
                        let dy = n.position[1] + 1.2 - p.position[1];
                        let dz = n.position[2] - p.position[2];
                        let d = (dx * dx + dy * dy + dz * dz).sqrt();
                        if d < best {
                            best = d;
                        }
                    }
                    if best < p.min_npc_dist {
                        p.min_npc_dist = best;
                    }
                }
            }
        }
        // 弹着标记老化：先老化再生成（本帧刚打的孔从 age=0 开始）
        for m in self.impact_marks.iter_mut() {
            m.age += dt;
        }
        self.impact_marks.retain(|m| m.age < IMPACT_MARK_LIFE);
        let mut hit_count = 0u32;
        let old = std::mem::take(&mut self.projectiles);
        let mut alive = Vec::with_capacity(old.len());
        for p in old {
            if !p.is_alive() {
                PROJ_EXPIRED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if p.min_npc_dist < 0.5 {
                    PROJ_MISS_NEAR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else if p.min_npc_dist <= 2.0 {
                    PROJ_MISS_MID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else {
                    PROJ_MISS_FAR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                // 弹着点：仅爆炸弹（如未来榴弹武器）过期时引爆 AoE；普通子弹静默消失
                // （修复：V3.0 高速弹射程尽头"神秘连发爆炸"特效——子弹不是炸弹）
                if p.explosive() {
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
                }
                continue;
            }
            if !allow_kills {
                alive.push(p);
                continue;
            }
            if self.collide_physics(&p) {
                PROJ_OBSTACLE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // 命中障碍：子弹直接消耗（障碍永久存在，枪械不再打爆障碍；
                // 手榴弹/爆炸物 AoE 仍可摧毁掩体，见爆炸结算）。
                // 不计入 hit_count → 不触发命中提示/音效（打墙没有"命中反馈"）。
                // 爆炸弹命中才有 AoE。
                // 弹着标记（弹孔）：只在**障碍刚体**上留痕 —— 球体（树冠一类）与打空的子弹不留。
                if let Some((_, point, normal)) = self.first_obstacle_hit(&p) {
                    self.push_impact_mark(point, normal);
                }
                if self.hit_obstacle_index(&p).is_some() && p.explosive() {
                    self.spawn_explosion(p.position, EXPLOSION_RADIUS, EXPLOSION_DAMAGE, false);
                }
                continue;
            }
            if let Some((idx, hit_h)) = self.hit_npc_index(&p) {
                // 友军伤害关闭：玩家弹命中同阵营 NPC 穿身而过（不造成伤害/火花/命中计数）
                if p.from_player()
                    && self.npcs[idx].team == crate::engine::ai::Team::Blue
                {
                    alive.push(p);
                    continue;
                }
                hit_count += 1;
                PROJ_NPC.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // V3.0：部位倍率按武器分段表 + 已飞距离查表（投射物携带 part_tiers）
                let mult = p.part_multiplier(hit_h, self.npcs[idx].position[1]);
                let dmg = p.damage_at_distance() * mult;
                // 命中火花点（NPC 身体命中位置）+ 伤害飘字值
                self.hit_points.push([p.position[0], hit_h, p.position[2]]);
                self.hit_damages.push(dmg);
                log::info!(
                    "weapons: 命中 NPC #{} 高度={:.1} 倍率={:.1} 伤害={:.0} 距离={:.0}m",
                    self.npcs[idx].id,
                    hit_h - self.npcs[idx].position[1],
                    mult,
                    dmg,
                    p.distance_traveled()
                );
                self.damage_npc(idx, dmg, DamageSource::Player);
                continue;
            }
            // 玩家弹 vs 网络远端玩家（服务器权威命中：杀远端玩家）
            if p.from_player() && self.net_players.iter().any(|q| q.alive) {
                let hit = self.hit_net_player(&p);
                if hit {
                    hit_count += 1;
                    self.hud.show_hit_marker();
                    continue;
                }
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

    /// 投射物是否命中物理刚体/球体（命中即销毁，不产生击杀）。
    /// V3.0 高速弹（710m/s → 每帧 11.8m）不能用单点采样（会跳过小掩体/球体），
    /// 改为上一帧→当前位置的线段与 AABB/球体求交。
    fn collide_physics(&self, p: &Projectile) -> bool {
        let (ax, ay, az) = (p.prev_position()[0], p.prev_position()[1], p.prev_position()[2]);
        let (bx, by, bz) = (p.position[0], p.position[1], p.position[2]);
        for body in &self.world.bodies {
            let aabb = body.aabb();
            if Self::segment_hits_aabb(ax, ay, az, bx, by, bz, &aabb) {
                return true;
            }
        }
        for s in &self.world.spheres {
            if Self::segment_hits_sphere(ax, ay, az, bx, by, bz, s) {
                return true;
            }
        }
        false
    }

    /// 线段 [A,B] 与 AABB 求交，返回**入口参数 t ∈ [0,1] 与入口面所在轴**（0=x / 1=y / 2=z）。
    ///
    /// 只有这一份 slab 求交代码：`segment_hits_aabb` 是它的 `is_some()` 包装。
    /// 弹孔要的是"打中了没有"**和**"打在哪一面上"，两件事分开实现一定会漂移。
    fn segment_aabb_entry(
        ax: f32, ay: f32, az: f32,
        bx: f32, by: f32, bz: f32,
        aabb: &crate::engine::physics::Aabb,
    ) -> Option<(f32, usize)> {
        let (dx, dy, dz) = (bx - ax, by - ay, bz - az);
        let mut tmin: f32 = 0.0;
        let mut tmax: f32 = 1.0;
        let mut axis = 0usize;
        for (i, (d, a, lo, hi)) in [
            (dx, ax, aabb.min.x, aabb.max.x),
            (dy, ay, aabb.min.y, aabb.max.y),
            (dz, az, aabb.min.z, aabb.max.z),
        ]
        .into_iter()
        .enumerate()
        {
            if d.abs() < 1e-9 {
                if a < lo || a > hi {
                    return None;
                }
            } else {
                let t1 = (lo - a) / d;
                let t2 = (hi - a) / d;
                let (lo_t, hi_t) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
                if lo_t > tmin {
                    tmin = lo_t;
                    axis = i;
                }
                tmax = tmax.min(hi_t);
                if tmin > tmax {
                    return None;
                }
            }
        }
        Some((tmin, axis))
    }

    /// 线段 [A,B] 与 AABB 求交（slab 法，参数 t ∈ [0,1]）
    fn segment_hits_aabb(
        ax: f32, ay: f32, az: f32,
        bx: f32, by: f32, bz: f32,
        aabb: &crate::engine::physics::Aabb,
    ) -> bool {
        Self::segment_aabb_entry(ax, ay, az, bx, by, bz, aabb).is_some()
    }

    /// 线段命中的**第一个**障碍刚体 → (下标, 弹着点, 表面法线)。
    ///
    /// 扫描顺序与 `hit_obstacle_index` 一致（同序 `world.bodies`），两者用同一份求交，
    /// 所以"子弹被谁挡下"与"弹孔画在哪堵墙"永不会给出不同答案。法线取**入口面**的轴向。
    ///
    /// 🔴 弹着点就取碰撞 AABB 的入口面：2026-09-17 起 marker 的**可见尺寸 == 碰撞 AABB**
    /// （判据见 `geom::Shape::template_half_extent`），AABB 面就是画出来的那层面。
    /// 在此之前这里必须乘一个 `visual_half_gain = 2.0` 才能把弹孔推到墙皮上 ——
    /// 那是同一个"可见尺寸翻倍"约定的另一半，不一起改就会把弹孔埋进几何里
    /// （不崩不报，只是"打了枪墙上没有孔"，2026-09-15 实测踩过）。
    fn first_obstacle_hit(&self, p: &Projectile) -> Option<(usize, [f32; 3], [f32; 3])> {
        let a = p.prev_position();
        let b = p.position;
        // 🔴 取**参数 t 最小**的那个，不能取"列表里第一个命中的"：
        // `world.bodies` 的顺序是建关顺序，不是距离顺序 —— 列表里的远处障碍完全可能排在
        // 近处障碍前面。取错了不会报错，只是弹孔贴在一块**被前面那根柱子挡住**的面上，
        // 表现成"打了枪墙上没有孔"（2026-09-15 实测踩到，找了整整一轮）。
        let mut best: Option<(f32, usize, [f32; 3], [f32; 3])> = None;
        for (i, body) in self.world.bodies.iter().enumerate() {
            let aabb = body.aabb();
            let Some((t, axis)) =
                Self::segment_aabb_entry(a[0], a[1], a[2], b[0], b[1], b[2], &aabb)
            else {
                continue;
            };
            // t == 0：线段起点已经在盒内，**没有入口面**，也就没有可贴的表面。
            // 硬算会拿"起点 + 任意轴"当弹孔 —— 效果是一块悬在半空的暗方块。
            if t <= 0.0 {
                continue;
            }
            if best.is_some_and(|(bt, ..)| bt <= t) {
                continue;
            }
            let lo = [aabb.min.x, aabb.min.y, aabb.min.z];
            let hi = [aabb.max.x, aabb.max.y, aabb.max.z];
            let mut point = [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ];
            // 来射方向沿该轴为正 ⇒ 从负侧面进入 ⇒ 法线朝负方向（背向子弹来向）
            let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]][axis];
            let mut normal = [0.0f32; 3];
            normal[axis] = if d > 0.0 { -1.0 } else { 1.0 };
            // 弹着点取入口面本身：另外两轴保持命中点（弹孔不会横向漂移）。
            // 可见尺寸 == 碰撞 AABB 之后，AABB 面就是画出来的那层墙皮，不需要任何补偿倍率。
            point[axis] = if normal[axis] > 0.0 { hi[axis] } else { lo[axis] };
            best = Some((t, i, point, normal));
        }
        best.map(|(_, i, point, normal)| (i, point, normal))
    }

    /// 线段 [A,B] 与球体求交（二次方程判别式，参数 t ∈ [0,1]）
    fn segment_hits_sphere(
        ax: f32, ay: f32, az: f32,
        bx: f32, by: f32, bz: f32,
        s: &crate::engine::physics::SphereBody,
    ) -> bool {
        let (fx, fy, fz) = (ax - s.center.x, ay - s.center.y, az - s.center.z);
        let (dx, dy, dz) = (bx - ax, by - ay, bz - az);
        let a = dx * dx + dy * dy + dz * dz;
        let r2 = s.radius * s.radius;
        if a < 1e-9 {
            return fx * fx + fy * fy + fz * fz <= r2;
        }
        let b = 2.0 * (fx * dx + fy * dy + fz * dz);
        let c = fx * fx + fy * fy + fz * fz - r2;
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            return false;
        }
        let sq = disc.sqrt();
        let t1 = (-b - sq) / (2.0 * a);
        let t2 = (-b + sq) / (2.0 * a);
        t2 >= 0.0 && t1 <= 1.0
    }

    /// 投射物命中的障碍刚体下标（球体/未命中返回 None）。
    /// 下标与 map.obstacles/world.bodies 严格对应（apply_level 同序构建、同序移除）。
    /// 与 collide_physics 一致使用线段求交（高速弹不穿透掩体）。
    fn hit_obstacle_index(&self, p: &Projectile) -> Option<usize> {
        let (ax, ay, az) = (p.prev_position()[0], p.prev_position()[1], p.prev_position()[2]);
        let (bx, by, bz) = (p.position[0], p.position[1], p.position[2]);
        self.world.bodies.iter().position(|body| {
            let aabb = body.aabb();
            Self::segment_hits_aabb(ax, ay, az, bx, by, bz, &aabb)
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

    /// 记入本轮阵亡（`round_kills_*` 的**唯一写入口**）。
    ///
    /// 🔴 2026-09-26 修：阵亡有**两条路** —— `damage_npc`（子弹/爆炸的主路径，当场把阵亡者移出
    /// `npcs`）与 `update_stress_respawns` 的兜底扫描（扫「还在数组里的 `hp <= 0`」）。以前只有
    /// 后一条在计数 ⇒ 主路径一个人都没计：170 秒 128v127 会战实测**自报阵亡 35、实际 68**
    /// （蓝营 79 vs 123；实际值由连强度反推：128 − 60）。指挥军情的 `kills` 字段直接读这个数，
    /// 于是司令看到的伤亡只有真相的一半。判据 = `every_death_path_is_counted_in_the_round_tally`。
    fn tally_round_deaths(&mut self, team: Team, count: u32) {
        if count == 0 {
            return;
        }
        match team {
            Team::Red => self.round_kills_red += count,
            Team::Blue => self.round_kills_blue += count,
        }
    }

    /// NPC 受伤结算：扣血至 0 → 移除 + 计分 + 任务目标推进；返回是否击杀。
    /// 调用方保证 `idx` 有效；下标移除后不再回移（调用方按逆序遍历或立即退出）。
    fn damage_npc(&mut self, idx: usize, dmg: f32, source: DamageSource) -> bool {
        let id = self.npcs[idx].id;
        // 受击反馈：命中瞬间闪白（0.15s 衰减）
        self.npc_hit_flash.insert(id, 0.15);
        let npc = &mut self.npcs[idx];
        npc.hp -= dmg;
        if npc.hp > 0.0 {
            return false;
        }
        let victim_team = npc.team;
        self.tally_round_deaths(victim_team, 1);
        self.npcs.remove(idx);
        // 🔴 2026-09-13 修：**只有击杀敌方才算战果**。
        // 原先这三处对**任何**击杀都生效（`score += KILL_SCORE`、`objective.progress(1)`、
        // `objective_register_kill()`），于是**打死队友也推进"歼灭敌人"**。
        // 用户 2026-09-13 实机截图就是这条：右上角 feed 里明明有 `YOU KILLED BLUE #174`
        // （自己人），HUD 却已经 `歼灭敌人 128/128` 并弹出 `VICTORY`，
        // 而小地图上红方还剩一大片 —— 计数把友军伤亡算成了战果。
        let is_enemy = victim_team != crate::engine::ai::Team::Blue;
        if is_enemy {
            self.score += KILL_SCORE;
        }
        // 击杀提示（右上角 feed）：敌我**都**提示 —— 打死自己人是需要立刻看见的事故。
        // 🔴 2026-09-15：现在**带上击杀者**（此前只报死者，玩家看不出是谁干的）。
        let victim = format!("{} #{id}", team_name(victim_team));
        let line = match source {
            DamageSource::Player => kill_line(KillerLabel::You, &victim, ""),
            DamageSource::Blast => blast_kill_line(self.last_blast_center, &victim),
        };
        self.hud.push_kill(line);
        log::info!(
            "kill: npc #{} eliminated (wave {}) team={:?} enemy={} score={}",
            id,
            self.wave,
            victim_team,
            is_enemy,
            self.score
        );
        // 任务目标：歼灭数推进（达成 → 胜利横幅/日志，波次推进不受影响）
        // ⚠️ 仅敌方：友军伤亡若也计入，一轮会在敌人尚存时提前"胜利"
        if is_enemy {
            if self.objective.progress(1) {
                self.on_objective_complete();
            }
            // 关卡系统：击杀计数（KillCount 规则用）
            self.objective_register_kill();
        }
        true
    }

    /// 生成爆炸实体：AoE 伤害 + 径向击退（生成时一次性结算，逆序遍历避免下标回移）。
    /// - 伤害衰减复用 `simd::shockwave_pressure`（所有 NPC 一次批算，指令集选路可测）；
    /// - `knockback=true` 时对命中 NPC 施加径向推挤（advance_npc 每帧指数衰减）；
    /// - 玩家在冲击半径内 → 震屏（`camera_shake_offset` 每帧读取）。
    /// 🔴 **爆炸冲击波是否被障碍挡住**（2026-09-15 补：此前 AoE 只看距离，`ob` 后面的目标
    /// 与开阔地一样吃满伤害 —— 隔着掩体炸不死人，因为掩体不存在）。
    ///
    /// 判据取自**爆心几何**，不是"障碍在不在半径内"：
    /// - 只考虑**爆心到目标之间**的障碍 ⇒ 目标背后的掩体不替它挡（那是背向的，挡不住冲击波）；
    /// - **包含爆心或目标的障碍一律跳过** —— 手榴弹贴在掩体上炸时，爆心在障碍 AABB 内部，
    ///   而那一格障碍自己就是被炸的对象，不能反过来把爆心"堵死"（全图只有一格、
    ///   其余全被挡住是最坏的结果）；目标站在障碍里同理（那是生成/推挤的异常态，
    ///   由距离衰减负责，不该让 blast 判成"被自己挡住"）。
    ///
    /// 与 `npc_occluded` 同一套 `segment_hits_aabb`（同一份几何代码，避免两套求交漂移），
    /// 差别只在**扫描的是 `map.obstacles` 而不是 `world.bodies`**：前者是玩法障碍
    /// （有血量、可摧毁），后者是物理刚体，而 AoE 伤害结算的正是前者。
    fn obstacle_blocks_blast(&self, from: [f32; 3], to: [f32; 3]) -> bool {
        self.map
            .obstacles
            .iter()
            .any(|ob| Self::blast_sample_blocked(from, to, ob))
    }

    /// 单个障碍的判定（拆出来是为了能直测"包含端点就跳过"这条规则）
    fn blast_sample_blocked(from: [f32; 3], to: [f32; 3], ob: &MapObstacle) -> bool {
        let aabb = Self::obstacle_aabb(ob);
        // 端点落在盒内 ⇒ 这一格不是"隔在中间"，不构成遮挡
        if Self::aabb_contains_point(&aabb, from) || Self::aabb_contains_point(&aabb, to) {
            return false;
        }
        Self::segment_hits_aabb(
            from[0], from[1], from[2], to[0], to[1], to[2], &aabb,
        )
    }

    /// 障碍的碰撞 AABB（碰撞体始终是 AABB、与 `shape` 无关 —— 见 `MapObstacle::shape` 注释）
    fn obstacle_aabb(ob: &MapObstacle) -> crate::engine::physics::Aabb {
        use crate::engine::physics::Vec3;
        crate::engine::physics::Aabb {
            min: Vec3::new(ob.x - ob.half_w, ob.y - ob.half_h, ob.z - ob.half_d),
            max: Vec3::new(ob.x + ob.half_w, ob.y + ob.half_h, ob.z + ob.half_d),
        }
    }

    fn aabb_contains_point(a: &crate::engine::physics::Aabb, p: [f32; 3]) -> bool {
        p[0] >= a.min.x
            && p[0] <= a.max.x
            && p[1] >= a.min.y
            && p[1] <= a.max.y
            && p[2] >= a.min.z
            && p[2] <= a.max.z
    }

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
        // 供本次 AoE 结算期间拼击杀文本用（爆炸无单一击杀者，见字段注释）
        self.last_blast_center = center;
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
        // 玩家自伤：仅在半径内、游戏进行中、且**爆心与玩家之间没有障碍**；伤害 = 距离衰减 × 封顶系数
        // （挡在掩体后面就吃不到 —— 否则"躲起来"对爆炸无效）
        let player_point = [eye.x, eye.y, eye.z];
        if damage > 0.0
            && dist < radius
            && self.game_state == GameState::Playing
            && self.hud.health > 0.0
            && !self.player_invincible
            && !self.obstacle_blocks_blast(center, player_point)
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
                    // 死亡补给：全部武器弹匣补满 + 备弹恢复初始
                    self.weapons.reset_all_ammo();
                    log::info!("explosion: 玩家被自己手榴弹炸死（弹药已重置）");
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
            // 🔴 冲击波要绕开障碍：隔着掩体的 NPC 不该吃这一发（此前只看压力衰减、
            // 完全不管中间有没有墙 —— 掩体对爆炸等于不存在）。
            let npc_point = [
                self.npcs[i].position[0],
                self.npcs[i].position[1] + NPC_HIT_CENTER_Y,
                self.npcs[i].position[2],
            ];
            if self.obstacle_blocks_blast(center, npc_point) {
                continue; // 击退已施加（冲击波推开掩体后的人），伤害不给
            }
            self.damage_npc(i, damage * f, DamageSource::Blast);
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
    ///
    /// ## 频率与幅度都是被实机反馈逼出来的，不要凭直觉调回去
    /// 旧实现是 `sin(t*47.13)` / `cos(t*53.71)`，即 **7.5Hz 与 8.5Hz**、幅度 ±0.35m。
    /// 两个问题：
    /// 1. **7.5–8.5Hz 高于人眼把抖动读成"震动"的频段**，落在"画面在抖"与"画面在闪"
    ///    之间；而枪模是屏幕锁定的（`view_inv` 抵消 view），于是**世界高频抖、枪不动**，
    ///    玩家报的"移动时整个画面大量撕裂和线条""高频小幅度摆动加大量残影"就是这个
    ///    相对运动。真实后坐/爆炸震屏在 1.5–3Hz 量级。
    /// 2. **幅度 0.35m 恰好等于玩家胶囊半径 0.35m**（`physics.rs PlayerBody::radius`）。
    ///    背贴墙时眼睛被推到墙面甚至墙内，配合 `near = 0.1` 与背面剔除，近端面被裁掉、
    ///    远端面被剔除 → **瞬间看穿墙**（用户报的"透视"里最隐蔽的一条）。
    /// 现在频率降到 2.2/2.6Hz（两轴刻意不成整数比，避免合成周期性图案），幅度上限
    /// 压到 0.12m，留出 0.23m 的墙面余量。
    pub fn camera_shake_offset(&self) -> (f32, f32) {
        if self.shake_timer <= 0.0 {
            return (0.0, 0.0);
        }
        let s = self.shake_strength * (self.shake_timer / SHAKE_DURATION);
        let t = self.time;
        ((t * 13.82).sin() * s, (t * 16.34).cos() * s)
    }

    /// 投射物命中的 NPC 下标（segment-sphere 相交：上一帧位置→当前位置连线与命中球求交，
    /// 命中球中心在 NPC 头顶 +0.8、半径 0.8；高速弹（200m/s 每帧 3.3m）避免隧道效应漏判）
    /// 命中检测：返回 (NPC 下标, 命中点相对地面的高度)。
    /// 高度用于部位倍率判定（头 1.5 / 胸 1.0 / 臂 0.8 / 腿 0.6，见设计文档）。
    fn hit_npc_index(&self, p: &Projectile) -> Option<(usize, f32)> {
        let (ax, ay, az) = (p.prev_position()[0], p.prev_position()[1], p.prev_position()[2]);
        let (bx, by, bz) = (p.position[0], p.position[1], p.position[2]);
        let (dx, dy, dz) = (bx - ax, by - ay, bz - az);
        let len2 = dx * dx + dy * dy + dz * dz;
        if len2 < 1e-9 {
            return None;
        }
        for (i, npc) in self.npcs.iter().enumerate() {
            let cx = npc.position[0];
            // 命中球：中心 +1.0m、半径 1.05m → 覆盖 -0.05..2.05m（含头+余量）。
            // 2026-08-19 修复：玩家眼位 y≈2.0，水平弹道从旧球顶（1.85m）上方掠过
            // → 瞄准身体也打不中（头顶掠过 miss）——加大后水平弹道可命中。
            // 注意：球心高度与下方 hit_height 的 +NPC_HIT_CENTER_Y 必须一致。
            let cy = npc.position[1] + NPC_HIT_CENTER_Y;
            let cz = npc.position[2];
            let r = 1.05;
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
                // 命中点世界高度 = 球心 + 最近点相对偏移 qy + NPC 地面 y
                let hit_height = qy + npc.position[1] + NPC_HIT_CENTER_Y;
                return Some((i, hit_height));
            }
        }
        None
    }

    /// 部位伤害倍率（固定阈值回退表：头部 ×1.5、胸部 ×1.0、手臂 ×0.8、腿部 ×0.6）。
    /// 命中高度 1.45 以上为头、0.95~1.45 胸、0.6~0.95 臂、以下为腿（NPC 身高 ~1.78m）。
    /// V3.0 起生产路径走 Projectile::part_multiplier（按武器分段表）；此表仅供测试与
    /// 无分段投射物（旧接口）回退参考。
    #[allow(dead_code)]
    fn part_multiplier(hit_height: f32, npc_ground_y: f32) -> f32 {
        let h = hit_height - npc_ground_y;
        if h >= 1.45 {
            1.5
        } else if h >= 0.95 {
            1.0
        } else if h >= 0.6 {
            0.8
        } else {
            0.6
        }
    }

    /// 在瞄准方向上加锥形散布：dx/dy 为相对方向的偏移角（弧度）。
    /// 以方向为 Z 轴构建正交基（right/up），返回归一化后的散布方向。
    fn spread_direction(direction: [f32; 3], dx: f32, dy: f32) -> [f32; 3] {
        let len = (direction[0] * direction[0]
            + direction[1] * direction[1]
            + direction[2] * direction[2])
            .sqrt()
            .max(1e-6);
        let d = [direction[0] / len, direction[1] / len, direction[2] / len];
        // right = normalize(cross(d, up))，up 退化（垂直射）时退回 +X
        let up = [0.0f32, 1.0, 0.0];
        let mut rx = d[1] * up[2] - d[2] * up[1];
        let mut ry = d[2] * up[0] - d[0] * up[2];
        let mut rz = d[0] * up[1] - d[1] * up[0];
        let rl = (rx * rx + ry * ry + rz * rz).sqrt();
        if rl < 1e-6 {
            rx = 1.0;
            ry = 0.0;
            rz = 0.0;
        } else {
            rx /= rl;
            ry /= rl;
            rz /= rl;
        }
        // up2 = cross(right, d)
        let ux = ry * d[2] - rz * d[1];
        let uy = rz * d[0] - rx * d[2];
        let uz = rx * d[1] - ry * d[0];
        let mut ox = d[0] + rx * dx + ux * dy;
        let mut oy = d[1] + ry * dx + uy * dy;
        let mut oz = d[2] + rz * dx + uz * dy;
        let ol = (ox * ox + oy * oy + oz * oz).sqrt().max(1e-6);
        ox /= ol;
        oy /= ol;
        oz /= ol;
        [ox, oy, oz]
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
            // 援军同样收口到玩家可达域（与主波次同一条规则）
            let reach =
                crate::engine::ai::reachable_mask(&self.grid, world_to_grid(player.x, player.z));
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
                    &reach,
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
                    //
                    // 🔴 `rule.waves` **可以长于** `WAVES_PER_LEVEL`（`defense_line.toml` 是 5）：
                    // survive 下必须**整条走 rule.waves**，不许掉进「清满 3 波就升关」那条分支 ——
                    // 旧写法在 waves=5 的图上第 3 波清完就升关并把 wave 归 1，于是第 4/5 波永远
                    // 到不了、胜利条件永远不成立（2026-09-25 真机：`wave 3 cleared` 之后紧跟
                    // `wave 1 spawned … effective=4`；红测
                    // `survive_wave_count_above_waves_per_level_still_reaches_victory`）。
                    // 胜利那一拍**不许再生成下一波**：旧代码把 `spawn_wave` 放在整个 if/else
                    // 之后，于是「守住全部波次 → 胜利」的同一帧又把最后一波重新刷了出来
                    // （2026-09-25 真机 5 波通关抓到：`survive: 全部 5 波守住 → 胜利` 之后
                    // 紧跟 `wave: wave 5 spawned 14 enemies (kind=Boss …)`，NPC #60–#73 又刷一批）。
                    let mut spawn_next = true;
                    if self.is_survive_rule() {
                        if self.wave >= self.survive_total_waves() {
                            self.hud.victory_banner = Some("防区固守！全部波次守住".to_string());
                            self.game_state = GameState::Victory(crate::engine::ai::Team::Blue);
                            self.set_won_team(crate::engine::ai::Team::Blue);
                            log::info!(
                                "survive: 全部 {} 波守住 → 胜利",
                                self.survive_total_waves()
                            );
                            spawn_next = false;
                        } else {
                            self.wave += 1;
                            // survive：波间补给窗口（血量回复 + 弹药补满）
                            self.supply_survive_break();
                        }
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
                    }
                    if spawn_next {
                        self.spawn_wave(self.wave, player);
                    }
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
        // 出生点收口用的**玩家可达域**：整个波次算一次（O(格数)），每只出生只做一次查表。
        let reach = crate::engine::ai::reachable_mask(&self.grid, world_to_grid(player.x, player.z));
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
            let id = self.spawn_npc_ring(player, i as u32, count as u32, n, spd, hpx, rng, role, &reach);
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
        reach: &[bool],
    ) -> usize {
        let tau = std::f32::consts::TAU;
        let angle = slot as f32 * (tau / divisor.max(1) as f32) + wave_n as f32 * 0.37;
        let radius = 40.0 + 40.0 * ((slot * 7 + wave_n * 3) % 5) as f32 / 4.0;
        let (x, z) = self.push_out_of_obstacle(
            (player.x + angle.cos() * radius).clamp(-250.0, 250.0),
            (player.z + angle.sin() * radius).clamp(-250.0, 250.0),
        );
        // 🔴 出生点收口到**玩家可达域**（`reach`）：只保证"可站立"是不够的 ——
        // 可站立的小口袋会让这只永远走不到玩家（#17 根因链第 3 条的出生侧），
        // 且它每 1/3 秒重规划一次、每秒几百次 A* 全部 `连通域穷尽`。
        let (x, z) = match self.nearest_in_component(reach, world_to_grid(x, z)) {
            Some(g) => grid_to_world(g),
            None => (x, z),
        };
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
            direct_goal: false,
            direct_x: 0.0,
            direct_z: 0.0,
            last_goal: [0.0, 0.0],
            aidiag_bucket: u64::MAX,
                attack_timer: 0.0,
                reposition: None,            hp,
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

    /// 出生点避开障碍：从给定点出发做**确定性扩环搜索**，返回最近的可站立点。
    ///
    /// 🔴 2026-09-12 重写。原实现有三个缺陷，实测 **6/255（红5 蓝1）** 个单位出生后仍卡在
    /// 不可通行格上：
    /// ① 外推方向是**离世界原点**（`sx += sx/d*4`），与障碍无关 —— 位于原点西北/东南侧的
    ///    单位会被推得更远，完全可能推进另一栋楼；
    /// ② 只走 8 步，走完仍不可通行就**静默返回坏点**，既无兜底也无计数；
    /// ③ 因此这个故障从上线起就没有任何人看见过。
    ///
    /// 现在改为「环 r 上均匀取 8r 个采样、由近及远、方向顺序固定」⇒ 结果**确定**
    /// （同一输入必得同一输出，冒烟与截图对比才不会失效）；扫完仍失败则打 `warn` 并原样返回。
    /// 判据仍是导航网格 `is_passable`（它与单位能否移动是同一条口径）；
    /// 「建筑视觉体大于碰撞盒」是另一件事，见 AGENTS.md 未结案 4。
    fn push_out_of_obstacle(&self, x: f32, z: f32) -> (f32, f32) {
        const CELL: f32 = 4.0;
        const MAX_RING: i32 = 8;
        if !self.blocked_at(x, z) {
            return (x.clamp(-250.0, 250.0), z.clamp(-250.0, 250.0));
        }
        for r in 1..=MAX_RING {
            let rf = r as f32;
            let samples = 8 * r;
            for k in 0..samples {
                let a = std::f32::consts::TAU * (k as f32) / (samples as f32);
                let px = x + a.cos() * rf * CELL;
                let pz = z + a.sin() * rf * CELL;
                if !self.blocked_at(px, pz) {
                    return (px.clamp(-250.0, 250.0), pz.clamp(-250.0, 250.0));
                }
            }
        }
        log::warn!("spawn: ({x:.0}, {z:.0}) 扩环 {MAX_RING} 层仍找不到可站立点，原样返回");
        (x.clamp(-250.0, 250.0), z.clamp(-250.0, 250.0))
    }

    /// 该点是否不可站立（导航网格判据，与单位移动同一条口径）
    fn blocked_at(&self, x: f32, z: f32) -> bool {
        !self.grid.is_passable(world_to_grid(x, z))
    }

    /// 在给定连通域（`mask`，由 `ai::reachable_mask` / `ai::largest_component_mask` 给出）里
    /// 取离 `want` 最近的格；`want` 本身在域内就原样返回。
    ///
    /// 确定性：先比直线距离²，再比行主序序号（同一输入必得同一格）。
    /// 用途 = **出生点收口**：`push_out_of_obstacle` 只保证"可站立"，而可站立的**小口袋**
    /// （院子里 9 格、墙缝 2 格）会让单位永远走不到任何人（实测每秒几百次 A* 全部
    /// `连通域穷尽`）。域内没有格时返回 `None`（调用方保持原点位，不 panic）。
    fn nearest_in_component(&self, mask: &[bool], want: GridPos) -> Option<GridPos> {
        let w = self.grid.width();
        let h = self.grid.height();
        let inside = |x: i32, y: i32| {
            x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && mask[y as usize * w + x as usize]
        };
        if inside(want.x, want.y) {
            return Some(want);
        }
        let mut best: Option<(i64, usize, GridPos)> = None;
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                if !inside(x, y) {
                    continue;
                }
                let d = ((x - want.x) as i64).pow(2) + ((y - want.y) as i64).pow(2);
                let idx = y as usize * w + x as usize;
                if best.as_ref().map_or(true, |(bd, bi, _)| (d, idx) < (*bd, *bi)) {
                    best = Some((d, idx, GridPos::new(x, y)));
                }
            }
        }
        best.map(|(_, _, g)| g)
    }

    /// 该点是否可站立。调试机位用它挑一个**不被墙挡**的观察方向 ——
    /// 手算方向失败过 13 次，判据本来就该由程序用（见 docs/PROGRESS.md）。
    pub fn standable(&self, x: f32, z: f32) -> bool {
        !self.blocked_at(x, z)
    }

    /// 调试用：该点是否落在**静态刚体（建筑/障碍）**的 AABB 内。
    ///
    /// 用的是**与 `npc_occluded` 完全同一套** `segment_hits_aabb`（退化成一个 ±0.01m 的
    /// 极短线段），不另写一套包含判定 —— 本仓最贵的 bug 就是"同一个量两套来源"。
    ///
    /// 存在的理由：`standable` 走导航网格，建筑走刚体 AABB，**两者不是同一套几何**。
    /// 这个查询用来量化两者的差异（未结案 4 的根因），也是第 24 轮取景问题的判据。
    pub fn point_in_body(&self, x: f32, y: f32, z: f32) -> bool {
        self.world
            .bodies
            .iter()
            .any(|body| Self::segment_hits_aabb(x - 0.01, y, z, x + 0.01, y, z, &body.aabb()))
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
        // 🔬 **互换对照开关**（2026-09-13 加，用于未结案第一条"红蓝阵营不对称"）
        //
        // 压力模式下**红方恒胜**（四次 190s 对撞，蓝方净耗 101/102/102/103，红方 34/52/56/76），
        // 而出生几何按构造是对称的。判据写在未结案清单里：
        //   **把两侧半场对调 —— 赢家跟着半场走 = 地图几何；跟着队伍走 = 单位/AI 行为。**
        // `RV3D_SWAP_SIDES=1` 只交换两个半场，不动队伍/人数/角色/任何其它参数，
        // 所以它是干净的**单变量**对照。默认关闭 ⇒ 生产行为逐字节不变。
        let swap_sides = std::env::var("RV3D_SWAP_SIDES").as_deref() == Ok("1");
        // 见下方 `facing` 的注释：出生时朝向"对面半场"而不是朝向玩家
        let face_enemy = std::env::var("RV3D_FACE_ENEMY").as_deref() == Ok("1");
        // 🔴 压力模式的出生点必须收口到**主连通域**（2026-09-25 真机实测）：
        // 出生环 150–198m 正好穿过城市街区，`push_out_of_obstacle` 只保证"可通行"，
        // 实测 4 只红方里 2 只落在 **9 格 / 2 格**的小口袋（探针
        // `tmp_stress_spawn_reachability`），于是每秒几百次 A* 全部 `连通域穷尽` ——
        // 红蓝两军各在自己的院子里隔空对射，那套"20 轮红蓝对撞"的 A/B 也受此影响。
        // 参考域取**全图最大连通域**（不能取玩家所在的中央安全区：那是另一个小域）。
        let reach = crate::engine::ai::largest_component_mask(&self.grid);
        for side in 0..2u32 {
            let team = if side == 0 { Team::Red } else { Team::Blue };
            let base_angle = if (side == 0) != swap_sides { 0.0 } else { std::f32::consts::PI };
            // 蓝方少生 1 名：玩家本身就是蓝方士兵（红 64 vs 蓝 63+玩家 = 64v64）
            let per_side = if side == 0 { sides } else { sides.saturating_sub(1) };
            for i in 0..per_side {
                // 半场 ±~63° 扇形铺开，半径 150m + 确定性抖动（超出障碍环带 58-130m）
                let spread = -1.1 + (i as f32 / sides.max(1) as f32) * 2.2;
                let angle = base_angle + spread;
                let radius = STRESS_SPAWN_RADIUS + 12.0 * ((i * 7 + side) % 5) as f32;
                let (x, z) = self.push_out_of_obstacle(
                    (player.x + angle.cos() * radius).clamp(-250.0, 250.0),
                    (player.z + angle.sin() * radius).clamp(-250.0, 250.0),
                );
                let (x, z) = match self.nearest_in_component(&reach, world_to_grid(x, z)) {
                    Some(g) => grid_to_world(g),
                    None => (x, z),
                };
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
                direct_goal: false,
                direct_x: 0.0,
                direct_z: 0.0,
                last_goal: [0.0, 0.0],
                aidiag_bucket: u64::MAX,
                attack_timer: 0.0,
                reposition: None,                    hp: profile.hp,
                    max_hp: profile.hp,
                    role: role_for(i, 1, profile.flank_chance),
                    tactic: Tactic::Advance,
                    dodge_timer: 0.0,
                    hit_cooldown: 0.0,
                    last_hp: profile.hp,
                    team,
                    // 🔬 2026-09-13：**出生朝向**。
                    //
                    // 原写法是 `(player.z - z).atan2(player.x - x)` —— **所有 NPC 都朝向玩家**。
                    // 而玩家恒为 `Team::Blue`、站在原点，于是：
                    //   * 红方 NPC 朝向的是一个**敌人** ⇒ 方向正确；
                    //   * 蓝方 NPC 朝向的是一个**友军** ⇒ **开局就在看错方向**。
                    // 这正是"赢家跟着队伍走、不跟着半场走"（`RV3D_SWAP_SIDES` 对照实测）的
                    // 一个候选解释：若 `facing` 参与感知/推进，蓝方等于让了先手。
                    //
                    // `RV3D_FACE_ENEMY=1` 把朝向改成**指向对面半场**（真正的敌人方向），
                    // 作为单变量对照。默认关 ⇒ 生产行为不变。
                    facing: if face_enemy {
                        if base_angle == 0.0 { std::f32::consts::PI } else { 0.0 }
                    } else {
                        (player.z - z).atan2(player.x - x)
                    },
                    fire_accum: 0.0,
                    knockback: [0.0, 0.0],
                grenade_timer: 0.0,
                });
            }
        }
        // 临时埋点（RV3D_NPC_POS=1）：前 3 个 NPC 的实际坐标（摆调试机位用）+
        // **出生后仍卡在不可通行格上的红/蓝计数**。
        // 后者是 `push_out_of_obstacle` 的失败计数：它 8 步推不出去时会**静默返回原样的坏点**，
        // 而它的判据是导航网格 `is_passable` —— 与建筑视觉体/碰撞盒不是同一套几何。
        // 红方 base_angle=0(+X)、蓝方 π(−X)，城市非镜像对称 ⇒ 两侧落点可能系统性不同。
        // 取景验完即删（教训 20）。
        if std::env::var("RV3D_NPC_POS").as_deref() == Ok("1") {
            for n in self.npcs.iter().take(3) {
                log::info!(
                    "npcpos: #{} team={:?} ({:.1}, {:.1}, {:.1})",
                    n.id,
                    n.team,
                    n.position[0],
                    n.position[1],
                    n.position[2]
                );
            }
            let (mut bad_r, mut bad_b) = (0u32, 0u32);
            for n in &self.npcs {
                if !self
                    .grid
                    .is_passable(world_to_grid(n.position[0], n.position[2]))
                {
                    match n.team {
                        Team::Red => bad_r += 1,
                        Team::Blue => bad_b += 1,
                    }
                }
            }
            log::info!(
                "npcpos: 出生仍卡在不可通行格 红={} 蓝={} / 共 {}",
                bad_r,
                bad_b,
                self.npcs.len()
            );
        }
        let red_ids: Vec<usize> = self.npcs.iter().filter(|n| n.team == Team::Red).map(|n| n.id).collect();
        let blue_ids: Vec<usize> = self.npcs.iter().filter(|n| n.team == Team::Blue).map(|n| n.id).collect();
        self.command = Some((
            crate::engine::ai_command::Army::build(Team::Red, &red_ids),
            crate::engine::ai_command::Army::build(Team::Blue, &blue_ids),
        ));
        log::info!("command: 三三制编成 红营 {} 人 / 蓝营 {} 人", red_ids.len(), blue_ids.len());
        self.round_started_at = self.time;
        log::info!(
            "battle: 压力模式第 {} 轮开战（红 {} vs 蓝 {}+玩家，共 {} 名 NPC，并行 AI={}）",
            self.stress_round,
            sides,
            sides.saturating_sub(1),
            self.npcs.len(),
            self.ai_parallel
        );
    }

    /// 推进单个 NPC：感知 → 状态机 → 战术决策 → 躲避 → A* 路径 → 移动 → 朝向。
    /// 与旧版串行循环体逐行为一致（普通波次目标=玩家，行为不变；压力模式目标=敌对 NPC）。
    fn step_npc(index: usize, npc: &mut Npc, ctx: &AiStepCtx) {
        // 视野半径：压力模式全场可见（两军立即接火），普通模式保持原值
        let sight = if ctx.stress { STRESS_SIGHT } else { NPC_SIGHT };
        // 目标位置：与遮挡预计算共用 resolve_ai_target，杜绝"按 A 算遮挡、按 B 决策"
        let target_pos = resolve_ai_target(
            index,
            ctx.stress,
            ctx.spectator,
            ctx.targets,
            ctx.fallback_targets,
            ctx.player,
        );
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
        // 目标被几何体挡住 = 不可见。缺数据（空 slice）时按"未挡住"处理，保持旧行为。
        let occluded = ctx.target_occluded.get(index).copied().unwrap_or(false);
        let can_see_target = dist < sight && !occluded;
        npc.perception = NpcPerception {
            enemy_visible: can_see_target,
            // 🔴「知道要打谁」与「现在看得见」分两条通道（未结案 #17 的修法②）：
            // 出生半径 40–80m 而 `NPC_SIGHT = 60` ⇒ 出生在 60m 外的那批**永远**看不见玩家，
            // 旧行为下它们恒为 Patrol，而 `update_waves` 要求 `npcs.is_empty()` ⇒
            // **这一波永远清不掉**（实测 #8 在 77.8m 上守了整局）。进攻方不该靠视距才知道要打哪。
            // ⚠️ 开火不受影响：`Chase → Attack` 仍要求 `enemy_visible`（上面那行）。
            target_known: ctx.target_known,
            // 压力模式攻击距离放宽 x2.5（约 30m）：同速互追刷步机的残局收敛（2026-08-23）
            enemy_in_range: dist < npc.attack_range * if ctx.stress { 2.5 } else { 1.0 },
            start_patrol: prev == NpcState::Idle,
            patrol_finished: false,
            player_aiming: facing_angle < AIM_ANGLE && can_see_target,
            player_facing: target_facing,
            took_hit,
            low_hp: npc.hp < npc.max_hp * LOW_HP_RATIO,
            under_fire,
        };
        let state = npc.state_machine.update(npc.perception);
        // 诊断埋点（`RV3D_AI_DIAG=1`）：把"这个 NPC 为什么不进 Attack"压成一行。
        // 未结案 #17：survive 第 1 波残余 1–2 只十余分钟恒为 `Patrol`，波次永远清不掉；
        // 而 `Patrol` 只能由 `enemy_visible == false` 维持（`ai.rs::NpcStateMachine`），
        // `enemy_visible = dist < sight && !occluded` ⇒ 只可能是"太远"或"被挡"，
        // 这一行把二者分开（`occluded` 是原始判据；`lines` 为 0 = 遮挡数据缺失、按未挡处理）。
        // 🔴 键 = 时间桶(5s) + id，但**节流状态必须挂在每只 NPC 自己身上**。
        //   旧实现是 `static SEEN: AtomicU64` 只记"最后一个 key"：两只以上卡住的 NPC
        //   逐帧交替 ⇒ 每帧都"变了" ⇒ 恒真，300s 刷 16.4 万行（§20.6 实测），
        //   而这里的注释一直声称"每个卡住的 NPC 每 5s 一行"——注释与代码同时是错的。
        //   `step_npc` 对每个 `npc` 是 `&mut` 独占（`par_for_each_mut`）⇒ 无需 atomic。
        if ai_diag() && state != NpcState::Attack {
            if aidiag_due(&mut npc.aidiag_bucket, ctx.time, npc.id) {
                // 2026-09-25 扩展：实测 NPC 有路径却在**爬行**（0.02 m/s vs 设定 4 m/s），
                // 只靠 state/dist 判不出来 ⇒ 把"移动为什么没发生"的四个候选一起打出来：
                //   path=idx/len（路点推进到哪）wp_d（离当前路点多远）
                //   dodge（>0 = 正在侧向弹开，这一支会**跳过前进**）
                //   goal=direct|path（走的是直行兜底还是路径）
                let wp_d = if npc.path_index < npc.path.len() {
                    let (wx, wz) = grid_to_world(npc.path[npc.path_index]);
                    ((wx - npc.position[0]).powi(2) + (wz - npc.position[2]).powi(2)).sqrt()
                } else {
                    -1.0
                };
                log::info!(
                    "aidiag: #{} state={:?} dist={:.1} sight={:.0} known={} occluded={} lines={} pos=({:.1}, {:.1}) path={}/{} wp_d={:.1} dodge={:.1} goal={}",
                    npc.id,
                    state,
                    dist,
                    sight,
                    ctx.target_known,
                    occluded,
                    ctx.target_occluded.len(),
                    npc.position[0],
                    npc.position[2],
                    npc.path_index,
                    npc.path.len(),
                    wp_d,
                    npc.dodge_timer,
                    if npc.direct_goal { "direct" } else { "path" }
                );
            }
        }
        // 火-机动交替打（2026-08-26）：攻击态站打数秒 → 换下一个掩体/侧移点（再站打）
        if state == NpcState::Attack {
            npc.attack_timer += ctx.dt;
            if npc.reposition.is_none() {
                // 站打阈值：3.4s 后开始换位；换位冷却由 timer 重置管理
                if npc.attack_timer > 3.4 {
                    // 优先换到掩体点（周围障碍环带内遮挡点）；无掩体则沿垂直方向侧移 9m
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    let goal = crate::engine::ai::find_cover_points(ctx.grid, npc_g, COVER_MAX_DIST)
                        .into_iter()
                        .filter(|c| {
                            let (gx, gz) = grid_to_world(c.pos);
                            let dx = gx - npc.position[0];
                            let dz = gz - npc.position[2];
                            dx * dx + dz * dz > 9.0
                        })
                        .min_by_key(|c| c.dist)
                        .map(|c| {
                            let (gx, gz) = grid_to_world(c.pos);
                            [gx, gz]
                        });
                    let goal = goal.unwrap_or_else(|| {
                        // 无掩体：垂直目标方向侧移（id 奇偶定左右，制造交替推进感）
                        let dx = target_pos.x - npc.position[0];
                        let dz = target_pos.z - npc.position[2];
                        let dl = (dx * dx + dz * dz).sqrt().max(1.0);
                        let side = if npc.id % 2 == 0 { 1.0 } else { -1.0 };
                        [
                            npc.position[0] - dz / dl * 9.0 * side,
                            npc.position[2] + dx / dl * 9.0 * side,
                        ]
                    });
                    npc.reposition = Some(goal);
                }
            } else {
                // 已在换位中：到位（<2m）或换位超时（5s）→ 回站打并重置计时
                let [gx, gz] = npc.reposition.unwrap();
                let dx = gx - npc.position[0];
                let dz = gz - npc.position[2];
                if dx * dx + dz * dz < 4.0 || npc.attack_timer > 8.4 {
                    npc.reposition = None;
                    npc.attack_timer = 0.0;
                }
            }
        } else {
            // 非攻击态：清换位/计时（保持其它状态机行为）
            npc.attack_timer = 0.0;
            npc.reposition = None;
        }
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
        // 🔴 2026-09-26：**掩体爬行者（CoverCrawler）不再被冲锋覆盖**。
        // 实测依据（survive 模式 270s / 4 波，`RV3D_AI_DIAG=1` 的 `aidiag: tactic 1s`）：
        // `CoverSeek=0%` **整场**、`CoverAdvance=0%` —— 不是"地图没掩体"（defense_line 内圈
        // 有 8 段沙袋），而是 `should_charge`（≥50% 的 NPC 在追/打就全队冲锋）**几乎一直成立**，
        // 而下面那条 CoverSeek 升级要求 `!ctx.charge`，冲锋覆盖又把 CoverCrawler 也改成 Advance
        // ⇒ 掩体战术在生存模式里**一次都没生效**。现在只豁免这一个角色（约 1/6，第 3 波起存在），
        // "冲锋 = 其余角色全队直突"的原设计保持不变。
        let is_cover_crawler = npc.role == TacticalRole::CoverCrawler;
        if ctx.charge
            && npc.role != TacticalRole::Suppressor
            && !is_cover_crawler
            && tactic != Tactic::Retreat
            && !is_flank_maneuver
        {
            tactic = Tactic::Advance;
        }
        // 掩体利用：突击/压制手接近射程边缘时先评估障碍环带掩体（先移动到掩体再推进开火）。
        // 环带内无射程内掩体（如玩家处于中央安全区）时保持原直线推进 → 冒烟站定语义不变；
        // 只在 Chase 态生效且冲锋时不做（冲锋 = 全队直突）——**唯一的例外是掩体爬行者**。
        // 压力模式（NPC-vs-NPC）：目标在射程内且本 NPC 附近（40m）存在障碍格 → 也进入
        // 掩体利用（互射战场用障碍环带/关卡掩体，总指挥指令单 #2 阶段二）。
        if state == NpcState::Chase
            && (!ctx.charge || is_cover_crawler)
            && matches!(
                tactic,
                Tactic::Advance | Tactic::Suppress | Tactic::CoverAdvance
            )
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
            ctx.squad_wps.get(index).copied().flatten(),
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

    /// NPC 相互推离（空间哈希；1.6m 内斥力，位置微调，不影响路径规划下一帧重算）
    fn apply_npc_separation(npcs: &mut [Npc]) {
        if npcs.len() < 2 {
            return;
        }
        // 空间哈希：cell = 4m；键 = (gx, gy)
        let cell = 4.0f32;
        let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> =
            std::collections::HashMap::with_capacity(npcs.len() / 2 + 1);
        for (i, n) in npcs.iter().enumerate() {
            let key = ((n.position[0] / cell).floor() as i32, (n.position[2] / cell).floor() as i32);
            grid.entry(key).or_default().push(i);
        }
        let min_d = 1.6f32;
        let mut pushes = vec![[0.0f32, 0.0]; npcs.len()];
        for (_, bucket) in grid.iter() {
            for &i in bucket {
                let (xi, zi) = (npcs[i].position[0], npcs[i].position[2]);
                // 邻域 3x3 cell
                let (cx, cy) = ((xi / cell).floor() as i32, (zi / cell).floor() as i32);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let key = (cx + dx, cy + dy);
                        if let Some(other) = grid.get(&key) {
                            for &j in other {
                                if j <= i {
                                    continue;
                                }
                                let dxp = npcs[j].position[0] - xi;
                                let dzp = npcs[j].position[2] - zi;
                                let d2 = dxp * dxp + dzp * dzp;
                                if d2 >= min_d * min_d {
                                    continue;
                                }
                                let d = d2.sqrt().max(1e-3);
                                let push = (min_d - d) * 0.5;
                                let ux = dxp / d;
                                let uz = dzp / d;
                                pushes[i][0] -= ux * push;
                                pushes[i][1] -= uz * push;
                                pushes[j][0] += ux * push;
                                pushes[j][1] += uz * push;
                            }
                        }
                    }
                }
            }
        }
        for (i, n) in npcs.iter_mut().enumerate() {
            let (px, pz) = (pushes[i][0], pushes[i][1]);
            if px != 0.0 || pz != 0.0 {
                // 归因埋点（#17）：分离力是**每帧一次的纯位置位移**（不乘 dt、不设上限），
                // 手感上"半步" = 0.08m（≈25fps 下一帧的行走位移 0.16m 的一半）。
                if ai_diag() {
                    let ord = std::sync::atomic::Ordering::Relaxed;
                    NOTE_SEP_PUSHED.fetch_add(1, ord);
                    if (px * px + pz * pz).sqrt() >= 0.08 {
                        NOTE_SEP_BIG.fetch_add(1, ord);
                    }
                }
                n.position[0] += px;
                n.position[2] += pz;
                n.position[1] = terrain_height_at(n.position[0], n.position[2]);
            }
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
        // 2026-08-24：借用代替克隆（16KB/帧网格拷贝消除；与 self.npcs/map 字段级借用拆分共存）
        let grid = &self.grid;
        let time = self.time;
        let player_yaw = camera.yaw;
        // 分层调度（2026-08-11）：稳定重排 npcs 为 [Near..., Far...]，
        // 返回 Near 段长度。Near = 与玩家实时交互/近距离（延迟敏感，走 scene_pool
        // = P 核/CCD0），Far = 远距离重计算（走 ai_pool = CCD1/E 核）。
        // 重排后 under_fire / targets 均在当前数组顺序上构建，帧内索引对齐；
        // 各 NPC 步进彼此独立（AiStepCtx 只读），重排不改变步进语义。
        let stress = self.stress;
        // 观战模式（玩家无敌）：NPC 目标兜底 = 敌方重心（红/蓝互搏，火力不浪费在无敌玩家上）
        // 计时埋点（RV3D_AI_PROF=1，第 47 轮）：`update_ai` 顶部三处**每帧固定 O(n) 工作**。
        // 动机：第 46 轮实测"并行池 vs 串行"在 255 NPC 下**完全无法区分**
        // （ai_us 1460 vs 1391、fps 132.8 vs 132.7）—— 若开销在逐 NPC 步进上，
        // 并行应当明显更快。既然没有，说明 ai_us 被与"是否并行"无关的每帧工作主导。
        let t_a = std::time::Instant::now();
        let (rc, bc) = team_centroids(&self.npcs);
        let t_b = std::time::Instant::now();
        let fallback_targets: Vec<[f32; 3]> = if self.player_invincible {
            self.npcs
                .iter()
                .map(|n| match n.team {
                    Team::Red => [bc[0], 0.0, bc[1]],
                    Team::Blue => [rc[0], 0.0, rc[1]],
                })
                .collect()
        } else {
            Vec::new()
        };
        let t_c = std::time::Instant::now();
        let tier_params = AiTierParams::default();
        let near_len = partition_ai_tiers(&mut self.npcs, |npc| {
            ai_tier_of(npc, &player, stress, &tier_params)
        });
        let t_d = std::time::Instant::now();
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static TICK: AtomicU32 = AtomicU32::new(0);
            if std::env::var("RV3D_AI_PROF").is_ok()
                && TICK.fetch_add(1, Ordering::Relaxed) % 119 == 0
            {
                log::info!(
                    "aiprof: 重心={}us 兜底目标={}us 分层重排={}us 三处合计={}us",
                    t_b.duration_since(t_a).as_micros(),
                    t_c.duration_since(t_b).as_micros(),
                    t_d.duration_since(t_c).as_micros(),
                    t_d.duration_since(t_a).as_micros()
                );
            }
        }
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
        // 指挥节拍：0.5s 营司令评估 + 战士班目标点（未接敌编队推进，接敌由逐人战术接管）
        if self.stress {
            if let Some(cmd) = self.command.as_mut() {
                // rc/bc 复用 update_ai 顶部已算的敌我重心（不再重复 O(n) 扫描）
                // LLM 指挥官：红蓝各一独立上下文窗口互搏（无/失效 → 该侧启发式）
                let to_ov = |cmds: Vec<crate::llm_cmd::CompanyCmd>| {
                    cmds.into_iter()
                        .map(|c| crate::engine::ai_command::CmdOverride {
                            order: match c.order {
                                crate::llm_cmd::LlmOrder::Assault => {
                                    crate::engine::ai_command::CompanyOrder::Assault
                                }
                                crate::llm_cmd::LlmOrder::Hold => {
                                    crate::engine::ai_command::CompanyOrder::Hold
                                }
                                crate::llm_cmd::LlmOrder::FlankL => {
                                    crate::engine::ai_command::CompanyOrder::Flank(1)
                                }
                                crate::llm_cmd::LlmOrder::FlankR => {
                                    crate::engine::ai_command::CompanyOrder::Flank(-1)
                                }
                                crate::llm_cmd::LlmOrder::Regroup => {
                                    crate::engine::ai_command::CompanyOrder::Regroup
                                }
                            },
                            x: c.x,
                            z: c.z,
                        })
                        .collect::<Vec<_>>()
                };
                let llm_red: Option<Vec<crate::engine::ai_command::CmdOverride>> =
                    self.llm.as_ref().and_then(|l| l.take_red()).map(to_ov);
                let llm_blue: Option<Vec<crate::engine::ai_command::CmdOverride>> =
                    self.llm.as_ref().and_then(|l| l.take_blue()).map(to_ov);
                // 军情两个口径：本营阵亡（round_kills_<自己>）与战果（= 敌方阵亡）。
                // 🔴 重组判据读的是**战果** —— 喂本营阵亡会让那条分支永远不触发（未结案 27）。
                cmd.0.update(
                    &self.npcs,
                    &grid,
                    dt,
                    self.round_kills_red,
                    self.round_kills_blue,
                    bc,
                    llm_red.as_deref(),
                );
                cmd.1.update(
                    &self.npcs,
                    &grid,
                    dt,
                    self.round_kills_blue,
                    self.round_kills_red,
                    rc,
                    llm_blue.as_deref(),
                );
                // 态势推送（红/蓝各独立上下文）
                if let Some(l) = &self.llm {
                    let sr = build_llm_situation(&cmd.0);
                    let sb = build_llm_situation(&cmd.1);
                    l.push_red(&sr, cmd.0.companies.len());
                    l.push_blue(&sb, cmd.1.companies.len());
                }
                if self.time - self.command_log_at >= 5.0 {
                    self.command_log_at = self.time;
                    log::info!("command: 红{} | 蓝{}", cmd.0.summary(), cmd.1.summary());
                }
            }
        }
        // 计时：以下三段都在**并行分派之前**、每帧做超线性扫描，
        // 正是"并行≈串行 + 中位 1178/最大 10677 尖峰"的形态（第 49 轮）。
        let t_pre = std::time::Instant::now();
        let squad_wps: Vec<Option<[f32; 2]>> = if self.stress {
            if let Some(cmd) = self.command.as_ref() {
                self.npcs.iter().map(|n| {
                    let army = match n.team { Team::Red => &cmd.0, Team::Blue => &cmd.1 };
                    army.squad_waypoint(n.id)
                }).collect()
            } else { vec![None; self.npcs.len()] }
        } else { vec![None; self.npcs.len()] };
        // 弹道威胁预扫：存活子弹水平距离 < THREAT_RADIUS 且朝 NPC 方向飞行 → 该 NPC 受火力威胁
        let t_uf = std::time::Instant::now();
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
        // ⚠️ 上一轮 `t_occ` 插错了位置（落在 `target_occlusion` 之后），导致"威胁预扫"
        // 这个标签把下面三段都算了进去。现在按真实区间拆成三个计时点。
        let t_pick0 = std::time::Instant::now();
        // 压力模式：每 NPC 预选最近敌对目标（敌对 NPC 优先、玩家兜底；O(n²) 纯读，串行）
        let targets: Vec<Option<(usize, [f32; 3], f32)>> = if self.stress {
            pick_stress_targets(&self.npcs, STRESS_SIGHT)
        } else {
            Vec::new()
        };
        // 掩体利用评估用的当前关卡障碍环带（theme 随关卡轮换）
        let theme = theme_for_level(self.level);
        // 视线遮挡预计算：必须在 ctx 之前算完（返回 owned Vec），否则会与
        // step_ai_* 对 self.npcs 的可变借用冲突。resolve_ai_target 在 stress=false 时
        // 忽略 targets、spectator=false 时忽略 fallback_targets，所以一条调用通吃三种模式。
        let t_pick = std::time::Instant::now();
        // 🔴 视线遮挡**每 N 帧重算一次**（2026-09-12 第 102 轮）。
        //
        // 第 50 轮实测：本节占 `ai_us` 的 **18–39%（244–519µs）**，成本是
        // **每帧为全部 NPC** 做线段-AABB 扫描（255 × 2 采样 × 约 1100 刚体 ≈ **56 万次/帧**）。
        //
        // 而遮挡关系在相邻帧之间几乎不变，**AI 的反应时间在 100–300ms 量级** ⇒
        // 3 帧（约 23ms @130fps）的陈旧**完全在容差内**。
        //
        // ⚠️ A/B 开关（第 103 轮加）：`RV3D_OCCL_REFRESH=1` 回到"每帧重算"的旧行为。
        // 每次 `update_ai` 只读一次环境变量（**每帧 1 次，不是每实例**，见教训 32）⇒ 不必 OnceLock。默认 4。
        let occlusion_refresh: u32 = std::env::var("RV3D_OCCL_REFRESH")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(4);
        let recompute = self.occl_cache_age == 0 || self.occl_cache.len() != self.npcs.len();
        let target_occluded: Vec<bool> = if recompute {
            let v = target_occlusion(
                &self.npcs,
                &self.world.bodies,
                self.stress,
                self.player_invincible,
                &targets,
                &fallback_targets,
                &player,
            );
            self.occl_cache = v.clone();
            self.occl_cache_age = occlusion_refresh;
            v
        } else {
            self.occl_cache_age -= 1;
            self.occl_cache.clone()
        };
        let t_occ = std::time::Instant::now();
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static TICK: AtomicU32 = AtomicU32::new(0);
            if std::env::var("RV3D_AI_PROF").is_ok()
                && TICK.fetch_add(1, Ordering::Relaxed) % 119 == 0
            {
                log::info!(
                    "aiprof2: 班目标点={}us 威胁预扫={}us 目标选择={}us 视线遮挡={}us 四段合计={}us（子弹 {} / NPC {}）",
                    t_uf.duration_since(t_pre).as_micros(),
                    t_pick0.duration_since(t_uf).as_micros(),
                    t_pick.duration_since(t_pick0).as_micros(),
                    t_occ.duration_since(t_pick).as_micros(),
                    t_occ.duration_since(t_pre).as_micros(),
                    self.projectiles.iter().filter(|p| p.is_alive()).count(),
                    self.npcs.len()
                );
            }
        }
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
                squad_wps: &squad_wps,
                spectator: self.player_invincible,
                // 只有在"真在打的一局"里才把玩家当作已知目标：
                // 开始菜单的游走（StartMenu 也调 update_ai）必须保持"没看见就随便走"的观感，
                // 压力模式有自己的一套选目标逻辑（target_known 会让红蓝绕过 pick_stress_targets）。
                target_known: self.game_state == GameState::Playing && !self.stress,
                fallback_targets: &fallback_targets,
                target_occluded: &target_occluded,
            };
            if self.npcs.len() >= PARALLEL_AI_MIN && self.ai_parallel {
                // safety: 已分层（Near 在前），双池分片互不相交；ctx 只含共享只读数据
                Self::step_ai_parallel(&mut self.npcs, near_len, &ctx);
            } else {
                Self::step_ai_serial(&mut self.npcs, &ctx);
            }
            // 分离力（2026-08-23 三三制防重叠）：空间哈希 4m 单元，1.6m 内施加推离，
            // 消除阵型/搜索点的人叠人（楔形队形+残局集结处生效）
            Self::apply_npc_separation(&mut self.npcs);
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
                "ai: npcs={} near={} far={} idle={} patrol={} chase={} attack={} tactics={:?} red={} blue={} ra={} ba={} rc=({:.0},{:.0}) bc=({:.0},{:.0})",
                self.npcs.len(),
                near_len,
                self.npcs.len() - near_len,
                counts[0],
                counts[1],
                counts[2],
                counts[3],
                tactics,
                self.npcs.iter().filter(|n| n.team == Team::Red).count(),
                self.npcs.iter().filter(|n| n.team == Team::Blue).count(),
                self.npcs
                    .iter()
                    .filter(|n| n.team == Team::Red && n.state_machine.state() == NpcState::Attack)
                    .count(),
                self.npcs
                    .iter()
                    .filter(|n| n.team == Team::Blue && n.state_machine.state() == NpcState::Attack)
                    .count(),
                bc[0], bc[1],
                rc[0], rc[1],
            );
        }
        // 攻击态 NPC 对玩家造成伤害（1 秒一次），驱动 HUD 血条
        // 伤害值取当前有效波次的 dps（Boss 波更高，见 wave_profile）
        let dps = wave_profile(self.effective_wave(self.wave)).dps;
        if !self.stress && !self.player_invincible
            && self.time - self.last_damage_time >= 1.0
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
                self.hud.push_kill("你被击杀了".to_string());
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
                // 死亡补给：全部武器弹匣补满 + 备弹恢复初始
                self.weapons.reset_all_ammo();
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
                // 伤害距离与感知对齐（2026-08-23：压力模式 2.5 倍攻击距离，
                // 否则全员感知 30m 内站定但无法开火 → 僵持观望）
                let rng = npc.attack_range * if self.stress { 2.5 } else { 1.0 };
                if dx * dx + dz * dz <= rng * rng {
                    pairs.push((i, *t));
                }
            }
        }
        for (i, t) in pairs {
            self.npcs[i].fire_accum += dt;
            if self.npcs[i].fire_accum >= 1.0 {
                self.npcs[i].fire_accum = 0.0;
                self.npcs[t].hp -= dps;
                // 击杀提示：NPC 互射击杀（击杀者阵营 + id，与玩家击杀同一套拼装）
                if self.npcs[t].hp <= 0.0 && self.npcs[t].hp > -dps {
                    let (aid, a, v) = (self.npcs[i].id, self.npcs[i].team, self.npcs[t].team);
                    let vid = self.npcs[t].id;
                    let killer = KillerLabel::Named(format!("{} #{aid}", team_name(a)));
                    let victim = format!("{} #{vid}", team_name(v));
                    self.hud.push_kill(kill_line(killer, &victim, ""));
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
        // 本轮自损累计（阵亡者按阵营计数；供指挥军情的 kills 字段 = **该营自身阵亡数**）
        // enemy_dead = 敌军（Red）本帧阵亡数，单独累计供任务目标推进
        // ⚠️ 这里只扫「还留在数组里的 hp<=0」（兜底路径）；`damage_npc` 打死的人当场就被移出数组，
        // 所以那边必须自己计数 —— 见 `tally_round_deaths` 的注释与判据。
        let mut red_dead = 0u32;
        let mut blue_dead = 0u32;
        for n in &self.npcs {
            if n.hp <= 0.0 {
                match n.team {
                    Team::Red => red_dead += 1,
                    Team::Blue => blue_dead += 1,
                }
            }
        }
        let enemy_dead = red_dead;
        self.tally_round_deaths(Team::Red, red_dead);
        self.tally_round_deaths(Team::Blue, blue_dead);
        self.npcs.retain(|n| n.hp > 0.0);
        let red = self.npcs.iter().filter(|n| n.team == Team::Red).count();
        let blue = self.npcs.len() - red;
        if self.npcs.len() != before {
            // 任务目标：本轮歼灭数推进（达成 → 胜利横幅/日志；补员逻辑不受影响）
            // 🔴 只计敌军阵亡。旧写法用 (before - self.npcs.len()) = **双方合计**阵亡，
            // 压力模式下 target=128 是**单方**兵力、全场 255 人，于是双方合计死到 128
            // （全场才死一半）就刷"本轮敌军全灭"横幅 —— 横幅是假的。普通模式全部 NPC
            // 都属 Red，两种口径等价，所以这个改动只修正压力模式。
            if self.objective.progress(enemy_dead) {
                self.on_objective_complete();
            }
            log::info!(
                "battle: 阵亡 {}（红={} 蓝={} 存活）",
                before - self.npcs.len(),
                red,
                blue
            );
        }
        // 回合超时上限（2026-08-23：城市地图下残局易僵持刷步，300s 按存活多者判胜）
        if red > 0 && blue > 0 && self.round_started_at >= 0.0 && self.time - self.round_started_at > 300.0 {
            let winner = if red >= blue { Some(Team::Red) } else { Some(Team::Blue) };
            let who = if winner == Some(Team::Red) { "红方" } else { "蓝方" };
            self.round_reset_at = self.time + 10.0;
            self.round_winner = winner;
            self.hud.victory_banner = Some(format!("{} 获胜（回合超时 存活 {}:{}）— 10 秒后下一轮", who, red, blue));
            log::info!(
                "battle: 第 {} 轮超时（红={} 蓝={}）→ {} 获胜，10 秒后重置",
                self.stress_round, red, blue, who
            );
        }
        // 兵力先耗尽方判负（2026-08-23：玩家无敌观战，纯 AI-vs-AI 博弈）
        if (red == 0 || blue == 0) && self.round_reset_at < 0.0 {
            let winner = if red == 0 && blue == 0 {
                None
            } else if red == 0 {
                Some(Team::Blue)
            } else {
                Some(Team::Red)
            };
            self.round_winner = winner;
            self.round_reset_at = self.time + 10.0;
            let who = match winner {
                Some(Team::Red) => "红方",
                Some(Team::Blue) => "蓝方",
                None => "平局",
            };
            self.hud.victory_banner = Some(format!("{} 获胜（兵力耗尽）— 10 秒后下一轮", who));
            log::info!(
                "battle: 第 {} 轮结束（红={} 蓝={}）→ {} 获胜，10 秒后重置",
                self.stress_round,
                red,
                blue,
                who
            );
        }
        // 10 秒后重置：清场重开本轮
        if self.round_reset_at >= 0.0 && self.time >= self.round_reset_at {
            self.round_reset_at = -1.0;
            self.round_winner = None;
            self.stress_round += 1;
            log::info!("battle: 第 {} 轮重开（阵亡清场）", self.stress_round);
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
            // 直方图（不再全量枚举/格式化，消除逐帧日志刷屏）
            let (mut ov, mut rs, mut si, mut sr, mut gh) = (0usize, 0usize, 0usize, 0usize, 0usize);
            for e in &self.collisions {
                match e.kind {
                    physics::CollisionKind::AabbOverlap => ov += 1,
                    physics::CollisionKind::AabbResolved => rs += 1,
                    physics::CollisionKind::SphereIntersect => si += 1,
                    physics::CollisionKind::SphereResolved => sr += 1,
                    physics::CollisionKind::GroundHit => gh += 1,
                }
            }
            log::info!(
                "physics: {} events (overlap={} resolved={} sphere={} sphere_res={} ground={})",
                self.collisions.len(),
                ov,
                rs,
                si,
                sr,
                gh
            );
        }
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
    squad_wp: Option<[f32; 2]>,
) {
    // 无路径（或已走完）时按状态 + 战术选择目标
    if npc.path.is_empty() || npc.path_index >= npc.path.len() {
        // 🔴 2026-09-25 真机实测补：**先把身体挪出阻挡格，再寻路**。
        // 起因：`direct_goal` 直行与分离力会把 NPC 顶进障碍 AABB（其所在格子不可通行），
        // 而寻路是从"最近可通行格"起算的 ⇒ 第一个路点可能在墙的**另一侧** ⇒
        // NPC 顶着墙走、永远到不了路点（`path_index` 不前进 ⇒ 也不再重规划，
        // 实测爬行速度 **0.02 m/s** vs 设定 4 m/s，`astar calls=0`）。
        // ⇒ 用与**出生**同一套判据（`passable_or_nearest` 环扫）把身体挪回可站立点，
        // 这样"身体所在格 = 路径起点"重新自洽，第一个路点就是它的邻格。
        let own = world_to_grid(npc.position[0], npc.position[2]);
        if !grid.is_passable(own) {
            if let Some(gp) = crate::engine::ai::passable_or_nearest(grid, own, 8) {
                let (wx, wz) = grid_to_world(gp);
                npc.position[0] = wx;
                npc.position[2] = wz;
                NOTE_NPC_UNSTUCK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        // 火-机动换位：换位目标优先（任何状态，一经设定即向换位点移动）
        let goal = if let Some(rp) = npc.reposition {
            world_to_grid(rp[0], rp[1])
        } else {
            match state {
            NpcState::Chase => match tactic {
                // 突击/压制：直线逼近（压制手到射程边缘即转 Attack 站定）
                // 班目标点：未接敌且有命令时优先向班目标点推进（>15m 时）
                Tactic::Advance | Tactic::Suppress => {
                    if let Some(wp) = squad_wp {
                        let sd2 = (npc.position[0] - wp[0]).powi(2) + (npc.position[2] - wp[1]).powi(2);
                        if sd2 > 15.0 * 15.0 {
                            world_to_grid(wp[0], wp[1])
                        } else {
                            world_to_grid(target.x, target.z)
                        }
                    } else {
                        world_to_grid(target.x, target.z)
                    }
                }
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
                // 班目标点：有命令时优先按班目标推进；否则确定性巡逻点（随 id 相位与时间缓慢旋转）
                if let Some(wp) = squad_wp {
                    world_to_grid(wp[0], wp[1])
                } else {
                    // 🔴 #17 根因修复：旧版绕 home 画圆（r=20+id*3），出生在视距
                    // 外的敌人永远绕自家转圈不收敛（aidiag 实证：dist 83-91m、
                    // occluded=false），波次永远清不掉。现在圆心向目标推进 70%、
                    // 半径封顶 18m：扫掠仍提供变化，但每圈都在逼近，走进视距后
                    // Chase/Attack 自然接管。
                    let angle = npc.id as f32 * 2.399 + (time / 8.0).floor() * 0.7;
                    let cx = npc.home[0] + (target.x - npc.home[0]) * 0.7;
                    let cz = npc.home[1] + (target.z - npc.home[1]) * 0.7;
                    world_to_grid(cx + 18.0 * angle.cos(), cz + 18.0 * angle.sin())
                }
            }
        }
        };
        // 诊断：把本帧选定的目标记下来（`aidiag: move` 每行的 `goal=`；见 `Npc::last_goal`）
        npc.last_goal = grid_to_world(goal).into();
        let start_raw = world_to_grid(npc.position[0], npc.position[2]);
        // 🔴 2026-09-25 真机实测修：NPC 经常**站在阻挡格里**（`direct_goal` 直行会把它顶进障碍，
        // 分离力也会互相挤进去），而 `find_path` 要求起点可通行 ⇒ 每次重规划都 O(1) 失败
        // （实测 `calls=108 fails=108 起点阻挡=108`），NPC 从此**完全没有路径**，
        // 只能继续直行贴墙 ⇒ `occluded=true` ⇒ 玩家打不到 ⇒ 波次永远清不掉（#17 的可见症状）。
        // ⇒ 起点先搬到最近的可通行格（确定性的），再寻路。
        let start = crate::engine::ai::passable_or_nearest(grid, start_raw, 8).unwrap_or(start_raw);
        // 寻路兜底（2026-08-23 残局卡死修复）：目标不可达时先找最近可行格，
        // 仍无路径则直行进逼（可能蹭障碍但绝不原地踏步），保证任何局面都在动。
        let mut path = find_path(grid, start, goal);
        if path.is_none() {
            // 目标侧同样搬一次：螺旋扫描目标周边（半径 8 格）找可行的最近格
            if let Some(gp) = crate::engine::ai::passable_or_nearest(grid, goal, 8) {
                path = find_path(grid, start, gp);
            }
        }
        npc.path = path.clone().unwrap_or_default();
        npc.path_index = 0;
        // 直行机动：路径全空（含兜底失败）→ 朝世界坐标目标直线移动，绝不原地踏步
        npc.direct_goal = path.is_none();
        if npc.direct_goal {
            let (wx, wz) = grid_to_world(goal);
            npc.direct_x = wx;
            npc.direct_z = wz;
        }
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

    // 攻击态原地站定（冒烟瞄准依据 `npc: #id stand`）；火-机动换位中不站定（连续移动）
    if state == NpcState::Attack && npc.reposition.is_none() {
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
    } else if npc.direct_goal {
        (npc.direct_x, npc.direct_z)
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
        let (step_x0, step_z0) = (npc.position[0], npc.position[2]);
        // 2026-08-25 穿墙修复 + 2026-09-25 沿墙滑动：移动后对存活的静态障碍 AABB 推开；
        // 若整步被推回（覆盖率建网后 NPC 会贴着薄墙/家具走），改沿接触面切向滑一步。
        let (px, pz) =
            step_with_slide(obstacles, (step_x0, step_z0), (mx, mz), step, NPC_BODY_RADIUS);
        npc.position[0] = px;
        npc.position[2] = pz;
        // 归因埋点（#17）：这一步"想走"了多远、实际净位移多少 —— 净位移 < 半步 即仍被障碍推回
        // （滑动也没救回来的正撞/夹角，是真正贴着墙磨的帧）。
        if ai_diag() {
            let ord = std::sync::atomic::Ordering::Relaxed;
            NOTE_MOVE_STEP.fetch_add(1, ord);
            let net = ((px - step_x0).powi(2) + (pz - step_z0).powi(2)).sqrt();
            if net < step * 0.5 {
                NOTE_MOVE_UNDONE.fetch_add(1, ord);
            }
        }
    }
    npc.position[1] = terrain_height_at(npc.position[0], npc.position[2]);
}

/// NPC 身体半径（米）：与 `resolve_circle_obstacles` 的推开半径、`world_to_grid` 的
/// 「身体所在格」判据共用同一个值（别在别处再写 0.45）。
const NPC_BODY_RADIUS: f32 = 0.45;

/// 一步"走 + 防穿墙 + **沿墙滑动**"：返回新的水平位置。
///
/// 🔴 2026-09-25 加（覆盖率建网规则的配套修复）。导航网格改成"覆盖 ≥1/3 格才封格"之后，
/// 路径会贴着薄墙/家具走，而原实现把"走进障碍"的整步交给 `resolve_circle_obstacles` 推回来
/// ⇒ 真机实测（RV3D_AI_DIAG=1，defense_line 第 1 波残局）**1 秒 64 帧里有 18–30 帧位移被
/// 完全抵消**，NPC 实测速度掉到 **1.3–2.2 m/s**（设定 4.0）⇒ 9–14m 外磨到超时，
/// 波次照样清不掉。
///
/// 修法与玩家的 `push_out_of_aabb` **一样**：推回量超过半步时，把意图方向**投影到接触面的
/// 切向**再走一次 —— 撞墙只损失法向分量，不丢整帧。正撞（意图与法线平行）没有切向可走，
/// 保留推回点；滑动结果若还不如原地，也保留推回点（**绝不倒退**）。
fn step_with_slide(
    obs: &[MapObstacle],
    from: (f32, f32),
    dir: (f32, f32),
    step: f32,
    r: f32,
) -> (f32, f32) {
    let want = (from.0 + dir.0 * step, from.1 + dir.1 * step);
    let (px, pz) = resolve_circle_obstacles(obs, want.0, want.1, r);
    let net = ((px - from.0).powi(2) + (pz - from.1).powi(2)).sqrt();
    if net >= step * 0.5 {
        return (px, pz);
    }
    // 接触法线方向 = 意图点 − 推回点（即障碍把这一步顶回来的方向）
    let (nx, nz) = (want.0 - px, want.1 - pz);
    let nl = (nx * nx + nz * nz).sqrt();
    if nl < 1e-4 {
        return (px, pz);
    }
    let (ux, uz) = (nx / nl, nz / nl);
    let dot = dir.0 * ux + dir.1 * uz;
    let (mut tx, mut tz) = (dir.0 - dot * ux, dir.1 - dot * uz);
    let tl = (tx * tx + tz * tz).sqrt();
    if tl < 1e-3 {
        return (px, pz); // 正撞：没有切向可走
    }
    tx /= tl;
    tz /= tl;
    let (sx, sz) = (px + tx * step, pz + tz * step);
    let (qx, qz) = resolve_circle_obstacles(obs, sx, sz, r);
    let gain = ((qx - from.0).powi(2) + (qz - from.1).powi(2)).sqrt();
    if gain > net {
        (qx, qz)
    } else {
        (px, pz)
    }
}

/// 圆（半径 r）对存活障碍 AABB 的水平推开（NPC 移动后防穿墙；MapObstacle 版）
fn resolve_circle_obstacles(obs: &[MapObstacle], x: f32, z: f32, r: f32) -> (f32, f32) {
    let mut ox = x;
    let mut oz = z;
    for ob in obs {
        let (hx, hz) = (ob.half_w, ob.half_d);
        let cx = (ox - ob.x).clamp(-hx, hx);
        let cz = (oz - ob.z).clamp(-hz, hz);
        let dx = ox - (ob.x + cx);
        let dz = oz - (ob.z + cz);
        let d2 = dx * dx + dz * dz;
        if d2 < r * r {
            if d2 > 1e-6 {
                let d = d2.sqrt();
                let push = r - d;
                ox += dx / d * push;
                oz += dz / d * push;
            } else {
                let px = hx + r - (ox - ob.x).abs();
                let pz = hz + r - (oz - ob.z).abs();
                if px < pz {
                    ox += if ox > ob.x { px } else { -px };
                } else {
                    oz += if oz > ob.z { pz } else { -pz };
                }
            }
        }
    }
    (ox, oz)
}

/// 网络远端玩家快照 id 基址（与 NPC id 空间隔离：100000+）
/// 快照里远端玩家的 id 区（`crate::net::NET_PLAYER_BASE` 的同源定义；改一处必须改两处
/// —— 现在两处是同一个常量，见下面的 `use`）。
const NET_PLAYER_BASE: u32 = crate::net::NET_PLAYER_BASE;

/// 圆（半径 r）对静态障碍 AABB 的水平推开：返回 (x, z)（AABB 为 (cx±half_w, cz±half_d)）
fn resolve_circle_static(
    bodies: &[physics::Body],
    x: f32,
    z: f32,
    r: f32,
) -> (f32, f32) {
    let mut ox = x;
    let mut oz = z;
    for b in bodies {
        let (hx, hz) = (b.half_extents.x, b.half_extents.z);
        let cx = (ox - b.position.x).clamp(-hx, hx);
        let cz = (oz - b.position.z).clamp(-hz, hz);
        let dx = ox - (b.position.x + cx);
        let dz = oz - (b.position.z + cz);
        let d2 = dx * dx + dz * dz;
        if d2 < r * r {
            if d2 > 1e-6 {
                let d = d2.sqrt();
                let push = r - d;
                ox += dx / d * push;
                oz += dz / d * push;
            } else {
                // 圆心在盒内：沿最小穿透轴推出
                let px = hx + r - (ox - b.position.x).abs();
                let pz = hz + r - (oz - b.position.z).abs();
                if px < pz {
                    ox += if ox > b.position.x { px } else { -px };
                } else {
                    oz += if oz > b.position.z { pz } else { -pz };
                }
            }
        }
    }
    (ox, oz)
}

/// 红蓝阵营存活 NPC 的平均 x/z（阵营为空 → [0.0, 0.0]；命令行军/军情用）
/// 生成红营态势 JSON（LLM 指挥官输入；严格字段：兵力/重心/接敌/当前命令）
fn build_llm_situation(a: &crate::engine::ai_command::Army) -> String {
    let side = match a.side {
        Team::Red => "red",
        Team::Blue => "blue",
    };
    let mut s = format!(
        "{{\"battle\":\"128v128\",\"side\":\"{side}\",\"map_half\":270,\"enemy\":{{\"x\":{:.0},\"z\":{:.0}}},\"companies\":[",
        a.enemy_centroid[0], a.enemy_centroid[1]
    );
    for (i, c) in a.companies.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let cur = match c.order {
            crate::engine::ai_command::CompanyOrder::Assault => "Assault",
            crate::engine::ai_command::CompanyOrder::Hold => "Hold",
            crate::engine::ai_command::CompanyOrder::Flank(1) => "FlankL",
            crate::engine::ai_command::CompanyOrder::Flank(_) => "FlankR",
            crate::engine::ai_command::CompanyOrder::Regroup => "Regroup",
        };
        s.push_str(&format!(
            "{{\"id\":{i},\"strength\":{:.0},\"x\":{:.0},\"z\":{:.0},\"contact\":{},\"current\":\"{cur}\"}}",
            c.report.strength, c.report.centroid[0], c.report.centroid[1], c.report.contact
        ));
    }
    s.push_str("]}");
    s
}

fn team_centroids(npcs: &[Npc]) -> ([f32; 2], [f32; 2]) {
    let mut rc = [0.0f32; 2];
    let mut bc = [0.0f32; 2];
    let mut rn = 0usize;
    let mut bn = 0usize;
    for n in npcs {
        match n.team {
            Team::Red => { rc[0] += n.position[0]; rc[1] += n.position[2]; rn += 1; }
            Team::Blue => { bc[0] += n.position[0]; bc[1] += n.position[2]; bn += 1; }
        }
    }
    let avg = |c: [f32; 2], n: usize| -> [f32; 2] {
        if n > 0 { [c[0] / n as f32, c[1] / n as f32] } else { [0.0, 0.0] }
    };
    (avg(rc, rn), avg(bc, bn))
}

// 子模块（见 docs/refactor-plan.md）
#[cfg(test)] mod tests;

// 子模块（见 docs/refactor-plan.md）
mod types;

// 子模块（见 docs/refactor-plan.md）
mod net;

// 子模块（见 docs/refactor-plan.md）
mod session;

