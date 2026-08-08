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
use crate::net::{Client, NetworkMessage, PlayerState, Server};
use super::camera::{Camera, CameraMode};
use super::ai::{
    find_cover_points, find_path, flank_goal, should_flank, wave_profile, GridMap, GridPos,
    NpcPerception, NpcState, NpcStateMachine, WaveKind,
};
use super::physics::{self, Body, CollisionEvent, CollisionListener, PlayerBody, Vec3 as Pv};
use super::renderer::terrain_height_at;
use super::window::{WINDOW_HEIGHT, WINDOW_WIDTH};
use super::weapons::{Firearm, Projectile, ProjectileWeapon};
use crate::ui::HudState;

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
/// 玩家移动速度（米/秒，第一人称 WASD）
const PLAYER_SPEED: f32 = 6.0;
/// NPC 就近掩体搜索半径（网格格数）
const COVER_MAX_DIST: u32 = 10;
/// 脚步声音效限频间隔（秒）
const FOOTSTEP_INTERVAL: f32 = 0.5;
/// 每关波次数：清完 WAVES_PER_LEVEL 波升关，难度按累计有效波次递进（跨关不回落）
const WAVES_PER_LEVEL: u32 = 3;
/// 程序化障碍环带内半径（米）。
///
/// 必须 > NPC 最大攻击距离(16) + 掩体搜索半径(40) = 56：否则攻击态 NPC 会就近跑去掩体，
/// 不再原地站定（冒烟依赖 `npc: #id stand` 日志瞄准点射）；同时保证玩家出生点附近弹道无阻挡。
const MAP_RING_INNER: f32 = 58.0;
/// 障碍簇数量基数：实际簇数 = MAP_CLUSTERS + seed % 5（6..=10）
const MAP_CLUSTERS: u32 = 6;
/// 障碍盒高度（米）
const MAP_BLOCK_HEIGHT: f32 = 2.4;

/// 网络环回演示（仅 RV3D_NET=1 启用）：同进程 Server + Client
struct NetworkDemo {
    server: Server,
    client: Client,
    seq: u32,
    last_log: f32,
}

/// 游戏主状态机（开始菜单 → 游戏中 → 死亡结算）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    /// 开始菜单：任意键开始
    StartMenu,
    /// 游戏中：波次战斗
    Playing,
    /// 死亡结算：R 重开
    GameOver,
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

/// 程序化地图上的静态障碍盒（AABB：世界坐标中心 + x/z 半尺寸，贴地高度 MAP_BLOCK_HEIGHT）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapObstacle {
    pub x: f32,
    pub z: f32,
    pub half_w: f32,
    pub half_d: f32,
    /// 障碍种类（第一关全部为 Wall；主题轮换见 theme_for_level）
    pub kind: ObstacleKind,
}

