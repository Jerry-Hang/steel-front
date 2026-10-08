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

