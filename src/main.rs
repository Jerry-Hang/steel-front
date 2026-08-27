//! 钢铁前线 (Steel Front) - 程序入口
//!
//! 游戏主循环：
//! 1. 初始化窗口（winit）
//! 2. 初始化 Vulkan 渲染器（ash）
//! 3. 事件循环处理输入
//! 4. 每帧更新相机并渲染

mod engine;
mod audio;
mod audio_out;
mod llm_cmd;
mod net;
mod ui;
mod config;
mod perf_log;

use std::time::{Duration, Instant};

use winit::{
    application::ApplicationHandler,
    event::{
        DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent,
    },
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use engine::camera::{Camera, CameraMode, KeyState};
use engine::ai::Team;
use engine::game::{Game, GameState};
use engine::renderer::{QualityPreset, Renderer};
use engine::window;
use net::{Client, Server};
use ui::{BindingAction, KeyBindings, RESOLUTIONS};
use winit::window::CursorGrabMode;

/// 绝对位置路径（CursorMoved）单次位移最大像素：超过视为光标传送伪事件
/// （X 服务端 warp/焦点切换跳变），跳过该事件并重基准 last_cursor，
/// 防止第一人称视角跳变/自转。仅用于非捕获态拖拽路径（菜单/设置预览）。
/// 捕获态视角由 DeviceEvent::MouseMotion（XInput2 raw 相对增量）驱动，
/// 不适用此像素阈值（raw 位移单位是设备原始计数，可远大于屏幕像素）。
const MAX_LOOK_DELTA_PX: f64 = 512.0;

/// raw 相对增量单事件上限：物理手速（1000Hz 采样下单事件 ≤ 几十计数）
/// 不可能达到的量级；超过视为残留 warp 回声（X 服务端 warp 在个别栈上
/// 也会产生 raw motion），跳过防止反馈环自转。
const MAX_RAW_LOOK_DELTA: f64 = 1024.0;

/// 帧率上限（present 节流）：0 = 无上限（压测模式，主循环全速跑以暴露渲染瓶颈）。
/// 设回正数（如 300）即恢复帧率门控。
const MAX_FPS: u64 = 0;

/// 环境变量真值解析（"1"/"true"/"on" = 真；其余为假）
fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|v| matches!(v.as_str(), "1" | "true" | "on" | "TRUE" | "ON" | "True"))
        .unwrap_or(false)
}

/// 环境变量浮点读取（解析失败返回 None）
fn env_f32(name: &str) -> Option<f32> {
    std::env::var(name).ok().and_then(|s| s.parse::<f32>().ok())
}
/// 单帧预算（纳秒；MAX_FPS=0 时为 0，不做 sleep/spin 节流）
const FRAME_BUDGET: Duration =
    Duration::from_nanos(if MAX_FPS > 0 { 1_000_000_000 / MAX_FPS } else { 0 });

/// 游戏应用主管理结构
struct GameApp {
    /// winit 窗口
    window: Option<Window>,
    /// Vulkan 渲染器
    renderer: Option<Renderer>,
    /// FPS 相机
    camera: Camera,
    /// 键盘按键状态
    key_state: KeyState,
    /// 鼠标左键是否按住（拖拽轨道旋转）
    dragging: bool,
    /// 鼠标右键是否按住（飞行模式拖拽转视角）
    right_dragging: bool,
    /// 开镜瞄准（右键按住；第一人称 FPS：准星收窄 + 枪模居中 + FOV 缩小）
    ads_active: bool,
    /// 开镜混合度 0..1（腰射→开镜 0.2s 指数平滑；驱动枪模锚点插值）
    ads_blend: f32,
    /// 最近一次开火时刻（anim_clock；枪模后坐用一次性"上抬→回落"脉冲）
    last_shot_at: f32,
    /// 伤害飘字列表：(伤害, 剩余秒)；命中时 push，0.6s 淡出（塔克夫式受击反馈）
    hit_damage_popups: Vec<(f32, f32)>,
    /// 上一帧光标位置（屏幕坐标）
    last_cursor: (f64, f64),
    /// 上一帧时间戳（用于 delta_time 计算）
    last_frame: Instant,
    /// 上一帧 update+render 总耗时（微秒，性能日志用）
    last_cycle_us: u64,
    /// 采集模式帧率上限（0 = 不限；LLM 模式 90）
    llm_cap_fps: f32,
    /// 上一帧 update（逻辑）耗时（微秒，性能日志用）
    last_update_us: u64,
    /// 上一帧 render（渲染提交）耗时（微秒，性能日志用）
    last_render_us: u64,
    /// 是否请求开火（按住状态，Auto 模式持续开火；抬起复位）
    fire_requested: bool,
    /// 开火按下瞬间（edge 触发：Semi/Burst3 模式用；update 消费后复位）
    fire_edge: bool,
    /// 光标是否已捕获（Playing 下鼠标视角）
    cursor_captured: bool,
    /// 捕获模式是否为系统级 Locked（raw 相对增量驱动视角）；
    /// false = 回退 Confined/无 grab，走绝对位置路径（WSLg/Xwayland 实测：
    /// 真实物理鼠标只产生 CursorMoved 绝对位置，不产生 XI_RawMotion raw 事件）
    cursor_locked: bool,
    /// 绝对位置路径：是否已收到首个真实指针位置基准（捕获瞬间未知指针位置，
    /// 首个事件只作基准，避免把"捕获前指针到中心差量"当视角位移）
    abs_baseline_valid: bool,
    /// 窗口是否聚焦（失焦时释放捕获，防止卡视角）
    focused: bool,
    /// 捕获瞬间回中 warp 的回声吞噬窗口：recenter 后 150ms 内到达的下一个
    /// CursorMoved / DeviceEvent::MouseMotion 视为 warp 回声（只作新基准、
    /// 不应用视角位移），防止把"捕获前光标到窗口中心的差量"当成视角位移。
    recenter_pending_until: Option<Instant>,
    /// 上次相机参数日志时间（1 秒一条，冒烟/调试用）
    last_cam_log: Instant,
    /// 游戏运行时中枢（物理/武器/AI/UI/音频/网络）
    game: Game,
    /// 程序是否正在运行
    running: bool,
    /// 事件循环代理（菜单点击退出用：请求事件循环退出）
    event_proxy: Option<winit::event_loop::EventLoopProxy<()>>,
    /// 配置中是否显式保存过分辨率（false = 首次运行，窗口创建时按显示器宽高比选默认）
    resolution_explicit: bool,
    /// NPC 动画时钟（秒，每帧累加 delta_time；驱动步态/后坐相位）
    anim_clock: f32,
    /// 上一帧存活 NPC 快照：id → (位置, 朝向, 阵营色)（尸体跟踪：本帧消失的 id 记入 corpses）
    last_npc_snapshot: std::collections::HashMap<usize, ([f32; 3], f32, [f32; 4])>,
    /// 上一帧 FPS（性能日志用）
    last_fps: f64,
    /// 倒地尸体：(位置, 朝向, 阵营色, 已存留秒数)；上限 20 具，超过 10 秒消退
    corpses: Vec<([f32; 3], f32, [f32; 4], f32)>,
    /// 枪口焰/弹壳粒子（0=枪口焰无重力淡出，1=弹壳重力落地）；渲染走 emissive 通道
    particles: Vec<Particle>,
    /// 性能日志（每次启动一份，logs/perf_*.log）
    perf_log: Option<perf_log::PerfLog>,
    /// 命令输入窗口是否打开（Enter 开关，Minecraft 风格左下角输入框）
    command_open: bool,
    /// 枪械检视模式（RV3D_INSPECT=武器编号 1-35）：只展示枪模，Orbit 相机拖拽查看
    inspect_weapon: Option<usize>,
    inspect_armed: bool,
    cam_logged: bool,
    /// RV3D_CAM 调试机位（飞行模式固定位姿；地图/场景检查用）
    cam_override: Option<(glam::Vec3, f32, f32)>,
    /// 命令输入缓冲（当前只接受数字，回车切换武器）
    command_buf: String,
    /// 当前武器枪模缓存（构建含光照烘焙，切枪时才重建；帧内只做视空间变换）
    gun_mesh_cache: Option<(String, crate::engine::guns::GunMesh)>,
    /// 导入的 GLB 枪模（assets/guns/*.glb → 烘焙顶点；无则回退程序化枪模）
    gun_glb: Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)>,
    /// 延迟自动切枪（测试用）：(目标武器号, 触发时刻)
    switch_weapon_at: Option<(usize, f32)>,
}

/// 视觉粒子：枪口焰（无重力，快速淡出）+ 弹壳（重力下落，落地消散）
struct Particle {
    pos: [f32; 3],
    vel: [f32; 3],
    age: f32,
    life: f32,
    size: f32,
    tint: [f32; 4],
    kind: u8, // 0=枪口焰 1=弹壳
}

