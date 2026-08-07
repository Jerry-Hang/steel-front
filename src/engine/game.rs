//! 游戏运行时中枢
//!
//! 把 weapons / ai / physics / ui / audio / network 模块接进主循环：
//! - 每帧 `update(dt, camera, fire)` 推进物理、武器、AI、音频、网络
//! - 渲染前由 main.rs 取 HUD quad 列表与光照 uniform
//!
//! 本文件只做模块间编排与少量胶水逻辑，具体算法仍留在各模块内。

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::audio::{AudioClip, AudioListener, AudioPlayer, AudioSource, Channel, SilentSink};
use crate::net::{Client, NetworkMessage, PlayerState, Server};
use super::camera::Camera;
use super::ai::{find_path, GridMap, GridPos, NpcPerception, NpcState, NpcStateMachine};
use super::physics::{self, Body, CollisionEvent, CollisionListener, SphereBody, Vec3 as Pv};
use super::renderer::terrain_height_at;
use super::window::{WINDOW_HEIGHT, WINDOW_WIDTH};
use super::weapons::{Projectile, ProjectileWeapon, Weapon};
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
    /// 主武器（投射物步枪）
    rifle: ProjectileWeapon,
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
    /// 波间倒计时（清空后 3 秒刷下一波）
    wave_timer: f32,
    /// 击杀累计得分
    score: u64,
    /// 下一个 NPC 全局 id（出生用，保证巡逻相位唯一）
    next_npc_id: u32,
    /// 上次状态日志时间（1 秒一条 game: wave=...）
    last_status_log: f32,
}

impl Game {
    /// 创建游戏中枢：初始化物理演示场景
    pub fn new() -> Self {
        let mut world = physics::World::new();
        world.gravity = 9.8;
        // 地形中央 60×60 区域已压平到 y=0；演示刚体放在场地角落
        world.ground_y = 0.0;
        // 演示刚体远离原点（>150m）：轨道相机射线必过原点，放在中心区会拦截射向 NPC 的投射物
        for (x, z) in [(120.0, 120.0), (140.0, 130.0), (125.0, 145.0)] {
            world
                .bodies
                .push(Body::new(Pv::new(x, 3.0, z), Pv::new(1.2, 1.2, 1.2)));
        }
        for (x, z) in [(135.0, 110.0), (110.0, 135.0)] {
            let mut sphere = SphereBody::new(Pv::new(x, 4.0, z), 1.0);
            sphere.restitution = 0.5;
            world.spheres.push(sphere);
        }
        let event_buf = Arc::new(Mutex::new(Vec::new()));
        world.add_listener(Box::new(EventBuffer(event_buf.clone())));
        // AI 导航网格：加两块障碍簇，让 A* 有路可绕
        let mut grid = GridMap::new(GRID_SIZE, GRID_SIZE);
        for (bx, bz) in [(48, 48), (70, 84)] {
            for dx in 0..3 {
                for dz in 0..3 {
                    grid.block(GridPos::new(bx + dx, bz + dz));
                }
            }
        }
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
        log::info!(
            "physics: {} AABB bodies, {} spheres, gravity={}, ground_y={}",
            world.bodies.len(),
            world.spheres.len(),
            world.gravity,
            world.ground_y
        );
        Self {
            world,
            collisions: Vec::new(),
            total_collisions: 0,
            time: 0.0,
            last_dt: 0.0,
            event_buf,
            last_event_log_time: 0.0,
            rifle: ProjectileWeapon::new("M1 Rifle", 25.0, 3.0, 200.0, 60.0, 5.0),
            projectiles: Vec::new(),
            fire_cooldown: 0.0,
            shots: 0,
            hits: 0,
            grid,
            npcs,
            ai_log_time: 0.0,
            last_damage_time: 0.0,
            hud: HudState::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
            frames: 0,
            fps_window_start: Instant::now(),
            audio_sample_rate: 48_000,
            audio: {
                let player = AudioPlayer::new(SilentSink::new(48_000, 2));
                let mut player = player;
                // 440Hz 测试音（0.5 秒）循环播放，声源放场地中央
                if let Some(clip) = make_test_clip(48_000) {
                    player.mixer_mut().set_master(0.8);
                    player.mixer_mut().set_channel_volume(Channel::Sfx, 1.0);
                    player.mixer_mut().play(
                        Arc::new(clip),
                        AudioSource::new(glam::Vec3::new(0.0, 2.0, 0.0), 1.0),
                        Channel::Sfx,
                        true,
                    );
                }
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
        }
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

    /// 开始一局：复位血量/弹药/分数/波次，清掉残留 NPC 后生成第 1 波，进入 Playing
    fn start_run(&mut self, player: &glam::Vec3) {
        self.hud.health = self.hud.max_health;
        self.hud.ammo = self.hud.max_ammo;
        self.score = 0;
        self.wave = 1;
        self.wave_timer = 0.0;
        self.fire_cooldown = 0.0;
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
        self.hud.screen = match self.game_state {
            GameState::StartMenu => HudScreen::Start,
            GameState::GameOver => HudScreen::GameOver,
            GameState::Playing => HudScreen::Game,
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
            "collisions: {}  hits: {}",
            self.total_collisions(),
            self.hits()
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
        self.fire_cooldown = self.rifle.fire_interval();
        self.projectiles.push(self.rifle.fire(origin, direction));
        self.shots += 1;
        log::info!(
            "weapons: shot #{} ({} alive)",
            self.shots,
            self.projectiles.len()
        );
        true
    }

    /// 累计命中数（供 UI / 日志）
    pub fn hits(&self) -> u64 {
        self.hits
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
                    self.wave += 1;
                    self.spawn_wave(self.wave, player);
                }
            }
        }
    }