/// 程序化关卡布局：确定性（种子 = 关卡号），障碍全部位于中央安全环带之外
#[derive(Debug, Clone, Default)]
pub struct LevelMap {
    pub obstacles: Vec<MapObstacle>,
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
            ring_outer: 130.0,
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
    /// 主武器：弹匣 + 换弹 + 后坐力（M1 步枪）
    firearm: Firearm,
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
    /// 程序化合成音效库（枪声/脚步/命中/换弹/提示/环境）
    sfx: SfxBank,
    /// 在场投射物
    projectiles: Vec<Projectile>,
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
    /// 上次对玩家造成伤害的时间（攻击态 NPC 每秒扣血）
    last_damage_time: f32,
    /// HUD 状态（每帧喂 fps/血量，渲染前取 quad 列表）
    pub hud: HudState,
    /// fps 统计：时间窗内帧数
    frames: u64,
    /// fps 统计时间窗起点
    fps_window_start: Instant,
    /// 音频播放器（SilentSink：rodio 未安装，样本被丢弃，但混音/衰减链路真实运行）
    audio: AudioPlayer<SilentSink>,
    /// 音频采样率
    audio_sample_rate: u32,
    /// 网络环回演示（默认关闭，RV3D_NET=1 启用）
    net_demo: Option<NetworkDemo>,
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
            firearm: Firearm::new(
                ProjectileWeapon::new("M1 Rifle", 25.0, 3.0, 200.0, 60.0, 5.0),
                30,
                120,
                2.0,
                0.006,
                0.003,
            ),
            pending_kick: (0.0, 0.0),
            player_body: PlayerBody::new(Pv::new(0.0, 0.0, 0.0), 0.5, 1.6),
            move_forward: false,
            move_backward: false,
            move_left: false,
            move_right: false,
            footstep_timer: 0.0,
            sfx: SfxBank::new(48_000),
            projectiles: Vec::new(),
            fire_cooldown: 0.0,
            shots: 0,
            hits: 0,
            grid: GridMap::new(GRID_SIZE, GRID_SIZE),
            npcs,
            ai_log_time: 0.0,
            last_damage_time: 0.0,
            hud: HudState::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
            frames: 0,
            fps_window_start: Instant::now(),
            audio_sample_rate: 48_000,
            level: 1,
            map: LevelMap::default(),
            audio: {
                let player = AudioPlayer::new(SilentSink::new(48_000, 2));
                let mut player = player;
                player.mixer_mut().set_master(0.8);
                player.mixer_mut().set_channel_volume(Channel::Sfx, 1.0);
                // 环境音循环（SilentSink：输出被丢弃，但混音/衰减链路真实运行）
                SfxBank::new(48_000).play(
                    player.mixer_mut(),
                    SfxKind::Ambient,
                    AudioSource::new(glam::Vec3::new(0.0, 2.0, 0.0), 0.4),
                    Channel::Sfx,
                    true,
                );
                player
            },
            net_demo: {
                let enabled = std::env::var("RV3D_NET")
                    .map(|v| !v.is_empty())
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
            game_state: GameState::StartMenu,
            wave: 1,
            wave_timer: 0.0,
            score: 0,
            next_npc_id: NPC_COUNT as u32,
            last_status_log: 0.0,
        };
        // 初始关卡布局（level 1，种子 = 1）：物理刚体 + AI 网格 + 玩家安全区复位
        game.apply_level(1);
        game
    }

    /// 当前游戏状态（供 main.rs 控制光标捕获等输入行为）
    pub fn state(&self) -> GameState {
        self.game_state
    }

    /// 开始菜单任意键：进入游戏（开始/重开一局）
    pub fn on_any_key(&mut self, player: &glam::Vec3) {
        if self.game_state == GameState::StartMenu {
            self.start_run(player);
        }
    }

    /// 死亡结算界面 R：重开一局
    pub fn request_restart(&mut self, player: &glam::Vec3) {
        if self.game_state != GameState::GameOver {
            return;
        }
        self.start_run(player);
    }

    /// 开始一局：复位血量/弹药/分数/波次/关卡，重建第 1 关地图，清掉残留 NPC 后生成第 1 波
    fn start_run(&mut self, player: &glam::Vec3) {
        self.hud.health = self.hud.max_health;
        self.hud.ammo = self.hud.max_ammo;
        self.hud.reserve = self.firearm.reserve();
        self.hud.settings_open = false;
        self.hud.confirm_quit = false;
        self.hud.cancel_rebind();
        self.score = 0;
        self.wave = 1;
        self.wave_timer = 0.0;
        self.fire_cooldown = 0.0;
        self.firearm.reset();
        self.pending_kick = (0.0, 0.0);
        // 重开一局 = 从第 1 关全新地图开始（同时把玩家拉回原点安全区）
        self.apply_level(1);
        self.player_body.pos = Pv::new(0.0, 0.0, 0.0);
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
        if !self.npcs.is_empty() {
            log::info!("game: purged {} leftover npcs on run start", self.npcs.len());
            self.npcs.clear();
        }
        self.spawn_wave(1, player);
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
        self.map = generate_level_map(level);
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
        self.last_dt = dt;
        self.time += dt;
        self.fire_cooldown = (self.fire_cooldown - dt).max(0.0);
        // 武器换弹计时 + HUD 弹药/换弹状态同步
        self.firearm.update(dt);
        self.hud.ammo = self.firearm.magazine();
        self.hud.max_ammo = self.firearm.max_magazine();
        self.hud.reserve = self.firearm.reserve();
        self.hud.reloading = self.firearm.is_reloading();
        self.hud.reload_progress = self.firearm.reload_progress();
        // 关卡号同步（由关卡推进 / 重开写入，供 HUD 显示）
        self.hud.level = self.level;
        // 命中标记衰减 + 音量同步
        self.hud.tick(dt);
        self.audio.mixer_mut().set_master(self.hud.volume);
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
        self.world.step(dt);
        self.drain_collisions();
        match self.game_state {
            GameState::StartMenu => {
                // 菜单吸引模式：世界照常运行（NPC 游走/追击），不结算伤害与波次
                self.update_projectiles(dt, true);
                self.update_ai(dt, camera);
            }
            GameState::Playing => {
                self.update_projectiles(dt, true);
                self.update_ai(dt, camera);
                self.update_waves(dt, &camera.position());
            }
            GameState::GameOver => {
                // 冻结玩法：AI/伤害/波次停止；投射物继续飞行但不再判定命中/击杀
                self.update_projectiles(dt, false);
            }
        }
        // 状态日志（1 秒一条，冒烟断言 game: wave= 序列用）
        if self.time - self.last_status_log >= 1.0 {
            self.last_status_log = self.time;
            let enemy_hp = self.npcs.first().map(|n| n.max_hp).unwrap_or(0.0);
            log::info!(
                "game: wave={} enemies={} enemy_hp={:.0} hp={:.0}/{:.0} score={}",
                self.wave,
                self.npcs.len(),
                enemy_hp,
                self.hud.health,
                self.hud.max_health,
                self.score
            );
        }
        // 音频：每帧按 dt 渲染样本（SilentSink 丢弃输出，混音/衰减链路真实运行）
        let frames = ((self.audio_sample_rate as f32) * dt) as usize;
        self.audio
            .tick(&AudioListener::new(camera.position()), frames.min(8192));
        self.update_net(camera);
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
                    if let Ok(ack) = demo.server.handle_join(from, name.clone()) {
                        let _ = demo.server.send_to(&ack, from);
                    }
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
        self.hud.screen = if self.hud.settings_open {
            HudScreen::Settings
        } else {
            match self.game_state {
                GameState::StartMenu => HudScreen::Start,
                GameState::GameOver => HudScreen::GameOver,
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
            self.firearm.ammo_ratio() * 100.0
        );
        render_text(&line2, 10.0, 62.0, Color::CYAN, 1.3, &mut quads);
        quads
    }

    /// 构建默认光照场景（方向光 + 环境光 + 2 点光；阴影未绑定贴图，保持关闭）
    pub fn light_uniform(&self) -> super::lighting::LightUniform {
        use super::lighting::{DirectionalLight, LightUniform, PointLight};
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
        LightUniform::build(
            Some(&sun),
            &[point_a, point_b],
            glam::Vec3::new(0.5, 0.55, 0.6),
            0.35,
            None,
        )
    }

    /// 尝试开火（受射速冷却限制）。`origin`/`direction` 来自相机；返回是否真的开火。
    pub fn fire(&mut self, origin: [f32; 3], direction: [f32; 3]) -> bool {
        if self.fire_cooldown > 0.0 {
            return false;
        }
        match self.firearm.try_fire(origin, direction) {
            Some(projectile) => {
                self.fire_cooldown = self.firearm.fire_interval();
                let (kick_pitch, kick_yaw) = self.firearm.current_kick();
                self.pending_kick.0 += kick_pitch;
                self.pending_kick.1 += kick_yaw;
                self.projectiles.push(projectile);
                self.shots += 1;
                let src = AudioSource::new(
                    glam::Vec3::new(origin[0], origin[1], origin[2]),
                    1.0,
                );
                // 开火音效带确定性音量抖动（0.95..=1.0，按射击计数循环），避免机械重复
                let shot_scale = 0.95 + 0.05 * ((self.shots % 5) as f32 / 4.0);
                self.sfx.play_variant(
                    &mut self.audio.mixer_mut(),
                    SfxKind::Gunshot,
                    src,
                    Channel::Sfx,
                    false,
                    shot_scale,
                );
                log::info!(
                    "weapons: shot #{} ({} alive)",
                    self.shots,
                    self.projectiles.len()
                );
                true
            }
            None => {
                // 空弹匣自动换弹（try_fire 内部触发）或换弹中：换弹提示音
                if self.firearm.is_reloading() {
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

    /// 请求换弹（R 键）；已在换弹/满弹匣/无备弹时无副作用
    pub fn request_reload(&mut self) {
        let was_reloading = self.firearm.is_reloading();
        self.firearm.start_reload();
        if !was_reloading && self.firearm.is_reloading() {
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

    /// 调试补给（设置面板 N 键）：弹匣补满 + 提示音
    pub fn give_ammo(&mut self) {
        self.firearm.reset();
        let src = AudioSource::new(self.player_eye(), 1.0);
        self.sfx.play(
            &mut self.audio.mixer_mut(),
            SfxKind::UiBlip,
            src,
            Channel::Sfx,
            false,
        );
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
        }
        // selection >= 2 是键位行，滚轮不做调整（Enter 进入绑定）
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
                let src = AudioSource::new(self.player_pos(), 0.5);
                // 脚步声交替强弱（0.8 / 1.0），确定性变化
                let step_scale = 0.8 + 0.2 * ((self.time * 2.0) as u32 % 2) as f32;
                self.sfx.play_variant(
                    &mut self.audio.mixer_mut(),
                    SfxKind::Footstep,
                    src,
                    Channel::Sfx,
                    false,
                    step_scale,
                );
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
                continue;
            }
            if !allow_kills {
                alive.push(p);
                continue;
            }
            if self.collide_physics(&p) {
                hit_count += 1;
                continue;
            }
            if let Some(idx) = self.hit_npc_index(&p) {
                hit_count += 1;
                let dmg = p.damage;
                let npc = &mut self.npcs[idx];
                npc.hp -= dmg;
                if npc.hp <= 0.0 {
                    let id = npc.id;
                    self.npcs.remove(idx);
                    self.score += KILL_SCORE;
                    log::info!(
                        "kill: npc #{} eliminated (wave {}) score={}",
                        id,
                        self.wave,
                        self.score
                    );
                }
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

    /// 投射物命中的 NPC 下标（命中球：中心在 NPC 头顶，半径 0.8）；未命中返回 None
    fn hit_npc_index(&self, p: &Projectile) -> Option<usize> {
        let (px, py, pz) = (p.position[0], p.position[1], p.position[2]);
        for (i, npc) in self.npcs.iter().enumerate() {
            let cx = npc.position[0];
            let cy = npc.position[1] + 0.8;
            let cz = npc.position[2];
            let dx = px - cx;
            let dy = py - cy;
            let dz = pz - cz;
            if dx * dx + dy * dy + dz * dz <= 0.8 * 0.8 {
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
            let slot_base = profile.count;
            let divisor = profile.count.max(1);
            for k in 0..profile.reinforcement_count {
                self.spawn_npc_ring(
                    player,
                    slot_base + k,
                    divisor,
                    self.wave,
                    profile.speed,
                    profile.hp,
                    profile.attack_range,
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
                    if self.wave >= WAVES_PER_LEVEL {
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
        let count = profile.count as usize;
        let speed = profile.speed;
        let hp = profile.hp;
        let attack_range = profile.attack_range;
        for i in 0..count {
            // Boss 波最后一只为主怪：替换常规小怪，max_hp 大 → 渲染侧体型/外观体现
            let (spd, hpx, rng) = match profile.boss {
                Some(b) if i + 1 == count => (b.speed, b.hp, b.attack_range),
                _ => (speed, hp, attack_range),
            };
            let id = self.spawn_npc_ring(player, i as u32, count as u32, n, spd, hpx, rng);
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
    ) -> usize {
        let tau = std::f32::consts::TAU;
        let angle = slot as f32 * (tau / divisor.max(1) as f32) + wave_n as f32 * 0.37;
        let radius = 40.0 + 40.0 * ((slot * 7 + wave_n * 3) % 5) as f32 / 4.0;
        let x = (player.x + angle.cos() * radius).clamp(-250.0, 250.0);
        let z = (player.z + angle.sin() * radius).clamp(-250.0, 250.0);
        // 出生点避开障碍盒（网格阻挡格）：沿径向向外推，最多 8 步（每步 4m），确定性
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
        let x = sx.clamp(-250.0, 250.0);
        let z = sz.clamp(-250.0, 250.0);
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
        });
        id
    }

    /// 推进 NPC：感知 → 状态机 → A* 路径 → 移动 → 地形高度
    fn update_ai(&mut self, dt: f32, camera: &Camera) {
        let player = camera.position();
        let grid = self.grid.clone();
        let flank_chance = wave_profile(self.wave).flank_chance;
        for i in 0..self.npcs.len() {
            let (npc, time) = (&mut self.npcs[i], self.time);
            let dx = npc.position[0] - player.x;
            let dz = npc.position[2] - player.z;
            let dist = (dx * dx + dz * dz).sqrt();
            let prev = npc.state_machine.state();
            npc.perception = NpcPerception {
                enemy_visible: dist < NPC_SIGHT,
                enemy_in_range: dist < npc.attack_range,
                start_patrol: prev == NpcState::Idle,
                patrol_finished: false,
            };
            let state = npc.state_machine.update(npc.perception);
            advance_npc(npc, state, &player, &grid, time, dt, flank_chance, self.wave);
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
        if self.time - self.ai_log_time >= 1.0 {
            self.ai_log_time = self.time;
            let mut counts = [0u32; 4];
            for npc in &self.npcs {
                counts[npc.state_machine.state() as usize] += 1;
            }
            log::info!(
                "ai: npcs={} idle={} patrol={} chase={} attack={}",
                self.npcs.len(),
                counts[0],
                counts[1],
                counts[2],
                counts[3]
            );
        }
        // 攻击态 NPC 对玩家造成伤害（1 秒一次），驱动 HUD 血条
        // 伤害值取当前有效波次的 dps（Boss 波更高，见 wave_profile）
        let dps = wave_profile(self.effective_wave(self.wave)).dps;
        if self.time - self.last_damage_time >= 1.0
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
                self.game_state = GameState::GameOver;
                log::info!(
                    "game: player down, score={} wave={} (GameOver: gameplay frozen, projectiles coast without kills)",
                    self.score,
                    self.wave
                );
            }
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

    /// 音频：测试音循环播放，tick 每帧运行且 voice 持续存在
    #[test]
    fn audio_mixer_runs_with_voice() {
        let mut game = Game::new();
        assert!(
            game.audio.mixer().voice_count() >= 1,
            "test tone should be playing"
        );
        let cam = Camera::new();
        for _ in 0..60 {
            game.update(1.0 / 60.0, &cam);
        }
        assert!(
            game.audio.mixer().voice_count() >= 1,
            "looping voice should persist"
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
        let before = game.firearm.magazine();
        assert!(before < 30, "弹匣应消耗过");
        game.request_reload();
        assert!(game.firearm.is_reloading(), "R 应开始换弹");
        for _ in 0..200 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(!game.firearm.is_reloading(), "换弹应完成");
        assert_eq!(game.firearm.magazine(), 30, "换弹后弹匣应补满");
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
}

/// 按状态推进单个 NPC：目标选择 → A* 寻路 → 沿路径移动 → 地形高度采样
fn advance_npc(
    npc: &mut Npc,
    state: NpcState,
    player: &glam::Vec3,
    grid: &GridMap,
    time: f32,
    dt: f32,
    flank_chance: f32,
    wave: u32,
) {
    // 无路径（或已走完）时按状态选择目标
    if npc.path.is_empty() || npc.path_index >= npc.path.len() {
        let goal = match state {
            NpcState::Chase => {
                let player_g = world_to_grid(player.x, player.z);
                let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                if should_flank(flank_chance, npc.id as u32, wave) {
                    // 包抄：沿 玩家→NPC 轴 的垂直方向偏移 2 格（8m），从侧翼逼近
                    let side = if npc.id % 2 == 0 { 1 } else { -1 };
                    flank_goal(grid, player_g, npc_g, side, 2)
                } else {
                    player_g
                }
            }
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

    if state == NpcState::Attack {
        // 攻击态原地站定
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
        let step = npc.speed * dt;
        npc.position[0] += dx / d * step;
        npc.position[2] += dz / d * step;
    }
    npc.position[1] = terrain_height_at(npc.position[0], npc.position[2]);
}