impl GameApp {
    /// 创建游戏应用实例
    fn new() -> Self {
        let mut game = Game::new();
        // 加载持久化配置（键位/音量/灵敏度）；文件缺失回退默认，见 config.rs
        let cfg = config::load();
        game.hud.volume = cfg.volume;
        game.hud.music_volume = cfg.music_volume;
        game.hud.sensitivity = cfg.sensitivity;
        game.hud.key_bindings = cfg.bindings;
        // 分辨率索引：显式保存过 → 用配置值；首次运行 → 0（resumed() 按显示器宽高比重选）
        game.hud.resolution_index = if cfg.resolution_explicit {
            RESOLUTIONS
                .iter()
                .position(|&r| r == cfg.resolution)
                .unwrap_or(0) as u8
        } else {
            0
        };
        // 画质索引与 ui.rs 选项表对齐；配置异常值回退默认
        game.hud.quality_index = cfg.quality.min(2) as u8;
        Self {
            window: None,
            renderer: None,
            camera: Camera::new(),
            key_state: KeyState::new(),
            dragging: false,
            right_dragging: false,
            ads_active: false,
            ads_blend: 0.0,
            last_shot_at: -1.0,
            hit_damage_popups: Vec::new(),
            last_cursor: (0.0, 0.0),
            last_frame: Instant::now(),
            last_cycle_us: 0,
            llm_cap_fps: {
                // 全局帧率上限（2026-08-23 防 GPU 驻停留态 device lost）：
                // RV3D_FPS 覆盖；默认 240；LLM 采集模式 90（留 GPU 余量）
                let llm_on = std::env::var("RV3D_LLM")
                    .map(|v| !(v.is_empty() || v == "0" || v == "off"))
                    .unwrap_or(false);
                let cap = env_f32("RV3D_FPS").unwrap_or(if llm_on { 90.0 } else { 240.0 });
                cap.max(20.0)
            },
            last_update_us: 0,
            last_render_us: 0,
            fire_requested: false,
            fire_edge: false,
            cursor_captured: false,
            cursor_locked: false,
            abs_baseline_valid: false,
            focused: true,
            recenter_pending_until: None,
            last_cam_log: Instant::now(),
            game,
            running: true,
            event_proxy: None,
            resolution_explicit: cfg.resolution_explicit,
            anim_clock: 0.0,
            last_npc_snapshot: std::collections::HashMap::new(),
            last_fps: 0.0,
            corpses: Vec::new(),
            particles: Vec::new(),
            perf_log: None,
            command_open: false,
            // 检视模式：--inspect=N 或 --inspect N 命令行参数优先，其次 RV3D_INSPECT 环境变量
            inspect_weapon: {
                let mut args = std::env::args().skip(1);
                let mut parsed: Option<usize> = None;
                while let Some(a) = args.next() {
                    if let Some(v) = a.strip_prefix("--inspect=") {
                        parsed = v.parse().ok();
                    } else if a == "--inspect" {
                        parsed = args.next().and_then(|v| v.parse().ok());
                    }
                }
                parsed
                    .or_else(|| {
                        std::env::var("RV3D_INSPECT")
                            .ok()
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .filter(|&n| (1..=35).contains(&n))
            },
            inspect_armed: false,
            cam_logged: false,
            cam_override: std::env::var("RV3D_CAM").ok().and_then(|s| {
                let mut it = s.split(':');
                let _mode = it.next()?; // 模式标记（fly）
                let pos = it.next()?.trim();
                let rot = it.next()?.trim();
                let p: Vec<f32> = pos.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                let r: Vec<f32> = rot.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                if p.len() == 3 && r.len() == 2 {
                    Some((
                        glam::Vec3::new(p[0], p[1], p[2]),
                        r[0].to_radians(),
                        r[1].to_radians(),
                    ))
                } else {
                    None
                }
            }),
            command_buf: String::new(),
            gun_mesh_cache: None,
            gun_glb: Self::load_gun_glb(),
            switch_weapon_at: None,
        }
    }

    /// 更新逻辑（每帧调用）
    fn update(&mut self) {
        // RV3D_CAM=fly:x,y,z:yaw_deg,pitch_deg：调试固定机位（地图/场景检查用）
        if self.cam_override.is_some() {
            // 仍推进帧时间/HUD FPS（避免调试机位下 HUD 恒 0 显像为“卡死”）
            let now = Instant::now();
            let dt = now.duration_since(self.last_frame).as_secs_f32();
            self.last_frame = now;
            if dt > 1e-6 {
                self.last_fps = 1.0 / dt.min(0.1) as f64;
            }
            self.anim_clock += dt.min(0.1);
            self.camera.mode = CameraMode::Flight;
            if let Some((p, yaw, pitch)) = self.cam_override {
                self.camera.set_flight_pos(p);
                self.camera.yaw = yaw;
                self.camera.pitch = pitch;
            }
            return;
        }
        // 枪械检视模式：不跑游戏逻辑，仅 Orbit 相机绕枪模（鼠标拖拽旋转/滚轮缩放，
        // 事件处理已有 orbit 控制）；首次进入设置相机朝向。
        if self.inspect_weapon.is_some() {
            self.camera.mode = CameraMode::Orbit;
            if !self.inspect_armed {
                self.inspect_armed = true;
                self.camera.target = glam::Vec3::new(0.0, 1.0, 0.0);
                self.camera.yaw = std::f32::consts::FRAC_PI_2; // 正侧视：枪口朝左
                self.camera.pitch = 0.08;
                self.camera.fov = 45.0_f32.to_radians();
                // 产品照式取景：远距离 + 长焦（弱透视，近远端大小接近，同真枪照片）
                self.camera.distance = 2.0;
                if let Some(n) = self.inspect_weapon {
                    if let Some(spec) = crate::engine::weapon_data::spec_by_number(n) {
                        if let Some(gm) = crate::engine::guns::gun_mesh_by_key(spec.key) {
                            let mut mn = [f32::MAX; 3];
                            let mut mx = [f32::MIN; 3];
                            for v in &gm.verts {
                                for i in 0..3 {
                                    mn[i] = mn[i].min(v.pos[i]);
                                    mx[i] = mx[i].max(v.pos[i]);
                                }
                            }
                            let e = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
                            let diag = (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt();
                            // 距离 = 4.5× 对角线：近端/远端大小差 <25%（之前 1.36m 时差达 2 倍，
                            // 广角微距式变形就是 1-4/1-5 里“这不像枪”的根源）
                            let dist = (diag * 4.5).max(2.0);
                            self.camera.distance = dist;
                            // 长焦 fov：按距离反推，保证整枪入画（1.15 余量）
                            self.camera.fov = (2.0 * ((diag * 0.5 * 1.15) / dist).atan())
                                .to_degrees()
                                .to_radians();
                            log::info!(
                                "inspect: bbox=[{:.3},{:.3},{:.3}]..[{:.3},{:.3},{:.3}] ext=[{:.3},{:.3},{:.3}] diag={:.3} dist={:.3}",
                                mn[0], mn[1], mn[2], mx[0], mx[1], mx[2],
                                e[0], e[1], e[2], diag, self.camera.distance
                            );
                        }
                    }
                }
                log::info!(
                    "inspect: 枪械检视模式（武器 #{}）——拖拽旋转 / 滚轮缩放",
                    self.inspect_weapon.unwrap()
                );
            }
            return;
        }
        // RV3D_AUTOSTART=1：测试用自动开始（绕过键盘，进 Playing 复现/冒烟）
        use std::sync::atomic::{AtomicBool, Ordering};
        static AUTO_STARTED: AtomicBool = AtomicBool::new(false);
        if !AUTO_STARTED.swap(true, Ordering::SeqCst)
            && env_truthy("RV3D_AUTOSTART")
        {
            let st = self.game.state();
            if st == GameState::StartMenu || st == GameState::LoadingMap {
                log::info!("autostart: RV3D_AUTOSTART=1 自动开始");
                self.game.on_any_key(&self.camera.position());
            }
            // RV3D_SWITCH_WEAPON=n：进入后自动切到 n 号武器（复现切枪崩溃用）；
            // RV3D_SWITCH_WEAPON_AFTER=秒：延迟切枪（模拟玩一会儿再切）
            let after = env_f32("RV3D_SWITCH_WEAPON_AFTER");
            let (target, switch_at) = match std::env::var("RV3D_SWITCH_WEAPON") {
                Ok(n) => (n.parse::<usize>().ok(), after.unwrap_or(0.0)),
                Err(_) => (None, 0.0),
            };
            if let Some(n) = target {
                if switch_at <= 0.0 {
                    log::info!("autostart: 自动切枪 #{}", n);
                    self.game.switch_weapon(n.saturating_sub(1));
                } else {
                    self.switch_weapon_at = Some((n, self.anim_clock + switch_at));
                }
            }
        }
        // 延迟自动切枪（测试用）
        if let Some((n, at)) = self.switch_weapon_at {
            if self.anim_clock >= at && self.game.state() == GameState::Playing {
                log::info!("autostart: 延迟切枪 #{}", n);
                self.game.switch_weapon(n.saturating_sub(1));
                self.switch_weapon_at = None;
            }
        }
        // RV3D_DIAG_NPC_FRONT=1：把 npc[0] 放到玩家正前方 20m 固定（弹道诊断隔离实验）
        static DIAG_NPC_FRONT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *DIAG_NPC_FRONT.get_or_init(|| {
            env_truthy("RV3D_DIAG_NPC_FRONT")
        }) && self.game.state() == GameState::Playing
        {
            // 相机 yaw=0 时 forward 方向（与 fire 弹道同源），NPC 放前方 20m
            let fwd = self.camera.forward();
            let pos = self.camera.position();
            let nx = pos.x + fwd.x * 20.0;
            let nz = pos.z + fwd.z * 20.0;
            let ny = crate::engine::renderer::terrain_height_at(nx, nz);
            self.game.diag_place_npc([nx, ny, nz]);
        }
        // RV3D_AUTOFIRE=1：自动开火（诊断射击链路：fire 是否发射、弹道是否命中）
        static AUTO_FIRE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *AUTO_FIRE.get_or_init(|| env_truthy("RV3D_AUTOFIRE"))
            && self.game.state() == GameState::Playing
        {
            self.fire_requested = true;
        }
        // 同步光标捕获状态（Playing + 聚焦 = 捕获；菜单/结算/失焦 = 释放）
        self.sync_cursor();

        // 计算帧时间差
        let now = Instant::now();
        let delta_time = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        // 确保 delta_time 不会太大（防止卡顿时大跳）
        let delta_time = delta_time.min(0.1);
        if delta_time > 1e-6 {
            self.last_fps = 1.0 / delta_time as f64;
        }
        // NPC 动画时钟（步态/后坐相位）与尸体老化
        self.anim_clock += delta_time;
        for c in self.corpses.iter_mut() {
            c.3 += delta_time;
        }
        self.corpses.retain(|c| c.3 < 10.0); // 尸体 10 秒后消退
        while self.corpses.len() > 20 {
            self.corpses.remove(0); // 上限 20 具（NPC 槽位 1024 = 146 人 × 7 段）
        }
        // 粒子推进：弹壳重力下落 + 落地停止；超龄移除
        for p in self.particles.iter_mut() {
            p.age += delta_time;
            if p.kind == 1 {
                p.vel[1] -= 18.0 * delta_time; // 弹壳重力
                p.pos[0] += p.vel[0] * delta_time;
                p.pos[1] += p.vel[1] * delta_time;
                p.pos[2] += p.vel[2] * delta_time;
                if p.pos[1] <= 0.05 {
                    p.pos[1] = 0.05;
                    p.vel = [0.0, 0.0, 0.0];
                    p.life = p.life.min(0.4); // 落地后最多停留 0.4s
                }
            }
        }
        self.particles.retain(|p| p.age < p.life);
        while self.particles.len() > 48 {
            self.particles.remove(0); // 上限 48 颗粒子
        }

        // 更新相机（双模式：轨道/飞行，含惯性速度与边界 clamp）
        self.camera.update(&self.key_state, delta_time);
        // 开镜瞄准：FOV 平滑过渡（70° 腰射 → 55° 开镜，步枪 ADS 轻微收窄而非狙击 zoom）
        // + 锚点混合度（枪模腰射右下 → 开镜居中，0.2s 指数平滑）
        let ads_target = if self.ads_active {
            55.0_f32.to_radians()
        } else {
            70.0_f32.to_radians()
        };
        let fov_delta = ads_target - self.camera.fov;
        if fov_delta.abs() > 1e-4 {
            self.camera.fov += fov_delta * (1.0 - (-10.0 * delta_time).exp());
        }
        let ads_blend_target = if self.ads_active { 1.0 } else { 0.0 };
        self.ads_blend +=
            (ads_blend_target - self.ads_blend) * (1.0 - (-10.0 * delta_time).exp());
        // 开镜状态硬化：非 Playing/菜单/设置打开时强制复位（防右键状态卡死 → 准星变小/消失）
        let ads_valid = self.ads_active
            && self.camera.mode == CameraMode::FirstPerson
            && self.game.state() == GameState::Playing
            && !self.game.settings_open()
            && !self.game.hud.esc_menu_open;
        self.game.hud.ads = ads_valid;
        // 小地图朝向（旋转地图使玩家前方朝上）
        self.game.hud.mm_yaw = self.camera.yaw;
        if !ads_valid {
            self.ads_active = false;
        }

        // 更新游戏逻辑（物理、武器、AI 等）
        // 先把本帧开火意图转发给网络层（客户端模式随 Input 上报服务端）
        self.game.set_net_fire(self.fire_requested);
        // V3.0 散射：开镜时散布缩小到 30%（腰射 100%）
        self.game.set_spread_scale(1.0 - self.ads_blend * 0.7);
        self.game.update(delta_time, &self.camera);

        // 基准挂钩：RV3D_BENCH_YAW / RV3D_BENCH_PITCH（度）每帧强制相机朝向，
        // 供性能基准固定视角用（与 RV3D_NPC_SCALE / RV3D_STRESS_AI 同类的测试环境变量，
        // 不设置则完全不影响正常游玩）。鼠标/后坐力每帧会被覆盖，基准时无需 bot 拖视角。
        if let Ok(yaw) = std::env::var("RV3D_BENCH_YAW") {
            if let Ok(y) = yaw.parse::<f32>() {
                self.camera.yaw = y.to_radians();
            }
        }
        if let Ok(pitch) = std::env::var("RV3D_BENCH_PITCH") {
            if let Ok(p) = pitch.parse::<f32>() {
                self.camera.pitch = p.to_radians().clamp(
                    -crate::engine::camera::PITCH_LIMIT,
                    crate::engine::camera::PITCH_LIMIT,
                );
            }
        }

        // 第一人称：玩家身体位置 → 相机眼睛（FP 相机不自己移动），并同步灵敏度
        if self.camera.mode == CameraMode::FirstPerson {
            // 爆炸震屏：本帧抖动偏移叠加到眼睛位置（无震屏时偏移为 0）
            let mut eye = self.game.player_eye();
            let (sx, sz) = self.game.camera_shake_offset();
            eye.x += sx;
            eye.z += sz;
            self.camera.set_first_person_eye(eye);
            self.camera.set_mouse_sens(self.game.sensitivity_rads());
        }

        // 开火：按开火模式分发（Semi=edge 单发 / Burst3=edge 三连发 / Auto=按住连发）。
        // 按住状态 fire_requested 保持 true，由武器 fire_cooldown 控制射速。
        let pos = self.camera.position();
        let dir = self.camera.forward();
        let mut fired = 0u32;
        match self.game.fire_mode() {
            crate::engine::game::FireMode::Semi => {
                if self.fire_edge {
                    let ok = self
                        .game
                        .fire_player([pos.x, pos.y, pos.z], [dir.x, dir.y, dir.z]);
                    if ok {
                        fired = 1;
                        self.last_shot_at = self.anim_clock;
                    }
                }
            }
            crate::engine::game::FireMode::Burst3 => {
                if self.fire_edge {
                    fired = self
                        .game
                        .fire_burst_player([pos.x, pos.y, pos.z], [dir.x, dir.y, dir.z]);
                    if fired > 0 {
                        self.last_shot_at = self.anim_clock;
                    }
                }
            }
            crate::engine::game::FireMode::Auto => {
                if self.fire_requested {
                    let ok = self
                        .game
                        .fire_player([pos.x, pos.y, pos.z], [dir.x, dir.y, dir.z]);
                    if ok {
                        fired = 1;
                        self.last_shot_at = self.anim_clock;
                    }
                }
            }
        }
        self.fire_edge = false;
        // 枪口焰 + 弹壳粒子（每实际发射一发生成一组）
        for _ in 0..fired {
            let muzzle = [
                pos.x + dir.x * 0.5,
                pos.y - 0.25,
                pos.z + dir.z * 0.5,
            ];
            self.particles.push(Particle {
                pos: muzzle,
                vel: [0.0, 0.0, 0.0],
                age: 0.0,
                life: 0.09,
                size: 0.18,
                tint: [1.0, 0.75, 0.25, 1.0], // 橙黄枪口焰
                kind: 0,
            });
            self.particles.push(Particle {
                pos: muzzle,
                vel: [dir.z * 1.5 + 0.4, 2.2, -dir.x * 1.5], // 侧向抛出
                age: 0.0,
                life: 1.4,
                size: 0.06,
                tint: [0.72, 0.55, 0.18, 1.0], // 黄铜弹壳
                kind: 1,
            });
        }

        // 伤害飘字：本帧命中伤害入列（0.6s 衰减淡出）。
        // 同帧同值合并（霰弹一次开火 8 弹丸命中只显示一条伤害），
        // 上限 3 条滚动——超出丢最旧，新伤害补进来（不出现"一次命中刷屏"）。
        {
            let mut seen = std::collections::HashSet::new();
            for dmg in self.game.take_hit_damages() {
                if seen.insert(dmg.to_bits()) {
                    self.hit_damage_popups.push((dmg, 0.6));
                }
            }
            if self.hit_damage_popups.len() > 3 {
                let overflow = self.hit_damage_popups.len() - 3;
                self.hit_damage_popups.drain(0..overflow);
            }
        }
        // 衰减（iter_mut 可修改）→ 过滤（retain 只读判断，闭包参数为 &T）
        for (_, t) in self.hit_damage_popups.iter_mut() {
            *t -= delta_time;
        }
        self.hit_damage_popups.retain(|item| item.1 > 0.0);
        // 命中火花：本帧命中点在目标处生成小火花粒子（受击反馈增强）
        for hp in self.game.take_hit_points() {
            for _ in 0..5 {
                self.particles.push(Particle {
                    pos: hp,
                    vel: [
                        (hp[0] * 13.7).fract() * 2.0 - 1.0,
                        ((hp[0] + hp[2]) * 7.3).fract() * 1.4,
                        (hp[2] * 11.3).fract() * 2.0 - 1.0,
                    ],
                    age: 0.0,
                    life: 0.18,
                    size: 0.03,
                    tint: [1.0, 0.85, 0.3, 1.0], // 橙黄火花
                    kind: 0,
                });
            }
        }
        // 武器后坐力：取走本帧开火累计的 kick 施加到相机（指数衰减由 camera.update 处理）
        let (kick_pitch, kick_yaw) = self.game.drain_kick();
        if kick_pitch != 0.0 || kick_yaw != 0.0 {
            self.camera.add_recoil(kick_pitch, kick_yaw);
        }

        // 服务器模式：客户端输入视角驱动本机相机（快照权威视角；无客户端输入时保持本地视角）
        if let Some((yaw, pitch)) = self.game.net_look() {
            self.camera.yaw = yaw;
            self.camera.pitch = pitch;
        }

        // 相机参数日志（1 秒一条，冒烟断言 yaw/pitch 变化用）
        if self.last_cam_log.elapsed().as_secs_f32() >= 1.0 {
            let (yaw, pitch, dist) = self.camera.orbit_params();
            log::info!(
                "cam: yaw={:.1} pitch={:.1} dist={:.1} mode={:?} cycle_us={} update_us={} render_us={}",
                yaw.to_degrees(),
                pitch.to_degrees(),
                dist,
                self.camera.mode,
                self.last_cycle_us,
                self.last_update_us,
                self.last_render_us
            );
            self.last_cam_log = Instant::now();
        }
    }

    /// 设置面板鼠标点击：命中某行 → 选中该项（与 Tab 循环一致）；音量/灵敏度条内点击
    /// 按位置比例直接设值（x 比例 = 值）。布局必须与 ui.rs settings_elements 一致。
    fn settings_click(&mut self, mx: f32, my: f32) {
        let s = self.game.hud.ui_scale();
        let w = self.game.hud.screen_w;
        let h = self.game.hud.screen_h;
        let dw = w / s;
        let dh = h / s;
        let bar_w = (dw * 0.32).min(320.0);
        let bar_h = 20.0;
        let label_w = 160.0;
        let row_h = 34.0;
        let start_y = dh * 0.28;
        let left = dw * 0.5 - (label_w + bar_w + 16.0) * 0.5;
        let mx_d = mx / s;
        let my_d = my / s;
        // 音量/灵敏度/音乐三行：点行选中；点在条上按比例设值
        for i in 0..3usize {
            let y = start_y + i as f32 * row_h;
            if my_d >= y && my_d <= y + bar_h {
                self.game.hud.settings_selection = i as u8;
                if mx_d >= left + label_w && mx_d <= left + label_w + bar_w {
                    let ratio = ((mx_d - (left + label_w)) / bar_w).clamp(0.0, 1.0);
                    match i {
                        0 => self.game.hud.volume = ratio,
                        1 => self.game.hud.sensitivity = ratio,
                        _ => self.game.hud.music_volume = ratio,
                    }
                    log::info!("settings: 鼠标点击设定 行{} = {:.0}%", i, ratio * 100.0);
                } else {
                    log::info!("settings: 鼠标选中行 {}", i);
                }
                return;
            }
        }
        // 分辨率/画质行：点击选中
        for i in 0..2usize {
            let row = 3 + i as u8;
            let y = start_y + row as f32 * row_h;
            if my_d >= y && my_d <= y + bar_h {
                self.game.hud.settings_selection = row;
                log::info!("settings: 鼠标选中行 {}", row);
                return;
            }
        }
        // 键位行：点击选中
        let key_start_y = start_y + 5.0 * row_h + 24.0;
        for i in 0..7usize {
            let y = key_start_y + i as f32 * 18.0;
            if my_d >= y && my_d <= y + 18.0 {
                self.game.hud.settings_selection = (5 + i) as u8;
                log::info!("settings: 鼠标选中键位行 {}", 5 + i);
                return;
            }
        }
    }


    /// 第一人称枪模程序化高模：按当前武器键名从 guns 库取 35 把枪的网格，
    /// 变换到视空间固定位置（view⁻¹ × 锚点 × 倾斜 × 缩放 × 俯角 × 翻转 180°：
    /// guns 库局部坐标枪口朝 +Z，翻转后朝屏幕外 -Z）。
    /// 开火后坐（相位脉冲）+ 行走晃动 + 腰射右倾/开镜扶正。
    /// 导入枪模：assets/guns/ak12.glb（用户提供的外部模型）→ 烘焙顶点（同程序化光照）
    fn load_gun_glb() -> Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)> {
        let path = "assets/guns/ak12.glb";
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                log::info!("assets: 未发现 {path}（{e}），使用程序化枪模");
                return None;
            }
        };
        match crate::engine::assets::parse_glb(&bytes) {
            Ok(mesh) => {
                if mesh.verts.is_empty() {
                    log::warn!("assets: {path} 为空网格，回退程序化枪模");
                    return None;
                }
                // 归一化：Sketchfab 原始刻度（本例长轴 Y 约 85 单位）→ 0.94m 真实枪长；
                // 包围盒数据中心到原点；长轴（最大跨度）对齐 +Z（游戏枪模前向）；Y-up 校正
                let mut mn = [f32::MAX; 3];
                let mut mx = [f32::MIN; 3];
                for v in &mesh.verts {
                    for i in 0..3 {
                        mn[i] = mn[i].min(v[i]);
                        mx[i] = mx[i].max(v[i]);
                    }
                }
                let ext = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
                let long = ext[0].max(ext[1]).max(ext[2]);
                // 枪模 m 矩阵含 0.5 缩放 → 模型长 1.35m 折算视觉 ~0.68m（AK-12 实枪比例）
                let scale = 1.35 / long.max(1e-4);
                // 长轴对齐：Sketchfab Z-up 导出（长轴=Y 85、高=Z 21、宽=X 7，枪竖立）
                // 绕 X -90°：长轴→-Z、枪顶→+Y；再绕 Y 180° 预旋转（配合 fp_gun_matrix 的
                // rotY(180°) 双重取负 → 最终枪口朝 -Z（屏幕深处），枪顶朝上
                let align = if ext[1] >= ext[0] && ext[1] >= ext[2] {
                    // 长轴=Y（Sketchfab Z-up 竖立枪）：-90°X 立正 + 180°Z 滚转（枪顶朝上、弹匣朝下）
                    glam::Mat4::from_rotation_z(std::f32::consts::PI)
                        * glam::Mat4::from_rotation_y(std::f32::consts::PI)
                        * glam::Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
                } else if ext[0] >= ext[1] && ext[0] >= ext[2] {
                    glam::Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2)
                } else {
                    glam::Mat4::IDENTITY
                };
                let center = [
                    (mn[0] + mx[0]) * 0.5,
                    (mn[1] + mx[1]) * 0.5,
                    (mn[2] + mx[2]) * 0.5,
                ];
                // 注意：不在此处做任何相机空间变换——FP 帧内与程序化枪共用 fp_gun_matrix
                // （view_inv × anchor × scale；世界空间 + 每帧跟随相机）
                let light = glam::Vec3::new(-0.45, 0.8, -0.3).normalize();
                let verts: Vec<crate::engine::meshgen::GVertex> = mesh
                    .verts
                    .iter()
                    .map(|v| {
                        let mut p = glam::Vec3::new(v[0] - center[0], v[1] - center[1], v[2] - center[2]) * scale;
                        let mut n = glam::Vec3::from_slice(&v[3..6]);
                        p = align.transform_point3(p);
                        n = align.transform_vector3(n).normalize_or_zero();
                        let ndl = n.dot(light).max(0.0);
                        let shade = 0.30 + 0.92 * ndl;
                        // 逐顶点材质色（GLB 多材质保留）；基色 ~0.05 提亮 ×4.2（模型原本深金属灰）
                        let raw = [v[8], v[9], v[10]];
                        let c = [
                            (raw[0] * 4.2 * shade).min(1.0),
                            (raw[1] * 4.2 * shade).min(1.0),
                            (raw[2] * 4.2 * shade).min(1.0),
                        ];
                        crate::engine::meshgen::GVertex {
                            pos: [p.x, p.y, p.z],
                            normal: [n.x, n.y, n.z],
                            uv: [v[6], v[7]],
                            color: c,
                        }
                    })
                    .collect();
                log::info!(
                    "assets: 导入枪模 {path}（{} 顶点 / {} 索引）",
                    verts.len(),
                    mesh.indices.len()
                );
                Some((verts, mesh.indices))
            }
            Err(e) => {
                log::warn!("assets: {path} 解析失败: {e}；回退程序化枪模");
                None
            }
        }
    }

    fn first_person_gun_mesh(&mut self) -> (Vec<crate::engine::meshgen::GVertex>, Vec<u32>) {
        // 导入枪模优先（检视与第一人称共用：检视时居中，第一人称保持导入姿态）
        if let Some((verts, indices)) = self.gun_glb.clone() {
            if self.inspect_weapon.is_some() {
                // 居中到 (0, 1.0, 0)
                let mut mn = [f32::MAX; 3];
                let mut mx = [f32::MIN; 3];
                for v in &verts {
                    for i in 0..3 {
                        mn[i] = mn[i].min(v.pos[i]);
                        mx[i] = mx[i].max(v.pos[i]);
                    }
                }
                let c = [
                    (mn[0] + mx[0]) * 0.5,
                    (mn[1] + mx[1]) * 0.5,
                    (mn[2] + mx[2]) * 0.5,
                ];
                let moved: Vec<crate::engine::meshgen::GVertex> = verts
                    .iter()
                    .map(|v| crate::engine::meshgen::GVertex {
                        pos: [v.pos[0] - c[0], v.pos[1] - c[1] + 1.0, v.pos[2] - c[2]],
                        ..*v
                    })
                    .collect();
                return (moved, indices);
            }
            // 第一人称：与程序化枪共用 fp_gun_matrix（世界空间 + 每帧跟随相机）
            let m = self.fp_gun_matrix();
            let moved: Vec<crate::engine::meshgen::GVertex> = verts
                .iter()
                .map(|v| crate::engine::meshgen::GVertex {
                    pos: {
                        let p = m.transform_point3(glam::Vec3::from(v.pos));
                        [p.x, p.y, p.z]
                    },
                    normal: {
                        let n = m.transform_vector3(glam::Vec3::from(v.normal));
                        [n.x, n.y, n.z]
                    },
                    ..*v
                })
                .collect();
            return (moved, indices);
        }
        // 检视模式：枪模放世界原点上方（居中），Orbit 相机绕其旋转查看
        if let Some(n) = self.inspect_weapon {
            let key = crate::engine::weapon_data::spec_by_number(n)
                .map(|s| s.key)
                .unwrap_or("ak12m");
            if let Some(gm) = crate::engine::guns::gun_mesh_by_key(key) {
                // 居中：bbox 中心移到 (0, 1.0, 0)（用包围盒中点，顶点均值会偏向部件密集侧）
                let mut mn = [f32::MAX; 3];
                let mut mx = [f32::MIN; 3];
                for v in &gm.verts {
                    for i in 0..3 {
                        mn[i] = mn[i].min(v.pos[i]);
                        mx[i] = mx[i].max(v.pos[i]);
                    }
                }
                let c = [
                    (mn[0] + mx[0]) * 0.5,
                    (mn[1] + mx[1]) * 0.5,
                    (mn[2] + mx[2]) * 0.5,
                ];
                let verts: Vec<crate::engine::meshgen::GVertex> = gm
                    .verts
                    .iter()
                    .map(|v| crate::engine::meshgen::GVertex {
                        pos: [v.pos[0] - c[0], v.pos[1] - c[1] + 1.0, v.pos[2] - c[2]],
                        normal: v.normal,
                        uv: v.uv,
                        color: v.color,
                    })
                    .collect();
                return (verts, gm.indices.clone());
            }
        }
        // 当前武器枪模：按键名取模（构建含光照烘焙，缓存避免每帧重建）。
        // 优雅回退：无网格 / 构建 panic → 记录日志并回退默认 HK416。
        let key = self.game.active_weapon_key();
        let gun = match &self.gun_mesh_cache {
            Some((k, gm)) if k == key => gm.clone(),
            _ => {
                let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::engine::guns::gun_mesh_by_key(key)
                }))
                .unwrap_or(None);
                let gm = match built {
                    Some(gm) => gm,
                    None => {
                        log::warn!(
                            "weapons: 枪模回退——键 '{}' 无可用网格，使用默认 HK416",
                            key
                        );
                        crate::engine::guns::gun_mesh_by_key("hk416").unwrap_or_else(|| {
                            log::error!("weapons: 默认枪模也缺失，使用空网格（枪模不可见）");
                            crate::engine::guns::GunMesh {
                                verts: Vec::new(),
                                indices: Vec::new(),
                                display_name: "EMPTY",
                                length: 0.0,
                            }
                        })
                    }
                };
                self.gun_mesh_cache = Some((key.to_string(), gm.clone()));
                gm
            }
        };
        let cam = &self.camera;
        // ADS 姿态（2026-08-18 第三轮规范：模型已水平校正，rotation 全零）：
        // 腰射：枪在右下 (0.25,-0.20,-0.60)，FOV 70°，十字准星可见；
        // 开镜：枪居中 (0.0,-0.08,-0.42)，FOV 55°，准星隐藏（机瞄三点一线）。
        // 缩放大幅调低（0.98→0.5）：枪长 1.2m 在 0.6m 距离下占视野 ~70%，
        // 解决"怼脸/穿模"（旧值枪张角 100° > FOV 70°，必然怼脸）。
        let m = self.fp_gun_matrix();
        gun.transformed(m)
    }
    /// 第一人称枪的世界空间矩阵（程序化/导入枪模共用）：view_inv × anchor × scale。
    /// 开火后坐脉冲 + 行走晃动 + ADS 插值 + FOV 缩放（2026-08-27 抽离共享）
    fn fp_gun_matrix(&self) -> glam::Mat4 {
        let cam = &self.camera;
        let hip_pos = glam::Vec3::new(0.25, -0.20, -0.60);
        let ads_pos = glam::Vec3::new(0.0, -0.08, -0.42);
        let mut anchor = hip_pos.lerp(ads_pos, self.ads_blend);
        let since_shot = self.anim_clock - self.last_shot_at;
        if since_shot >= 0.0 && since_shot < 0.30 {
            let t = since_shot / 0.30;
            let pulse = (1.0 - t) * (1.0 - t);
            anchor.y -= 0.07 * pulse;
            anchor.z += 0.05 * pulse;
        }
        let bob = (self.anim_clock * 10.0).sin() * 0.012 * (1.0 - self.ads_blend * 0.9);
        anchor.x += bob;
        let base_scale = 0.50 - 0.03 * self.ads_blend;
        let gun_scale =
            ((cam.fov * 0.5).tan() / 35.0_f32.to_radians().tan()).clamp(0.5, 1.0) * base_scale;
        let view_inv = cam.view_matrix().inverse();
        view_inv
            * glam::Mat4::from_translation(anchor)
            * glam::Mat4::from_rotation_z(0.0)
            * glam::Mat4::from_scale(glam::Vec3::splat(gun_scale))
            * glam::Mat4::from_rotation_x(-0.045)
            * glam::Mat4::from_rotation_y(std::f32::consts::PI)
    }

    /// ESC 菜单鼠标点击命中检测：命中选项矩形则执行对应动作（0=退出 1=设置）。
    /// 矩形布局必须与 ui.rs `esc_menu_elements` 一致（面板 380x240 居中，
    /// 选项 y = py+90 / py+146，宽 pw-120=260 居中，高 34）。返回是否命中任何选项。
    fn menu_click_hit(&mut self, mx: f32, my: f32) -> bool {
        // 面板布局按设计基准 1280x800 计算后乘 ui_scale（与 ui.rs 渲染一致）
        let s = self.game.hud.ui_scale();
        let dw = self.game.hud.screen_w / s;
        let dh = self.game.hud.screen_h / s;
        let pw = 380.0;
        let ph = 240.0;
        let px = (dw - pw) * 0.5;
        let py = (dh - ph) * 0.5;
        let opt_w = pw - 120.0;
        let opt_x = px + 60.0;
        for (i, oy) in [py + 90.0, py + 146.0].iter().enumerate() {
            if mx >= (opt_x * s) && mx <= ((opt_x + opt_w) * s) && my >= ((*oy - 6.0) * s) && my <= ((*oy + 28.0) * s) {
                if i == 0 {
                    log::info!("ESC 菜单：鼠标点击退出游戏");
                    self.running = false;
                    if let Some(proxy) = &self.event_proxy {
                        let _ = proxy.send_event(());
                    }
                } else {
                    log::info!("ESC 菜单：鼠标点击设置");
                    self.game.hud.esc_menu_open = false;
                    self.game.toggle_settings();
                }
                return true;
            }
        }
        false
    }

    /// 按游戏状态同步光标捕获：Playing = 捕获 + 隐藏；否则释放。
    fn sync_cursor(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let want = self.focused
            && self.game.state() == GameState::Playing
            && !self.game.settings_open()
            && !self.game.hud.esc_menu_open;
        // ESC 菜单/设置面板打开或失焦时释放鼠标（2026-08-15：菜单需鼠标点选）
        if want && !self.cursor_captured {
            // 优先 Locked：系统级指针锁定 + 相对 MouseMotion，光标不会飞出窗口。
            // Xwayland 等不支持 Locked 的环境回退 Confined；即使 grab 全不可用，
            // 只要 DeviceEvent::MouseMotion 到达（XInput2 raw motion），视角仍由相对增量驱动。
            let locked = window.set_cursor_grab(CursorGrabMode::Locked).is_ok();
            let grabbed = if locked {
                true
            } else {
                window.set_cursor_grab(CursorGrabMode::Confined).is_ok()
            };
            window.set_cursor_visible(false);
            self.cursor_captured = true;
            self.cursor_locked = locked;
            self.abs_baseline_valid = false;
            if !locked {
                // WSLg/Xwayland 回退：绝对位置路径。不在捕获瞬间回中——
                // 指针真实位置未知，等首个 CursorMoved 作基准（abs_baseline_valid）。
                self.recenter_pending_until = None;
            } else {
                // Locked grab 可用：raw 相对增量驱动。捕获瞬间回中隐藏光标，
                // 150ms 窗口吞掉这次 warp 的 raw 回声（仅此一次 warp）。
                let size = window.inner_size();
                let center = winit::dpi::PhysicalPosition::new(
                    size.width as f64 / 2.0,
                    size.height as f64 / 2.0,
                );
                let _ = window.set_cursor_position(center);
                self.last_cursor = (center.x, center.y);
                self.recenter_pending_until = Some(Instant::now() + Duration::from_millis(150));
            }
            log::info!(
                "input: cursor captured (mouse look on, grab={}, look={})",
                if locked {
                    "locked"
                } else if grabbed {
                    "confined"
                } else {
                    "none"
                },
                if locked { "relative" } else { "absolute" }
            );
        } else if !want && self.cursor_captured {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
            self.cursor_captured = false;
            self.cursor_locked = false;
            self.abs_baseline_valid = false;
            self.recenter_pending_until = None;
            self.camera.set_rotation_active(false);
            log::info!("input: cursor released");
        }
    }

    /// 把 WASD 按键状态转发给游戏（FPS 玩家移动）
    fn sync_game_movement(&mut self) {
        let k = &self.key_state;
        self.game.set_movement(k.forward, k.backward, k.left, k.right);
    }

    /// 把当前分辨率应用到窗口（尺寸相同则跳过；`Resized` 事件会触发渲染器重建交换链）
    fn apply_resolution(&self) {
        let (w, h) = self.game.hud.resolution();
        let Some(window) = &self.window else {
            log::info!("settings: 窗口未就绪，分辨率 {}x{} 待应用", w, h);
            return;
        };
        let cur = window.inner_size();
        if cur.width == w && cur.height == h {
            log::info!("settings: 分辨率保持 {}x{}", w, h);
            return;
        }
        let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
        log::info!("settings: 应用分辨率 {}x{}", w, h);
    }

    /// 把当前画质应用到渲染器（设置面板切换后即时生效）
    fn apply_quality(&mut self) {
        let preset = match self.game.hud.quality_index {
            0 => QualityPreset::Low,
            1 => QualityPreset::Medium,
            _ => QualityPreset::High,
        };
        log::info!("settings: 应用画质 {}", preset.label());
        if let Some(renderer) = &mut self.renderer {
            renderer.set_quality(preset);
        }
    }

    /// F12 截图：调渲染器把当前帧保存到 <平台截图目录>/steel_front_<秒时间戳>.png
    /// （Windows = 当前目录 screenshots/，非 Windows 沿用 /tmp 保持 WSL2 行为）
    fn capture_screenshot(&mut self) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        #[cfg(windows)]
        let path = {
            let dir = std::path::PathBuf::from("screenshots");
            let _ = std::fs::create_dir_all(&dir);
            dir.join(format!("steel_front_{}.png", ts))
        };
        #[cfg(not(windows))]
        let path = std::path::PathBuf::from(format!("/tmp/steel_front_{}.png", ts));
        match self.renderer.as_mut() {
            Some(renderer) => match renderer.capture_screenshot(&path) {
                Ok(()) => log::info!("截图已保存: {}", path.display()),
                Err(e) => log::error!("截图失败 {}: {}", path.display(), e),
            },
            None => log::warn!("截图跳过：渲染器未就绪"),
        }
    }

    /// 渲染一帧
    fn render(&mut self) {
        // 第一人称枪模程序化网格（相机姿态）：在借用 renderer 前生成，避免借用冲突
        let gun_mesh = self.first_person_gun_mesh();
        // 诊断（每 5 秒一次）：窗口 inner_size vs swapchain extent vs HUD 尺寸
        if self.anim_clock > 5.0 && (self.anim_clock - 5.0) % 5.0 < 0.05 {
            if let Some(win) = &self.window {
                let is = win.inner_size();
                log::info!(
                    "size diag: window_inner={}x{} hud={}x{}",
                    is.width,
                    is.height,
                    self.game.hud.screen_w,
                    self.game.hud.screen_h
                );
            }
        }
        if let Some(renderer) = &mut self.renderer {
            // 投影宽高比取实际窗口尺寸（16:10 等非 16:9 分辨率下不拉伸）
            let aspect = self
                .window
                .as_ref()
                .map(|w| {
                    let s = w.inner_size();
                    s.width.max(1) as f32 / s.height.max(1) as f32
                })
                .unwrap_or(16.0 / 9.0);
            if self.inspect_weapon.is_some() && !self.cam_logged {
                self.cam_logged = true;
                log::info!(
                    "inspect cam: pos=({:.3},{:.3},{:.3}) target=({:.3},{:.3},{:.3}) yaw={:.3} pitch={:.3} dist={:.3} fov={:.3} aspect={:.3}",
                    self.camera.position().x, self.camera.position().y, self.camera.position().z,
                    self.camera.target.x, self.camera.target.y, self.camera.target.z,
                    self.camera.yaw, self.camera.pitch, self.camera.distance,
                    self.camera.fov.to_degrees(), aspect
                );
            }
            let view = self.camera.view_matrix();
            // 投影矩阵不翻转 Y：主 shader（triangle.vert.spv）已在 gl_Position.y 上完成
            // Vulkan 翻转，若这里再翻一次会双重翻转导致画面上下颠倒（与 HUD shader 一致）。
            let proj = self.camera.projection_matrix(aspect);

            // 枪械检视模式：虚空环境——只画枪模（renderer 跳过地形/NPC/marker/阴影）
            renderer.void_mode = self.inspect_weapon.is_some();

            // HUD：用上一帧渲染统计生成覆盖层 quad 并上传（首帧统计为 0）
            let (near, far, lod) = renderer.last_stats();
            // 检视模式：无游戏 HUD（纯枪模检视画面）
            let mut quads = if self.inspect_weapon.is_some() {
                Vec::new()
            } else {
                self.game.hud_quads(near, far, lod)
            };
            // 命令输入窗口（Minecraft 风格左下角）：深色半透明底 + 提示符 + 闪烁光标
            if self.command_open && self.game.state() == GameState::Playing {
                let s = self.game.hud.ui_scale();
                let prompt = format!("> {}{}", self.command_buf, {
                    if (self.anim_clock * 2.0).sin() > 0.0 { '_' } else { ' ' }
                });
                let box_x = 10.0 * s;
                let box_y = (800.0 - 46.0) * s;
                let box_h = 36.0 * s;
                let text_scale = 2.0 * s;
                let text_w = crate::ui::text_width(&prompt, text_scale);
                let box_w = (text_w + 26.0 * s).max(180.0 * s);
                quads.push(crate::ui::Quad::new(
                    crate::ui::Rect::new(box_x, box_y, box_w, box_h),
                    crate::ui::Color::new(0.06, 0.06, 0.12, 0.72),
                ));
                crate::ui::render_text(
                    &prompt,
                    box_x + 10.0 * s,
                    box_y + 8.0 * s,
                    crate::ui::Color::WHITE,
                    text_scale,
                    &mut quads,
                );
                // 提示行：武器编号范围说明
                crate::ui::render_text(
                    "武器编号 1-35（回车切换，Esc 关闭）",
                    box_x + 2.0 * s,
                    box_y - 22.0 * s,
                    crate::ui::Color::YELLOW,
                    1.3 * s,
                    &mut quads,
                );
            }
            // 伤害飘字：准星下方逐条显示（红色，随剩余时间上浮淡出）
            let s = self.game.hud.ui_scale();
            let mut popup_y = 120.0 * s;
            for (dmg, remain) in &self.hit_damage_popups {
                let alpha = (remain / 0.6).clamp(0.0, 1.0);
                crate::ui::render_text(
                    &format!("-{:.0}", dmg),
                    (self.game.hud.screen_w / s) * 0.5 * s + 12.0 * s,
                    popup_y,
                    crate::ui::Color::new(1.0, 0.35 * alpha, 0.25 * alpha, alpha),
                    1.6 * s,
                    &mut quads,
                );
                popup_y += 20.0 * s;
            }
            renderer.set_hud_quads(&quads);
            renderer.set_lights(&self.game.light_uniform());
            // 世界障碍 marker：关卡地图障碍盒 → 按种类材质着色的盒实例（复用主 pipeline，
            // 见 renderer.rs MARKER_SLOT_BASE；模型矩阵/材质色统一由 WorldMarker::for_obstacle 构建，
            // 与物理刚体 AABB（game.rs apply_level，同 half_w/half_d）严格同尺寸）
            let markers: Vec<engine::renderer::WorldMarker> = self
                .game
                .map_obstacles()
                .iter()
                .map(engine::renderer::WorldMarker::for_obstacle)
                .collect();
            // 占领据点世界标记（关卡系统 RV3D_MAP/RV3D_MAPS 启用时非空）：
            // 每据点 = 细高立柱（归属色）+ 扁平底盘（半径 5.0，半透明归属色）。
            // 复用 WorldMarker 通道（主 pipeline 实例化），零渲染管线改动。
            let capture_markers: Vec<engine::renderer::WorldMarker> = self
                .game
                .capture_points()
                .into_iter()
                .flat_map(|(id, x, z, owner, _progress)| {
                    let tint = match owner {
                        Some(crate::engine::ai::Team::Blue) => [0.08, 0.35, 0.98, 1.0],
                        Some(crate::engine::ai::Team::Red) => [0.95, 0.12, 0.08, 1.0],
                        None => [0.45, 0.45, 0.45, 1.0],
                    };
                    let base_tint = [tint[0] * 0.6, tint[1] * 0.6, tint[2] * 0.6, 0.6];
                    let _ = id; // 标记 id 暂不绘制文字（HUD 已有 id 标签）
                    [
                        // 立柱（旗杆）
                        engine::renderer::WorldMarker {
                            model: glam::Mat4::from_translation(glam::Vec3::new(x, 2.0, z))
                                * glam::Mat4::from_scale(glam::Vec3::new(0.4, 4.0, 0.4)),
                            tint,
                        },
                        // 地面底盘（占领半径范围，半径 5.0 → scale 10.0）
                        engine::renderer::WorldMarker {
                            model: glam::Mat4::from_translation(glam::Vec3::new(x, 0.08, z))
                                * glam::Mat4::from_scale(glam::Vec3::new(10.0, 0.15, 10.0)),
                            tint: base_tint,
                        },
                    ]
                })
                .collect();
            let mut markers = markers;
            markers.extend(capture_markers);
            // 爆炸闪光：冲击波球壳随年龄膨胀、颜色转淡；走自发光路径（emissive 槽位，
            // shader 直出纯色跳过光照/贴图混合），夜间等暗光环境下依然清晰可见
            // 爆炸多层视觉（4 层同源演算，立体感：火球核 + 贴地冲击波环 + 火柱 + 烟柱）
            let mut emissive_markers: Vec<engine::renderer::WorldMarker> = self
                .game
                .explosions()
                .iter()
                .flat_map(|ex| {
                let t = (ex.age / ex.lifetime).clamp(0.0, 1.0);
                let cx = ex.center[0];
                let cz = ex.center[2];
                let r = ex.radius;
                // ① 火球核：亮黄白，快速膨胀 + 快速淡出（0-0.35 寿命为主）；半透明球形
                let fireball_t = (t * 2.8).min(1.0);
                let fb_s = r * (0.2 + 1.2 * fireball_t);
                let fb_alpha = (0.9 * (1.0 - fireball_t) + 0.15).clamp(0.0, 1.0);
                let mut out = vec![engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(cx, 1.2, cz))
                        * glam::Mat4::from_scale(glam::Vec3::splat(fb_s)),
                    tint: [
                        1.0,
                        0.85 * (1.0 - fireball_t) + 0.2,
                        0.35 * (1.0 - fireball_t),
                        fb_alpha,
                    ],
                }];
                // ② 贴地冲击波环：扁球体（球体几何压扁）沿地面水平扩散 + 高度衰减，半透明
                let ring_s = r * (0.4 + 1.6 * t);
                let ring_h = (1.1 * (1.0 - t)).max(0.15);
                let ring_alpha = (0.75 * (1.0 - t) + 0.1).clamp(0.0, 1.0);
                out.push(engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(cx, ring_h * 0.5, cz))
                        * glam::Mat4::from_scale(glam::Vec3::new(ring_s, ring_h, ring_s)),
                    tint: [1.0, 0.55 * (1.0 - t) + 0.15, 0.06, ring_alpha],
                });
                // ③ 火柱：垂直拉长火舌从地面向上（0.5-2 寿命段），半透明
                let col_h = 2.2 + 2.6 * t;
                let col_alpha = (0.8 * (1.0 - t) + 0.1).clamp(0.0, 1.0);
                out.push(engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(cx, 1.1 + col_h * 0.5, cz))
                        * glam::Mat4::from_scale(glam::Vec3::new(r * 0.5, col_h, r * 0.5)),
                    tint: [1.0, 0.45 * (1.0 - t), 0.05, col_alpha],
                });
                // ④ 烟柱：暗色膨胀上浮（后段，营造爆炸余烟），半透明
                let smoke_s = r * (0.5 + 1.4 * t);
                let smoke_h = 2.0 + 3.0 * t;
                let smoke_alpha = (0.6 * (1.0 - t) + 0.08).clamp(0.0, 1.0);
                out.push(engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(cx, 0.6 + smoke_h * 0.5, cz))
                        * glam::Mat4::from_scale(glam::Vec3::new(smoke_s, smoke_h, smoke_s)),
                    tint: [0.16 * (1.0 - t) + 0.05, 0.13 * (1.0 - t) + 0.04, 0.1 * (1.0 - t) + 0.03, smoke_alpha],
                });
                out
                })
                .collect();
            // 粒子（枪口焰/弹壳）转 emissive marker：枪口焰随 age 缩小淡出，弹壳保持小方块
            for p in &self.particles {
                let t = (p.age / p.life).clamp(0.0, 1.0);
                let size = if p.kind == 0 {
                    p.size * (1.0 - t * 0.7) // 焰：快速收缩
                } else {
                    p.size
                };
                let fade = 1.0 - t;
                emissive_markers.push(engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::from(p.pos))
                        * glam::Mat4::from_scale(glam::Vec3::splat(size)),
                    tint: [p.tint[0] * fade, p.tint[1] * fade, p.tint[2] * fade, 1.0],
                });
            }
            // 手雷可见实体：深橄榄色小方块（飞行/落地均可见，复用 emissive 通道）
            for gp in self.game.grenade_positions() {
                emissive_markers.push(engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::from(gp))
                        * glam::Mat4::from_scale(glam::Vec3::splat(0.16)),
                    tint: [0.35, 0.4, 0.12, 1.0],
                });
            }
            renderer.set_world_markers(&markers);
            renderer.set_emissive_markers(&emissive_markers);
            // NPC 士兵可视化：每个 NPC 由 renderer 展开为 7 段积木人（头/躯干/四肢/枪），
            // 按朝向旋转，阵营配色（红=敌军、蓝=友军/玩家阵营）；
            // 动画字段：移动中摆臂摆腿（步态）、攻击态枪身后坐脉冲
            let now_ids: std::collections::HashSet<usize> =
                self.game.npcs.iter().map(|n| n.id).collect();
            // 尸体跟踪：本帧消失的 NPC id（被击杀移除）→ 从上一帧快照找回位置/朝向/阵营
            for id in self.last_npc_snapshot.keys() {
                if !now_ids.contains(id) {
                    if let Some((pos, yaw, tint)) = self.last_npc_snapshot.get(id) {
                        self.corpses.push((*pos, *yaw, *tint, 0.0));
                    }
                }
            }
            // 更新快照（供下一帧 diff）
            self.last_npc_snapshot = self
                .game
                .npcs
                .iter()
                .map(|n| {
                    let tint = match n.team {
                        Team::Red => [0.95, 0.12, 0.08, 1.0],
                        Team::Blue => [0.08, 0.35, 0.98, 1.0],
                    };
                    (n.id, (n.position, n.facing, tint))
                })
                .collect();
            // 客户端联机模式：显示服务器世界（快照实体：位置/朝向/血量来自服务器权威），
            // 阵营色借用本地同 id NPC 的归属（同一确定性地图/波次，id 对齐）
            let net_mode = self.game.net_client.is_some();
            let npc_visuals: Vec<engine::renderer::NpcVisual> = if net_mode {
                let client = self.game.net_client.as_ref().unwrap();
                client
                    .entities()
                    .iter()
                    .filter(|(id, e)| (**id < 100_000 && e.hp > 0.0) || **id == 0 || **id >= 100_000)
                    .map(|(_, e)| {
                        // 阵营直接取自快照（服务器权威；NpcSnapshot.team 0=Red 1=Blue）
                        let tint = if e.hp > 0.0 {
                            if e.team == 1 { [0.08, 0.35, 0.98, 1.0] } else { [0.95, 0.12, 0.08, 1.0] }
                        } else {
                            [0.32, 0.32, 0.32, 1.0]
                        };
                        engine::renderer::NpcVisual {
                            pos: [e.state.curr.pos[0], e.state.curr.pos[1], e.state.curr.pos[2]],
                            yaw: e.state.curr.rot,
                            tint,
                            phase: self.anim_clock,
                            moving: true,
                            firing: e.firing,
                        }
                    })
                    .collect()
            } else {
                self
                .game
                .npcs
                .iter()
                .enumerate()
                // 隔墙透视修复：被障碍物完全遮挡的 NPC 不渲染
                .filter(|(i, _)| !self.game.npc_occluded(*i))
                .map(|(_, n)| {
                    let base = self
                        .last_npc_snapshot
                        .get(&n.id)
                        .map(|(_, _, t)| *t)
                        .unwrap_or(match n.team {
                            Team::Red => [0.95, 0.12, 0.08, 1.0],
                            Team::Blue => [0.08, 0.35, 0.98, 1.0],
                        });
                    // 受击反馈：命中瞬间闪白（按剩余强度混合白色）
                    let flash = self.game.npc_flash(n.id);
                    let tint = if flash > 0.0 {
                        let k = flash * 0.85;
                        [
                            base[0] + (1.0 - base[0]) * k,
                            base[1] + (1.0 - base[1]) * k,
                            base[2] + (1.0 - base[2]) * k,
                            1.0,
                        ]
                    } else {
                        base
                    };
                    engine::renderer::NpcVisual {
                        pos: n.position,
                        yaw: n.facing,
                        tint,
                        phase: self.anim_clock,
                        moving: n.speed > 0.5
                            && matches!(
                                n.state_machine.state(),
                                crate::engine::ai::NpcState::Patrol
                                    | crate::engine::ai::NpcState::Chase
                            ),
                        firing: n.state_machine.state() == crate::engine::ai::NpcState::Attack,
                    }
                })
                .collect()
            };
            renderer.set_npc_visuals(&npc_visuals);
            // NPC 枪口焰/弹壳：攻击态 NPC 限流生成（每帧最多 4 个，按 id 相位轮转避免全爆发）
            let mut firing_npcs: Vec<[f32; 3]> = self
                .game
                .npcs
                .iter()
                .enumerate()
                .filter(|(i, n)| {
                    !self.game.npc_occluded(*i)
                        && n.state_machine.state() == crate::engine::ai::NpcState::Attack
                })
                .map(|(_, n)| n)
                .filter(|n| (n.id as f32 + self.anim_clock * 6.0) % 4.0 < 1.0)
                .take(4)
                .map(|n| {
                    // 枪口世界位置：facing 为绕 Y 旋转角（0 = +Z），枪口在身前 0.85m、高 1.3m
                    let (s, c) = n.facing.sin_cos();
                    [
                        n.position[0] + s * 0.85,
                        1.3,
                        n.position[2] + c * 0.85,
                    ]
                })
                .collect();
            // 网络远端实体开火 → 同链路枪口焰（你看到对面玩家开枪的火光）
            if let Some(client) = self.game.net_client.as_ref() {
                for (_, e) in client.entities().iter() {
                    if e.firing && e.hp > 0.0 {
                        let (s, c) = e.state.curr.rot.sin_cos();
                        firing_npcs.push([
                            e.state.curr.pos[0] + s * 0.85,
                            e.state.curr.pos[1] + 0.15,
                            e.state.curr.pos[2] + c * 0.85,
                        ]);
                    }
                }
            }
            for muzzle in firing_npcs {
                self.particles.push(Particle {
                    pos: muzzle,
                    vel: [0.0, 0.0, 0.0],
                    age: 0.0,
                    life: 0.07,
                    size: 0.14,
                    tint: [1.0, 0.7, 0.2, 1.0],
                    kind: 0,
                });
            }
            // 尸体渲染（躺倒姿态，7 段/具；与活体共用 NPC 槽位区）
            let dead_visuals: Vec<engine::renderer::NpcVisual> = self
                .corpses
                .iter()
                .map(|(pos, yaw, tint, _age)| engine::renderer::NpcVisual {
                    pos: *pos,
                    yaw: *yaw,
                    tint: *tint,
                    phase: 0.0,
                    moving: false,
                    firing: false,
                })
                .collect();
            renderer.set_dead_bodies(&dead_visuals);
            // 第一人称枪模高模网格（已在 render() 入口生成，此处上传）
            // 枪模仅在第一人称游玩或检视模式渲染：结算/其它相机态下隐藏
            // （否则枪模会按锚点漂浮在场景中——2-4 反馈“变成 M1 加兰德”观感）
            let show_gun = self.inspect_weapon.is_some()
                || (self.game.state() == GameState::Playing
                    && self.camera.mode == CameraMode::FirstPerson);
            if show_gun {
                renderer.set_first_person_gun_mesh(&gun_mesh.0, &gun_mesh.1);
            } else {
                renderer.set_first_person_gun_mesh(&[], &[]);
            }

            // 尺寸保险（2026-08-15）：窗口实际尺寸与交换链不一致时重建——
            // 覆盖 DPI 缩放/全屏切换等任何导致 swapchain 与窗口错位的场景，
            // 根治"画面只显示左上角"（1:1 呈现但尺寸不匹配）。
            let (sw, sh) = renderer.swapchain_size();
            if let Some(win) = &self.window {
                let is = win.inner_size();
                if (is.width != sw || is.height != sh) && is.width > 0 && is.height > 0 {
                    log::warn!(
                        "size mismatch: window={}x{} swapchain={}x{} → 重建交换链",
                        is.width, is.height, sw, sh
                    );
                    let _ = renderer.recreate_swapchain();
                    let _ = self.game.hud.set_screen_size(is.width as f32, is.height as f32);
                }
            }
            if let Err(e) = renderer.render(view, proj) {
                if e == "交换链过期" {
                    log::warn!("交换链过期，尝试重建...");
                    let _ = renderer.recreate_swapchain();
                } else {
                    log::error!("渲染错误: {}", e);
                }
            }
            // 性能日志采样（1s 一行）
            if let Some(pl) = self.perf_log.as_mut() {
                let snap = renderer.perf_snapshot();
                let (near, _, _) = renderer.last_stats();
                pl.frame(self.last_fps, near, &snap);
            }
        }
    }
}