    /// 生成第 n 波敌人：数量/速度/血量随波次递进，环形出生在玩家周围。
    ///
    /// 出生前清掉残留存活 NPC，保证新旧波不共存。
    fn spawn_wave(&mut self, n: u32, player: &glam::Vec3) {
        if !self.npcs.is_empty() {
            log::info!(
                "wave: purged {} leftover npcs before wave {}",
                self.npcs.len(),
                n
            );
            self.npcs.clear();
        }
        let count = (4 + 2 * n).min(24) as usize;
        let speed = (4.0 * (1.0 + 0.06 * (n as f32 - 1.0))).min(8.0);
        let hp = 100.0 + 20.0 * (n as f32 - 1.0);
        let tau = std::f32::consts::TAU;
        for i in 0..count {
            let angle = i as f32 * (tau / count as f32) + n as f32 * 0.37;
            let radius = 40.0 + 40.0 * ((i as u32 * 7 + n * 3) % 5) as f32 / 4.0;
            let x = (player.x + angle.cos() * radius).clamp(-250.0, 250.0);
            let z = (player.z + angle.sin() * radius).clamp(-250.0, 250.0);
            let id = self.next_npc_id as usize;
            self.next_npc_id += 1;
            let y = terrain_height_at(x, z);
            log::info!("wave: npc #{} spawn ({:.1}, {:.1}, {:.1})", id, x, y, z);
            self.npcs.push(Npc {
                id,
                position: [x, y, z],
                speed,
                attack_range: 12.0,
                home: [x, z],
                state_machine: NpcStateMachine::new(),
                perception: NpcPerception::default(),
                path: Vec::new(),
                path_index: 0,
                hp,
                max_hp: hp,
            });
        }
        log::info!(
            "wave: wave {} spawned {} enemies (speed={:.1} hp={:.0})",
            n,
            count,
            speed,
            hp
        );
    }

    /// 推进 NPC：感知 → 状态机 → A* 路径 → 移动 → 地形高度
    fn update_ai(&mut self, dt: f32, camera: &Camera) {
        let player = camera.position();
        let grid = self.grid.clone();
        for i in 0..self.npcs.len() {
            let (npc, time) = (&mut self.npcs[i], self.time);
            let dx = npc.position[0] - player.x;
            let dz = npc.position[2] - player.z;
            let dist = (dx * dx + dz * dz).sqrt();
            npc.perception = NpcPerception {
                enemy_visible: dist < NPC_SIGHT,
                enemy_in_range: dist < npc.attack_range,
                start_patrol: npc.state_machine.state() == NpcState::Idle,
                patrol_finished: false,
            };
            let state = npc.state_machine.update(npc.perception);
            advance_npc(npc, state, &player, &grid, time, dt);
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
        if self.time - self.last_damage_time >= 1.0
            && self.game_state == GameState::Playing
            && self
                .npcs
                .iter()
                .any(|n| n.state_machine.state() == NpcState::Attack)
            && self.hud.health > 0.0
        {
            self.hud.health = (self.hud.health - 5.0).max(0.0);
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

    /// 演示刚体应在地面附近落地并静止
    #[test]
    fn physics_demo_bodies_settle() {
        let mut game = Game::new();
        assert_eq!(game.world.bodies.len(), 3);
        assert_eq!(game.world.spheres.len(), 2);
        for _ in 0..120 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        for body in &game.world.bodies {
            assert!(body.grounded, "body should be grounded");
            let bottom = body.position.y - body.half_extents.y;
            assert!((bottom - game.world.ground_y).abs() < 0.01, "body should rest on ground");
        }
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
        for _ in 0..10 {
            if let Ok(Some((msg, from))) = demo.server.recv() {
                if let NetworkMessage::Join { player_id, .. } = &msg {
                    got_join = *player_id == 0;
                    assert!(demo.server.handle_join(from, "local".into()).is_ok());
                }
            }
        }
        assert!(got_join, "server should receive the Join sent at init");
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
) {
    // 无路径（或已走完）时按状态选择目标
    if npc.path.is_empty() || npc.path_index >= npc.path.len() {
        let goal = match state {
            NpcState::Chase => world_to_grid(player.x, player.z),
            NpcState::Attack => world_to_grid(npc.position[0], npc.position[2]),
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

/// 生成 440Hz 正弦测试音（0.5 秒，双声道），用于驱动混音链路
fn make_test_clip(sample_rate: u32) -> Option<AudioClip> {
    let frames = (sample_rate as f32 * 0.5) as usize;
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f32 / sample_rate as f32;
        let v = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.4;
        samples.push(v);
        samples.push(v);
    }
    AudioClip::new(samples, sample_rate, 2)
}