impl ApplicationHandler for GameApp {
    /// 应用恢复/启动时创建窗口和渲染器
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        // 首次运行（配置无显式分辨率）：按显示器宽高比选默认
        // 16:10 → 1280x800，16:9 及其它 → 1280x720
        if !self.resolution_explicit {
            // Wayland 没有"主显示器"概念（winit primary_monitor() 恒 None），
            // 回退到面积最大的可用显示器；两者都拿不到时退回 1280x720
            let monitor = event_loop.primary_monitor().or_else(|| {
                event_loop
                    .available_monitors()
                    .max_by_key(|m| m.size().width * m.size().height)
            });
            let default_res = monitor
                .map(|m| {
                    let size = m.size();
                    let aspect = size.width as f32 / size.height.max(1) as f32;
                    log::info!("显示器: {}x{} aspect={:.3}", size.width, size.height, aspect);
                    if (1.5..=1.67).contains(&aspect) {
                        (1280, 800)
                    } else {
                        (1280, 720)
                    }
                })
                .unwrap_or((1280, 720));
            self.game.hud.resolution_index = RESOLUTIONS
                .iter()
                .position(|&r| r == default_res)
                .unwrap_or(0) as u8;
            log::info!("默认分辨率: {}x{}", default_res.0, default_res.1);
        }

        // ---- 创建窗口（尺寸取 HUD 当前分辨率：配置显式值或按显示器选定的默认值）----
        let (mut w, mut h) = self.game.hud.resolution();
        // 2026-08-15：窗口尺寸 clamp 到主显示器可用区（防止配置分辨率超屏 → 内容只显示左上角）。
        // 主显示器物理尺寸经 winit monitor.size()（物理像素）；DPI 缩放下逻辑 ≠ 物理，
        // 但 PhysicalSize 请求按物理像素处理，超屏窗口会被系统裁切。
        // 2026-08-15：窗口尺寸 clamp 到主显示器物理尺寸（防止超屏 → 内容只显示左上角）。
        // 请求分辨率（如 2560x1600）等于显示器物理大小时窗口为全屏无边框语义，
        // 但 Windows 任务栏会遮挡底部——此处仅防止"窗口 > 屏幕"的裁剪型错位。
        if let Some(monitor) = event_loop.primary_monitor() {
            let msize = monitor.size();
            if w > msize.width || h > msize.height {
                log::warn!(
                    "窗口尺寸 {}x{} 超过主显示器 {}x{}，自动缩放适配",
                    w, h, msize.width, msize.height
                );
                let scale = (msize.width as f32 / w.max(1) as f32)
                    .min(msize.height as f32 / h.max(1) as f32);
                w = (w as f32 * scale).max(320.0) as u32;
                h = (h as f32 * scale).max(200.0) as u32;
            }
        }
        // 2026-08-15：无边框窗口——请求分辨率等于显示器物理尺寸时窗口恰好铺满屏幕，
        // 无标题栏/边框挤压（否则窗口比屏幕略大 → DWM 裁剪 → 内容偏左上角）。
        // 2026-08-15：窗口尺寸用 LogicalSize（winit 按 scale_factor 自动转物理）——
        // 若直接给 PhysicalSize，DPI 缩放下 winit 可能按逻辑解释导致窗口/swapchain 尺寸错位
        // （表现为画面偏左上角/缩放不正确）。无边框 + 逻辑尺寸 = 显示器比例一致。
        // 2026-08-15：窗口尺寸用 LogicalSize（winit 按 scale_factor 自动转物理）——
        // 若直接给 PhysicalSize，DPI 缩放下 winit 可能按逻辑解释导致窗口/swapchain 尺寸错位
        // （表现为画面偏左上角/缩放不正确）。无边框 + 逻辑尺寸 = 显示器比例一致。
        // 窗口位置显式 (0,0)：默认位置可能偏移，2560x1600 窗口超出屏幕右下 → 画面偏左上。
        let winit_attr = Window::default_attributes()
            .with_title(window::WINDOW_TITLE)
            .with_inner_size(winit::dpi::LogicalSize::new(w as f64 / 1.5, h as f64 / 1.5))
            .with_position(winit::dpi::PhysicalPosition::new(0, 0))
            .with_decorations(false);

        let window = match event_loop.create_window(winit_attr) {
            Ok(w) => w,
            Err(e) => {
                log::error!("创建窗口失败: {:?}", e);
                event_loop.exit();
                return;
            }
        };

        log::info!("窗口创建成功: {}x{}", w, h);
        log::info!(
            "winit inner_size: {}x{} scale_factor={:.2}",
            window.inner_size().width,
            window.inner_size().height,
            window.scale_factor()
        );

        // ---- 初始化 Vulkan 渲染器 ----
        match Renderer::new(&window) {
            Ok(renderer) => {
                log::info!("Vulkan 渲染器初始化成功");
                self.renderer = Some(renderer);
            }
            Err(e) => {
                log::error!("渲染器初始化失败: {}", e);
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
        if let Some(win) = &self.window {
            let size = win.inner_size();
            self.game.hud.set_screen_size(size.width as f32, size.height as f32);
        }
        self.last_frame = Instant::now();

        // 应用持久化的分辨率与画质（窗口/渲染器就绪后即时生效）
        self.apply_resolution();
        self.apply_quality();

        // ---- 性能日志（每次启动一份 logs/perf_*.log）----
        let gpu = self
            .renderer
            .as_ref()
            .map(|r| r.gpu_name())
            .unwrap_or_else(|| "未知".to_string());
        let topo = crate::engine::cpu::topology();
        let vendor = match topo.vendor {
            crate::engine::cpu::CpuVendor::Amd => "AMD",
            crate::engine::cpu::CpuVendor::Intel => "Intel",
            crate::engine::cpu::CpuVendor::Other => "Other",
        };
        let cpu = format!("{} {}线程", vendor, topo.threads);
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
        let header = format!(
            "版本: {} | 启动: {} | GPU: {} | CPU: {} | 窗口: {}x{}",
            env!("CARGO_PKG_VERSION"),
            perf_log::now_human(),
            gpu,
            cpu,
            size.width,
            size.height
        );
        self.perf_log = perf_log::PerfLog::create(&header);
        if self.perf_log.is_some() {
            log::info!("性能日志已创建（logs/perf_*.log）");
        }
    }

    /// 设备级事件：系统相对鼠标增量（XInput2 raw motion，与光标位置无关）驱动视角。
    /// 捕获态唯一视角输入源：raw 增量是设备原始计数，与指针位置/grab 状态无关，
    /// 不依赖窗口内指针位置，也不产生"每帧回中 warp → 回声"反馈环。
    /// 用户事件（菜单点击退出用代理发送）：收到即退出事件循环
    fn user_event(&mut self, event_loop: &ActiveEventLoop, _event: ()) {
        log::info!("input: 收到退出事件，退出游戏");
        self.running = false;
        if let Some(pl) = self.perf_log.as_mut() {
            pl.finish();
            self.perf_log = None;
        }
        event_loop.exit();
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            // raw 相对增量仅在 Locked grab 可用时有效（真实设备级增量）；
            // WSLg/Xwayland（Locked 失败）真实鼠标不产生 raw 事件，走绝对位置路径。
            if self.cursor_captured && self.cursor_locked {
                // 捕获瞬间回中 warp 的 raw 回声在 recenter 窗口期（150ms）内到达：
                // 跳过，避免把"捕获前光标到窗口中心的差量"当成视角位移。
                // 真实鼠标移动不受限制：raw 增量直接驱动视角（不能用绝对像素阈值
                // 过滤，见 MAX_LOOK_DELTA_PX 注释）。
                if let Some(until) = self.recenter_pending_until {
                    if Instant::now() < until {
                        return;
                    }
                    self.recenter_pending_until = None;
                }
                let (dx, dy) = (delta.0 as f32, delta.1 as f32);
                // raw 单事件超物理上限：残留 warp 回声，跳过（防反馈环自转）
                if delta.0.abs() > MAX_RAW_LOOK_DELTA || delta.1.abs() > MAX_RAW_LOOK_DELTA {
                    return;
                }
                match self.camera.mode {
                    CameraMode::FirstPerson => self.camera.look(dx, dy),
                    CameraMode::Orbit => self.camera.orbit(dx, dy),
                    CameraMode::Flight => {
                        self.camera.set_rotation_active(true);
                        self.camera.add_rotation_input(dx, dy);
                    }
                }
            }
        }
    }

    /// 处理窗口事件
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            // 关闭窗口请求
            WindowEvent::CloseRequested => {
                log::info!("窗口关闭请求，退出程序");
                self.running = false;
                event_loop.exit();
            }

            // 键盘事件：处理 WASD 按键和 ESC 退出
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(key_code),
                        state,
                        ..
                    },
                ..
            } => {
                let pressed = state == ElementState::Pressed;

                // 退出确认中：任意非 ESC 按键取消待确认退出
                if pressed && key_code != KeyCode::Escape && self.game.hud.confirm_quit {
                    self.game.hud.confirm_quit = false;
                }

                // 开始菜单 / 关卡加载中：任意键（除 ESC）开始游戏
                if pressed
                    && (self.game.state() == GameState::StartMenu
                        || self.game.state() == GameState::LoadingMap)
                    && key_code != KeyCode::Escape
                {
                    self.game.on_any_key(&self.camera.position());
                }

                // 设置面板键位绑定监听：非 ESC 按键完成绑定，ESC 取消；随后不再走常规按键
                if self.game.settings_open() && self.game.rebinding_active() {
                    if pressed {
                        if key_code == KeyCode::Escape {
                            log::info!("settings: 取消键位绑定");
                            self.game.cancel_rebind();
                        } else if KeyBindings::is_reserved(key_code as u32) {
                            log::info!("settings: 保留键不可绑定 {:?}", key_code);
                            self.game.cancel_rebind();
                        } else {
                            log::info!("settings: 键位绑定完成 code={:?}", key_code);
                            self.game.complete_rebind(key_code as u32);
                        }
                    }
                    return;
                }

                // 命令输入窗口（Minecraft 风格）：打开时数字/退格/回车/ESC 专属处理，
                // 其余按键全部吞掉（移动/开火不响应）
                if self.command_open && self.game.state() == GameState::Playing {
                    if pressed {
                        match key_code {
                            KeyCode::Digit0 => self.command_buf.push('0'),
                            KeyCode::Digit1 => self.command_buf.push('1'),
                            KeyCode::Digit2 => self.command_buf.push('2'),
                            KeyCode::Digit3 => self.command_buf.push('3'),
                            KeyCode::Digit4 => self.command_buf.push('4'),
                            KeyCode::Digit5 => self.command_buf.push('5'),
                            KeyCode::Digit6 => self.command_buf.push('6'),
                            KeyCode::Digit7 => self.command_buf.push('7'),
                            KeyCode::Digit8 => self.command_buf.push('8'),
                            KeyCode::Digit9 => self.command_buf.push('9'),
                            KeyCode::Backspace => {
                                self.command_buf.pop();
                            }
                            KeyCode::Enter => {
                                let raw = self.command_buf.clone();
                                let n: usize = match raw.parse() {
                                    Ok(v) => v,
                                    Err(e) => {
                                        // 优雅回退：非数字输入 → 记录原因并忽略
                                        log::warn!(
                                            "command: 输入回退——'{}' 无法解析为数字（{}），忽略",
                                            raw,
                                            e
                                        );
                                        self.command_open = false;
                                        self.command_buf.clear();
                                        return;
                                    }
                                };
                                self.command_open = false;
                                self.command_buf.clear();
                                if n >= 1 {
                                    // 越界由 game.switch_weapon 内回退并记录日志
                                    log::info!("command: 切换到武器 #{}", n);
                                    self.game.switch_weapon(n - 1);
                                } else {
                                    log::warn!("command: 输入回退——编号 0 无效，忽略");
                                }
                            }
                            KeyCode::Escape => {
                                self.command_open = false;
                                self.command_buf.clear();
                            }
                            _ => {}
                        }
                        // 输入长度上限（35 最大两位，留余量）
                        if self.command_buf.len() > 4 {
                            self.command_buf.truncate(4);
                        }
                    }
                    return;
                }

                // ESC 是保留系统键（不参与重绑定）：设置面板打开时关闭面板；
                // 否则切换 ESC 毛玻璃菜单（退出游戏 / 设置两个选项）
                if pressed && key_code == KeyCode::Escape {
                    if self.game.settings_open() {
                        log::info!("ESC 关闭设置面板");
                        self.game.toggle_settings();
                    } else if self.game.hud.esc_menu_open {
                        log::info!("ESC 关闭菜单");
                        self.game.hud.esc_menu_open = false;
                    } else {
                        log::info!("ESC 打开菜单（退出游戏 / 设置）");
                        self.game.hud.esc_menu_open = true;
                        self.game.hud.esc_menu_selection = 0;
                        // 立即释放鼠标捕获（不等下一帧 sync_cursor）：否则用户立刻移动
                        // 点击时 last_cursor 仍是捕获中心 → 菜单选项命中错位
                        if self.cursor_captured {
                            if let Some(window) = &self.window {
                                let _ = window.set_cursor_grab(CursorGrabMode::None);
                                window.set_cursor_visible(true);
                            }
                            self.cursor_captured = false;
                            self.cursor_locked = false;
                            self.abs_baseline_valid = false;
                            self.recenter_pending_until = None;
                            log::info!("input: cursor released (ESC menu opened)");
                        }
                    }
                    return;
                }

                // ESC 菜单导航：Tab 切换选项（0=退出 1=设置），Enter 确认，其它键关闭菜单
                if self.game.hud.esc_menu_open {
                    if pressed && key_code == KeyCode::Tab {
                        self.game.hud.esc_menu_selection = (self.game.hud.esc_menu_selection + 1) % 2;
                        log::info!("ESC 菜单选中: {}", if self.game.hud.esc_menu_selection == 0 { "退出游戏" } else { "设置" });
                        return;
                    }
                    if pressed && key_code == KeyCode::Enter {
                        if self.game.hud.esc_menu_selection == 0 {
                            log::info!("ESC 菜单：退出游戏");
                            self.running = false;
                            event_loop.exit();
                        } else {
                            log::info!("ESC 菜单：打开设置");
                            self.game.hud.esc_menu_open = false;
                            self.game.toggle_settings();
                        }
                        return;
                    }
                    if pressed && key_code != KeyCode::Escape {
                        self.game.hud.esc_menu_open = false;
                    }
                }

                // 键位驱动：查当前键码绑定的可重绑定动作（移动/换弹/开火/菜单）
                if let Some(action) = self.game.hud.key_bindings.action_for(key_code as u32) {
                    match action {
                        BindingAction::Forward => {
                            self.key_state.forward = pressed;
                            self.sync_game_movement();
                        }
                        BindingAction::Backward => {
                            self.key_state.backward = pressed;
                            self.sync_game_movement();
                        }
                        BindingAction::Left => {
                            self.key_state.left = pressed;
                            self.sync_game_movement();
                        }
                        BindingAction::Right => {
                            self.key_state.right = pressed;
                            self.sync_game_movement();
                        }
                        BindingAction::Reload => {
                            if pressed {
                                let st = self.game.state();
                                if st == GameState::GameOver
                                    || matches!(st, GameState::Victory(_) | GameState::Defeat)
                                {
                                    log::info!("game: 重开本关");
                                    self.game.request_restart(&self.camera.position());
                                } else if st == GameState::Playing && !self.game.settings_open() {
                                    self.game.request_reload();
                                }
                            }
                        }
                        BindingAction::Fire => {
                            if pressed
                                && !self.game.settings_open()
                                && !self.command_open
                                && self.game.state() == GameState::Playing
                            {
                                self.fire_requested = true;
                                self.fire_edge = true;
                            } else if !pressed {
                                self.fire_requested = false;
                            }
                        }
                        BindingAction::Jump => {
                            // Space 跳跃（2026-08-15：开火改鼠标左键，Space 让位给跳跃）
                            if self.game.state() == GameState::Playing && !self.game.settings_open() {
                                self.game.jump_requested(pressed);
                            }
                        }
                        BindingAction::Menu => {
                            if pressed && !self.game.settings_open() {
                                log::info!("键位菜单键：打开设置面板");
                                self.game.toggle_settings();
                            }
                        }
                    }
                    return;
                }

                // 系统键（不可重绑定）：Tab 设置循环/相机切换，Q/E 升降，N 补给
                match key_code {
                    // Tab：设置面板打开时循环选中项；否则切换相机模式
                    KeyCode::Tab => {
                        if pressed {
                            if self.game.settings_open() {
                                self.game.cycle_settings();
                            } else {
                                let mode = self.camera.toggle_mode();
                                log::info!("相机模式切换: {:?}", mode);
                            }
                        }
                    }
                    KeyCode::KeyQ => self.key_state.down = pressed,
                    KeyCode::KeyE => self.key_state.up = pressed,
                    // 数字键 1/2：切换武器（M1 Rifle / Thompson SMG）
                    KeyCode::Digit1 => {
                        if pressed && self.game.state() == GameState::Playing && !self.game.settings_open() {
                            self.game.switch_weapon(0);
                        }
                    }
                    KeyCode::Digit2 => {
                        if pressed && self.game.state() == GameState::Playing && !self.game.settings_open() {
                            self.game.switch_weapon(1);
                        }
                    }
                    // B：切换开火模式（单发 / 三连发 / 连发）
                    KeyCode::KeyB => {
                        if pressed
                            && self.game.state() == GameState::Playing
                            && !self.game.settings_open()
                        {
                            self.game.cycle_fire_mode();
                            log::info!(
                                "command: 开火模式 -> {}",
                                self.game.fire_mode().label()
                            );
                        }
                    }
                    // G：投掷手榴弹（抛物线 + 引信 1.5-2.5s + 爆炸复用）
                    KeyCode::KeyG => {
                        if pressed && self.game.state() == GameState::Playing && !self.game.settings_open() {
                            let eye = self.game.player_eye();
                            let dir = self.camera.forward();
                            self.game.throw_grenade(
                                [eye.x, eye.y, eye.z],
                                [dir.x, dir.y, dir.z],
                            );
                        }
                    }
                    // Enter / 斜杠 /：打开命令输入窗口（类 MC：/ 打开、数字切枪、回车执行）。
                    // 设置面板打开时 Enter 仍走行循环/键位绑定逻辑
                    KeyCode::Enter | KeyCode::Slash => {
                        if pressed && self.game.settings_open() && key_code == KeyCode::Enter {
                            match self.game.hud.settings_selection() {
                                3 => {
                                    // RESOLUTION 行：循环切换分辨率并即时应用
                                    self.game.hud.cycle_resolution();
                                    self.apply_resolution();
                                }
                                4 => {
                                    // QUALITY 行：循环切换画质并即时应用
                                    self.game.hud.cycle_quality();
                                    self.apply_quality();
                                }
                                _ => {
                                    log::info!("settings: Enter 进入键位绑定");
                                    self.game.begin_rebind();
                                }
                            }
                        } else if pressed
                            && self.game.state() == GameState::Playing
                            && !self.game.hud.esc_menu_open
                        {
                            log::info!("command: 打开命令窗口（/）");
                            self.command_open = true;
                            self.command_buf.clear();
                            // 防卡键：清移动/开镜状态（窗口打开期间不响应移动/开火）
                            self.key_state.reset();
                            self.game.set_movement(false, false, false, false);
                            self.ads_active = false;
                        } else if pressed && key_code == KeyCode::Enter {
                            // Enter 且非 Playing：死亡/胜利结算重开本关
                            let st = self.game.state();
                            if st == GameState::GameOver
                                || matches!(st, GameState::Victory(_) | GameState::Defeat)
                            {
                                log::info!("game: Enter 重开本关");
                                self.game.request_restart(&self.camera.position());
                            }
                        }
                    }
                    // 设置面板调试补给（N 键补满弹匣）；胜利结算 N 键进入下一关
                    KeyCode::KeyN => {
                        if pressed && self.game.settings_open() {
                            log::info!("settings: N 键补给弹药");
                            self.game.give_ammo();
                        } else if pressed && matches!(self.game.state(), GameState::Victory(_)) {
                            if self.game.advance_level(&self.camera.position()) {
                                log::info!("game: N 进入下一关");
                            } else {
                                log::info!("game: 已通关（最后一关完成）");
                            }
                        }
                    }
                    // F5：关卡系统热重载（重新读取当前地图 TOML）
                    KeyCode::F5 => {
                        if pressed {
                            match self.game.reload_current_map() {
                                Ok(()) => log::info!("map: F5 热重载完成"),
                                Err(e) => log::warn!("map: F5 热重载失败: {}", e),
                            }
                        }
                    }
                    // F12：截图（任意画面可用，Windows 写到 ./screenshots/）
                    KeyCode::F12 => {
                        if pressed {
                            self.capture_screenshot();
                        }
                    }
                    _ => {}
                }
            }

            // 焦点变化：失焦时重置按键/拖拽并释放捕获，防止"卡键"
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if !focused {
                    self.key_state.reset();
                    self.game.set_movement(false, false, false, false);
                    self.dragging = false;
                    self.right_dragging = false;
                    self.camera.set_rotation_active(false);
                    // 失焦立即释放鼠标捕获（Win 键呼出菜单栏/Alt-Tab 时窗口失焦，
                    // 不等待下一帧 sync_cursor——否则鼠标被锁住只能 Alt+F4 强退）
                    if self.cursor_captured {
                        if let Some(window) = &self.window {
                            let _ = window.set_cursor_grab(CursorGrabMode::None);
                            window.set_cursor_visible(true);
                        }
                        self.cursor_captured = false;
                        self.cursor_locked = false;
                        self.abs_baseline_valid = false;
                        self.recenter_pending_until = None;
                        log::info!("input: cursor released (window unfocused)");
                    }
                }
            }

            // 鼠标按键：左键 = 开火（Playing）兼轨道拖拽；右键 = 飞行视角拖拽
            WindowEvent::MouseInput {
                state, button, ..
            } => {
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        // ESC 菜单打开：点击命中选项（退出/设置），不触发开火
                        if pressed && self.game.hud.esc_menu_open {
                            let (mx, my) = self.last_cursor;
                            if self.menu_click_hit(mx as f32, my as f32) {
                                log::info!("ESC 菜单鼠标点击选项");
                            }
                            return;
                        }
                        // 设置面板打开：点击命中行（选择/调节），不触发开火
                        if pressed && self.game.settings_open() {
                            let (mx, my) = self.last_cursor;
                            self.settings_click(mx as f32, my as f32);
                            return;
                        }
                        if pressed && !self.game.settings_open() {
                            // 开始菜单/加载中：点击也视为"任意键"开局（键盘焦点不可靠的环境兜底）
                            let st = self.game.state();
                            if st == GameState::StartMenu || st == GameState::LoadingMap {
                                self.game.on_any_key(&self.camera.position());
                            }
                            if st == GameState::Playing && !self.command_open {
                                self.fire_requested = true;
                                self.fire_edge = true;
                            }
                        } else if !pressed {
                            // 松开左键：停止连发
                            self.fire_requested = false;
                        }
                        self.dragging = pressed && !self.game.settings_open();
                    }
                    MouseButton::Right => {
                        // 第一人称：右键 = 开镜瞄准（ADS）；飞行模式保留右键拖拽转视角
                        if self.camera.mode == CameraMode::FirstPerson
                            && self.game.state() == GameState::Playing
                            && !self.game.settings_open()
                            && !self.command_open
                        {
                            self.ads_active = pressed;
                        } else {
                            self.right_dragging = pressed;
                            self.camera.set_rotation_active(pressed);
                        }
                    }
                    _ => {}
                }
            }

            // 鼠标移动（绝对位置）：非捕获态拖拽旋转；捕获态只重基准不驱动视角
            WindowEvent::CursorMoved { position, .. } => {
                let (px, py) = (position.x, position.y);
                // warp 回声事件吞噬窗口：recenter 后短时间内的下一个 CursorMoved
                // 只是回中回声，把它作为新基准并跳过，防止回声环把落点偏移当视角位移
                if let Some(until) = self.recenter_pending_until {
                    self.recenter_pending_until = None;
                    if Instant::now() < until {
                        self.last_cursor = (px, py);
                        return;
                    }
                }
                if self.cursor_captured {
                    if self.cursor_locked {
                        // Locked grab：raw 相对增量已驱动视角，绝对位置只更新基准
                        self.last_cursor = (px, py);
                        return;
                    }
                    // WSLg/Xwayland 回退：绝对位置路径。
                    // 基准 = 真实指针位置（或 warp 成功确认后的窗口中心）——
                    // 绝不把 last_cursor 假设为 warp 目标（旧 bug：warp 失败仍把
                    // 基准设成中心，指针距中心偏差被当视角位移 → 灵敏度爆炸/压地）。
                    if !self.abs_baseline_valid {
                        self.abs_baseline_valid = true;
                        self.last_cursor = (px, py);
                        return;
                    }
                    let dx = px - self.last_cursor.0;
                    let dy = py - self.last_cursor.1;
                    // 光标传送（服务端跳变）：跳过该事件，只重基准
                    if dx.abs() <= MAX_LOOK_DELTA_PX && dy.abs() <= MAX_LOOK_DELTA_PX {
                        match self.camera.mode {
                            CameraMode::FirstPerson => self.camera.look(dx as f32, dy as f32),
                            CameraMode::Orbit => self.camera.orbit(dx as f32, dy as f32),
                            CameraMode::Flight => {
                                self.camera.set_rotation_active(true);
                                self.camera.add_rotation_input(dx as f32, dy as f32);
                            }
                        }
                    }
                    // 回中指针（避免撞窗口边缘停顿）：warp 成功 → 基准=中心；
                    // 失败 → 基准=当前真实位置（下一事件从真实位置算增量）。
                    if let Some(window) = &self.window {
                        let size = window.inner_size();
                        let center = winit::dpi::PhysicalPosition::new(
                            size.width as f64 / 2.0,
                            size.height as f64 / 2.0,
                        );
                        if window.set_cursor_position(center).is_ok() {
                            self.last_cursor = (center.x, center.y);
                        } else {
                            self.last_cursor = (px, py);
                        }
                    } else {
                        self.last_cursor = (px, py);
                    }
                } else {
                    let (dx, dy) = (px - self.last_cursor.0, py - self.last_cursor.1);
                    // 非捕获态拖拽视角（菜单/设置预览 + 冒烟在无焦点环境下的瞄准路径）：
                    // 左键按住 = 轨道/第一人称转视角，右键 = 飞行视角
                    // 跳变（warp/传送）事件不转视角，只重基准
                    let teleported = (px - self.last_cursor.0).abs() > MAX_LOOK_DELTA_PX
                        || (py - self.last_cursor.1).abs() > MAX_LOOK_DELTA_PX;
                    if self.dragging && !teleported {
                        match self.camera.mode {
                            CameraMode::Orbit => self.camera.orbit(dx as f32, dy as f32),
                            CameraMode::FirstPerson => self.camera.look(dx as f32, dy as f32),
                            CameraMode::Flight => {}
                        }
                    }
                    if self.right_dragging && self.camera.mode == CameraMode::Flight && !teleported {
                        self.camera.add_rotation_input(dx as f32, dy as f32);
                    }
                    // 拖拽转视角时回中光标，避免把指针拖出窗口导致事件丢失（与捕获态一致）
                    if self.dragging
                        && (self.camera.mode == CameraMode::Orbit
                            || self.camera.mode == CameraMode::FirstPerson)
                    {
                        if let Some(window) = &self.window {
                            let size = window.inner_size();
                            let center = winit::dpi::PhysicalPosition::new(
                                size.width as f64 / 2.0,
                                size.height as f64 / 2.0,
                            );
                            let _ = window.set_cursor_position(center);
                            self.last_cursor = (center.x, center.y);
                            self.recenter_pending_until =
                                Some(Instant::now() + Duration::from_millis(150));
                        }
                    } else {
                        self.last_cursor = (px, py);
                    }
                }
            }

            // 滚轮：轨道 = 推拉距离；飞行 = 沿视线前进/后退
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f32,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.05,
                };
                if self.game.settings_open() {
                    self.game.adjust_settings(scroll * 0.05);
                } else {
                    match self.camera.mode {
                        CameraMode::FirstPerson => {
                            // 第一人称：滚轮切换武器（上=下一把，下=上一把）
                            self.game.cycle_weapon(scroll.round() as i32);
                        }
                        CameraMode::Orbit => self.camera.zoom(scroll),
                        CameraMode::Flight => self.camera.flight_wheel(scroll),
                    }
                }
            }

            // 光标移出窗口：停止拖拽，防止视角卡住
            WindowEvent::CursorLeft { .. } => {
                self.dragging = false;
                self.right_dragging = false;
                self.camera.set_rotation_active(false);
            }

            // 窗口大小变化时重建交换链
            WindowEvent::Resized(new_size) => {
                if new_size.width == 0 || new_size.height == 0 {
                    return; // 窗口最小化
                }
                log::info!("窗口大小变化: {}x{}", new_size.width, new_size.height);
                self.game
                    .hud
                    .set_screen_size(new_size.width as f32, new_size.height as f32);
                if let Some(renderer) = &mut self.renderer {
                    let _ = renderer.recreate_swapchain();
                }
            }

            _ => {}
        }
    }

    /// 事件队列空闲时调用（主循环体）
    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if !self.running || self.window.is_none() {
            return;
        }

        // 帧率门控（MAX_FPS=0 时无上限，压测模式不做 sleep/spin）
        if FRAME_BUDGET > Duration::ZERO {
            // thread::sleep 粒度约 1ms，先粗睡到剩 ~1ms，再自旋精确到预算
            let elapsed = self.last_frame.elapsed();
            if elapsed < FRAME_BUDGET {
                let remaining = FRAME_BUDGET - elapsed;
                if remaining > Duration::from_millis(1) {
                    std::thread::sleep(remaining - Duration::from_millis(1));
                }
                while self.last_frame.elapsed() < FRAME_BUDGET {
                    std::hint::spin_loop();
                }
            }
        }

        // 更新逻辑（相机、物理等）+ 渲染（记录周期/分阶段耗时供性能定位）
        let cycle_start = Instant::now();
        let update_start = Instant::now();
        self.update();
        let update_us = update_start.elapsed().as_micros() as u64;
        let render_start = Instant::now();
        self.render();
        self.last_render_us = render_start.elapsed().as_micros() as u64;
        self.last_update_us = update_us;
        self.last_cycle_us = cycle_start.elapsed().as_micros() as u64;
        // 采集模式帧率上限（RV3D_LLM=1 时 90FPS 封顶）：大幅降低 GPU 负载，
        // 避免与 llama-server 长时间同卡共存导致 VK_ERROR_DEVICE_LOST（2026-08-23）
        if self.llm_cap_fps > 0.0 {
            let used = cycle_start.elapsed().as_secs_f32();
            let target = 1.0 / self.llm_cap_fps;
            if used < target {
                std::thread::sleep(std::time::Duration::from_secs_f32(target - used));
            }
        }
    }
}

/// 程序入口点
fn main() {
    // 初始化日志系统
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 默认大战场：红 128 vs 蓝 127+玩家（=128v128，2026-08-22 要求海量 NPC 模拟真人压力）；
    // RV3D_STRESS_AI=N 自定义，=0 恢复传统波次模式
    if std::env::var("RV3D_STRESS_AI").is_err() {
        std::env::set_var("RV3D_STRESS_AI", "128");
    }

    // 资源路径导向（启动器写入 resource_paths.ini）：记录地图/音效/建模自定义目录
    if let Ok(text) = std::fs::read_to_string("resource_paths.ini") {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let (k, v) = (k.trim(), v.trim());
                if !v.is_empty() {
                    log::info!("res-path: {} -> {}", k, v);
                    let env_k = k
                        .trim_end_matches("_path")
                        .to_uppercase()
                        .replace('-', "_");
                    std::env::set_var(format!("STEELFRONT_{}", env_k), v);
                }
            }
        }
    }

    // DPI awareness 由 winit 0.30 自己管理（默认 per-monitor V2）——手动调用
    // SetProcessDpiAwarenessContext 会与 winit 内部设置冲突，导致窗口尺寸/缩放错位
    // （曾出现：swapchain 2560x1600 但窗口实际 1898x1061 → 画面只显示左上角）。

    // 中文字形按需惰性生成（font_cjk 缓存）；不预填充——GDI 光栅化会阻塞启动首帧

    log::info!("========================================");
    log::info!("  钢铁前线 (Steel Front) v{}", env!("CARGO_PKG_VERSION"));
    log::info!("  二战FPS游戏引擎 - Rust + Vulkan");
    log::info!("========================================");

    // CPU 拓扑检测（全局缓存，Game/Renderer 复用同一份）+ 主线程亲和性绑定
    // （AMD 双簇/Intel 混合；RV3D_CPU_PIN=off 可关）。渲染线程不固定 1-2 核：
    // 主线程绑的是整簇集合（CCD0/P-core），OS 调度器把渲染工作分给集合内空闲率最高的核。
    let cpu = engine::cpu::topology();
    cpu.log_summary();
    cpu.pin_main_thread();

    // WSLg（WSL2 + Wayland/Weston）的指针约束/相对指针协议支持不完整：
    // 捕获后光标不隐藏、视角不动，且右键拖动会在原生层静默崩溃（无 panic 日志）。
    // Xwayland 提供完整 XInput2 raw motion（本项目视角输入依赖，见 device_event），
    // 因此 WSL + Wayland 会话下强制 X11 后端。
    // 注意：winit 0.29+ 已删除 WINIT_UNIX_BACKEND 环境变量（v0.29 changelog），
    // 必须经 EventLoopBuilderExtX11::with_x11() 设置 forced_backend 才真正生效。
    #[cfg(target_os = "linux")]
    let force_x11 = {
        let is_wsl = std::fs::read_to_string("/proc/version")
            .map(|v| v.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false);
        if is_wsl && std::env::var_os("WAYLAND_DISPLAY").is_some() {
            log::info!(
                "input: WSLg Wayland 指针支持不完整，强制 X11 后端（Xwayland + XInput2 raw motion）"
            );
            true
        } else {
            false
        }
    };
    // 创建事件循环（WSLg 下强制 X11，走 Xwayland + XInput2 raw motion）
    let event_loop = {
        let mut builder = EventLoop::builder();
        #[cfg(target_os = "linux")]
        if force_x11 {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }
        match builder.build() {
            Ok(el) => el,
            Err(e) => {
                log::error!("创建事件循环失败: {:?}", e);
                return;
            }
        }
    };

    // 设置控制流为 Poll（持续轮询，适合游戏）
    event_loop.set_control_flow(ControlFlow::Poll);

    // 创建并运行游戏应用：捕获态视角一律由 XInput2 raw 相对增量驱动
    // （与指针位置无关，无 warp 回声环）；绝对位置仅用于非捕获拖拽路径。
    let mut app = GameApp::new();
    // 菜单点击退出用的事件循环代理（app 创建后设置）
    app.event_proxy = Some(event_loop.create_proxy());

    // 网络对战模式（默认关闭，不破坏单机）：RV3D_NET=server|client，
    // RV3D_NET_ADDR=127.0.0.1:<port>（默认 127.0.0.1:27015）。
    // 服务器：权威模拟 + 每 tick 广播快照；客户端：输入上报 + 快照插值缓冲。
    // 无头回环集成测试在 net.rs / game.rs（不依赖 Vulkan/winit）；
    // 渲染远端实体、NAT 穿透、断线重连为后续 TODO。
    let net_role = std::env::var("RV3D_NET").unwrap_or_default();
    let net_addr =
        std::env::var("RV3D_NET_ADDR").unwrap_or_else(|_| "127.0.0.1:27015".to_string());
    // NAT 中继（RV3D_NET_RDV=<host:port> + RV3D_NET_NAME=房间名）：
    // 服务器向中继注册；客户端查询房间名→公网地址直连（NAT 打洞第一步）
    let net_rdv = std::env::var("RV3D_NET_RDV").ok();
    let net_name = std::env::var("RV3D_NET_NAME").unwrap_or_else(|_| "steel".to_string());
    match net_role.as_str() {
        "server" => match Server::bind(&net_addr) {
            Ok(server) => {
                let addr = server
                    .local_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| net_addr.clone());
                log::info!("net: 服务器模式，监听 {}", addr);
                if let Some(rdv) = &net_rdv {
                    let port = net_addr
                        .rsplit(':')
                        .next()
                        .and_then(|p| p.parse::<u16>().ok())
                        .unwrap_or(27015);
                    let _ = server.rdv_register(rdv, &net_name, port);
                    log::info!("net: 已向中继 {rdv} 注册房间 {net_name}（端口 {port}，等待玩家查询）");
                }
                app.game.set_net_server(server);
            }
            Err(e) => log::error!("net: 服务器绑定 {} 失败: {}", net_addr, e),
        },
        "client" => {
            // 中继解析：通过房间名拿到主机公网地址（打洞探测已在 rdv_resolve 内发出）
            let target = if let Some(rdv) = &net_rdv {
                match crate::net::rdv_resolve(rdv, &net_name) {
                    Ok(a) => {
                        log::info!("net: 中继解析房间 {net_name} → {}", a);
                        a.to_string()
                    }
                    Err(e) => {
                        log::error!("net: 中继解析 {net_name} 失败: {e}（改用直连地址）");
                        net_addr.clone()
                    }
                }
            } else {
                net_addr.clone()
            };
            match Client::connect(&target) {
                Ok(client) => {
                    log::info!("net: 客户端模式，连接 {}", client.server_addr());
                    app.game.set_net_client(client);
                }
                Err(e) => log::error!("net: 客户端连接 {} 失败: {}", target, e),
            }
        },
        other => {
            if !other.is_empty() {
                log::warn!("net: 未知 RV3D_NET 值 {:?}（应为 server|client），忽略", other);
            }
        }
    }

    if let Err(e) = event_loop.run_app(&mut app) {
        log::error!("应用运行错误: {:?}", e);
    }

    log::info!("程序正常退出");
}
 
