//! 钢铁前线 (Steel Front) - 程序入口
//!
//! 游戏主循环：
//! 1. 初始化窗口（winit）
//! 2. 初始化 Vulkan 渲染器（ash）
//! 3. 事件循环处理输入
//! 4. 每帧更新相机并渲染

mod engine;
mod audio;
mod net;
mod ui;
mod config;

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
use engine::game::{Game, GameState, ObstacleKind};
use engine::renderer::{QualityPreset, Renderer};
use engine::window;
use net::{Client, Server};
use ui::{BindingAction, KeyBindings, RESOLUTIONS};
use winit::window::CursorGrabMode;

/// 单次鼠标位移最大像素：超过视为光标传送伪事件（X 服务端 warp/焦点切换跳变），
/// 跳过该事件并重基准 last_cursor，防止第一人称视角跳变/自转。
const MAX_LOOK_DELTA_PX: f64 = 512.0;

/// 帧率上限（present 节流）：0 = 无上限（压测模式，主循环全速跑以暴露渲染瓶颈）。
/// 设回正数（如 300）即恢复帧率门控。
const MAX_FPS: u64 = 0;
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
    /// 上一帧光标位置（屏幕坐标）
    last_cursor: (f64, f64),
    /// 上一帧时间戳（用于 delta_time 计算）
    last_frame: Instant,
    /// 上一帧 update+render 总耗时（微秒，性能日志用）
    last_cycle_us: u64,
    /// 上一帧 update（逻辑）耗时（微秒，性能日志用）
    last_update_us: u64,
    /// 上一帧 render（渲染提交）耗时（微秒，性能日志用）
    last_render_us: u64,
    /// 是否请求开火（Space 按下置位，update 消费）
    fire_requested: bool,
    /// 光标是否已捕获（Playing 下鼠标视角）
    cursor_captured: bool,
    /// 捕获模式是否为系统级 Locked（相对 MouseMotion 驱动视角，光标不出窗口）；
    /// false = 回退 Confined + warp 回中（绝对位置路径）
    cursor_locked: bool,
    /// 本会话是否收到过 DeviceEvent::MouseMotion（XInput2 相对增量）。
    /// 收到后视角改由相对事件驱动，绝对位置路径只作基准，避免 warp 回声乱转。
    mouse_relative_ok: bool,
    /// 窗口是否聚焦（失焦时释放捕获，防止卡视角）
    focused: bool,
    /// 回中 warp 回声事件吞噬窗口：recenter 后短时间内到达的下一个 CursorMoved
    /// 视为 warp 回声（只作新基准、不应用视角位移），防止回声环把落点偏移当视角。
    recenter_pending_until: Option<Instant>,
    /// 上次相机参数日志时间（1 秒一条，冒烟/调试用）
    last_cam_log: Instant,
    /// 游戏运行时中枢（物理/武器/AI/UI/音频/网络）
    game: Game,
    /// 程序是否正在运行
    running: bool,
    /// 配置中是否显式保存过分辨率（false = 首次运行，窗口创建时按显示器宽高比选默认）
    resolution_explicit: bool,
}

impl GameApp {
    /// 创建游戏应用实例
    fn new() -> Self {
        let mut game = Game::new();
        // 加载持久化配置（键位/音量/灵敏度）；文件缺失回退默认，见 config.rs
        let cfg = config::load();
        game.hud.volume = cfg.volume;
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
            last_cursor: (0.0, 0.0),
            last_frame: Instant::now(),
            last_cycle_us: 0,
            last_update_us: 0,
            last_render_us: 0,
            fire_requested: false,
            cursor_captured: false,
            cursor_locked: false,
            mouse_relative_ok: false,
            focused: true,
            recenter_pending_until: None,
            last_cam_log: Instant::now(),
            game,
            running: true,
            resolution_explicit: cfg.resolution_explicit,
        }
    }

    /// 更新逻辑（每帧调用）
    fn update(&mut self) {
        // 同步光标捕获状态（Playing + 聚焦 = 捕获；菜单/结算/失焦 = 释放）
        self.sync_cursor();

        // 计算帧时间差
        let now = Instant::now();
        let delta_time = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        // 确保 delta_time 不会太大（防止卡顿时大跳）
        let delta_time = delta_time.min(0.1);

        // 更新相机（双模式：轨道/飞行，含惯性速度与边界 clamp）
        self.camera.update(&self.key_state, delta_time);

        // 更新游戏逻辑（物理、武器、AI 等）
        // 先把本帧开火意图转发给网络层（客户端模式随 Input 上报服务端）
        self.game.set_net_fire(self.fire_requested);
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

        // 开火：从相机位置沿视线发射投射物
        if self.fire_requested {
            let pos = self.camera.position();
            let dir = self.camera.forward();
            self.game
                .fire([pos.x, pos.y, pos.z], [dir.x, dir.y, dir.z]);
            self.fire_requested = false;
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

    /// 按游戏状态同步光标捕获：Playing = 捕获 + 隐藏；否则释放。
    fn sync_cursor(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let want = self.focused
            && self.game.state() == GameState::Playing
            && !self.game.settings_open();
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
            self.mouse_relative_ok = false;
            if !locked {
                // 捕获瞬间回中光标：随后 warp 回声事件被吞掉作为新基准，
                // 避免把"捕获前光标位置到窗口中心"的差量当成视角位移
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
                "input: cursor captured (mouse look on, grab={})",
                if locked {
                    "locked"
                } else if grabbed {
                    "confined"
                } else {
                    "none-relative"
                }
            );
        } else if !want && self.cursor_captured {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
            self.cursor_captured = false;
            self.cursor_locked = false;
            self.mouse_relative_ok = false;
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

    /// F12 截图：调渲染器把当前帧保存到 /tmp/steel_front_<秒时间戳>.png
    fn capture_screenshot(&mut self) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
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
            let view = self.camera.view_matrix();
            // 投影矩阵不翻转 Y：主 shader（triangle.vert.spv）已在 gl_Position.y 上完成
            // Vulkan 翻转，若这里再翻一次会双重翻转导致画面上下颠倒（与 HUD shader 一致）。
            let proj = self.camera.projection_matrix(aspect);

            // HUD：用上一帧渲染统计生成覆盖层 quad 并上传（首帧统计为 0）
            let (near, far, lod) = renderer.last_stats();
            let quads = self.game.hud_quads(near, far, lod);
            renderer.set_hud_quads(&quads);
            renderer.set_lights(&self.game.light_uniform());
            // 世界障碍 marker：关卡地图障碍盒 → 红色盒实例（复用主 pipeline，见 renderer.rs MARKER_SLOT_BASE）
            let markers: Vec<engine::renderer::WorldMarker> = self
                .game
                .map_obstacles()
                .iter()
                .map(|ob| {
                    // 障碍种类配色：墙=红、块=橙、栅栏=蓝灰（便于区分地图主题）
                    let tint = match ob.kind {
                        ObstacleKind::Wall => [0.85, 0.25, 0.15, 1.0],
                        ObstacleKind::Block => [0.85, 0.6, 0.15, 1.0],
                        ObstacleKind::Barrier => [0.35, 0.5, 0.8, 1.0],
                    };
                    engine::renderer::WorldMarker {
                        model: glam::Mat4::from_translation(glam::Vec3::new(ob.x, 1.2, ob.z))
                            * glam::Mat4::from_scale(glam::Vec3::new(
                                ob.half_w * 2.0,
                                2.4,
                                ob.half_d * 2.0,
                            )),
                        tint,
                    }
                })
                .collect();
            // 爆炸闪光：冲击波球壳随年龄膨胀、颜色转淡；走自发光路径（emissive 槽位，
            // shader 直出纯色跳过光照/贴图混合），夜间等暗光环境下依然清晰可见
            let emissive_markers: Vec<engine::renderer::WorldMarker> = self
                .game
                .explosions()
                .iter()
                .map(|ex| {
                let t = (ex.age / ex.lifetime).clamp(0.0, 1.0);
                let s = ex.radius * (0.35 + 1.65 * t);
                let h = (2.4 * (1.0 - t)).max(0.3);
                engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(ex.center[0], 1.2, ex.center[2]))
                        * glam::Mat4::from_scale(glam::Vec3::new(s, h, s)),
                    tint: [1.0, 0.55 * (1.0 - t) + 0.2, 0.08, 1.0],
                }
                })
                .collect();
            renderer.set_world_markers(&markers);
            renderer.set_emissive_markers(&emissive_markers);
            // NPC 士兵可视化：每个 NPC 由 renderer 展开为 7 段积木人（头/躯干/四肢/枪），
            // 按朝向旋转，阵营配色（红=敌军、蓝=友军/玩家阵营）
            let npc_visuals: Vec<engine::renderer::NpcVisual> = self
                .game
                .npcs
                .iter()
                .map(|n| engine::renderer::NpcVisual {
                    pos: n.position,
                    yaw: n.facing,
                    tint: match n.team {
                        // 纯色渲染（shader flat_flag 路径）：高饱和阵营色，避免与灰地/障碍混淆
                        Team::Red => [0.95, 0.12, 0.08, 1.0],
                        Team::Blue => [0.08, 0.35, 0.98, 1.0],
                    },
                })
                .collect();
            renderer.set_npc_visuals(&npc_visuals);

            if let Err(e) = renderer.render(view, proj) {
                if e == "交换链过期" {
                    log::warn!("交换链过期，尝试重建...");
                    let _ = renderer.recreate_swapchain();
                } else {
                    log::error!("渲染错误: {}", e);
                }
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
        let (w, h) = self.game.hud.resolution();
        let winit_attr = Window::default_attributes()
            .with_title(window::WINDOW_TITLE)
            .with_inner_size(winit::dpi::PhysicalSize::new(w, h));

        let window = match event_loop.create_window(winit_attr) {
            Ok(w) => w,
            Err(e) => {
                log::error!("创建窗口失败: {:?}", e);
                event_loop.exit();
                return;
            }
        };

        log::info!("窗口创建成功: {}x{}", w, h);

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
    }

    /// 设备级事件：系统相对鼠标增量（XInput2 raw motion，与光标位置无关）驱动视角。
    /// 本环境（WSLg/Xwayland）grab 不可靠，此路径是视角不飞出窗口、不产生 warp 回声乱转的关键。
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            self.mouse_relative_ok = true;
            if self.cursor_captured {
                let (dx, dy) = (delta.0 as f32, delta.1 as f32);
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

                // 开始菜单：任意键（除 ESC）开始游戏
                if pressed
                    && self.game.state() == GameState::StartMenu
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

                // ESC 是保留系统键（不参与重绑定）：设置面板打开时关闭；
                // 否则两段式确认退出（首次显示提示，再按一次才退出，任意其它键取消）
                if pressed && key_code == KeyCode::Escape {
                    if self.game.settings_open() {
                        log::info!("ESC 关闭设置面板");
                        self.game.toggle_settings();
                    } else if self.game.hud.confirm_quit {
                        log::info!("ESC 再次按下，确认退出");
                        self.running = false;
                        event_loop.exit();
                    } else {
                        log::info!("ESC 按下，再按一次确认退出（任意其它键取消）");
                        self.game.hud.confirm_quit = true;
                    }
                    return;
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
                                if self.game.state() == GameState::GameOver {
                                    log::info!("game: 重开一局");
                                    self.game.request_restart(&self.camera.position());
                                } else if self.game.state() == GameState::Playing
                                    && !self.game.settings_open()
                                {
                                    self.game.request_reload();
                                }
                            }
                        }
                        BindingAction::Fire => {
                            self.fire_requested = pressed && !self.game.settings_open();
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
                    // Enter：设置面板选中分辨率/画质行时循环切换；选中键位行时进入"等待按键绑定"
                    KeyCode::Enter => {
                        if pressed && self.game.settings_open() {
                            match self.game.hud.settings_selection() {
                                2 => {
                                    // RESOLUTION 行：循环切换分辨率并即时应用
                                    self.game.hud.cycle_resolution();
                                    self.apply_resolution();
                                }
                                3 => {
                                    // QUALITY 行：循环切换画质并即时应用
                                    self.game.hud.cycle_quality();
                                    self.apply_quality();
                                }
                                _ => {
                                    log::info!("settings: Enter 进入键位绑定");
                                    self.game.begin_rebind();
                                }
                            }
                        } else if pressed && self.game.state() == GameState::GameOver {
                            log::info!("game: Enter 重开一局");
                            self.game.request_restart(&self.camera.position());
                        }
                    }
                    // 设置面板调试补给（N 键补满弹匣）
                    KeyCode::KeyN => {
                        if pressed && self.game.settings_open() {
                            log::info!("settings: N 键补给弹药");
                            self.game.give_ammo();
                        }
                    }
                    // F12：截图（任意画面可用，渲染器把当前帧写到 /tmp）
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
                }
            }

            // 鼠标按键：左键 = 开火（Playing）兼轨道拖拽；右键 = 飞行视角拖拽
            WindowEvent::MouseInput {
                state, button, ..
            } => {
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        if pressed && !self.game.settings_open() {
                            // 开始菜单：点击也视为"任意键"开局（键盘焦点不可靠的环境兜底）
                            if self.game.state() == GameState::StartMenu {
                                self.game.on_any_key(&self.camera.position());
                            }
                            if self.game.state() == GameState::Playing {
                                self.fire_requested = true;
                            }
                        }
                        self.dragging = pressed && !self.game.settings_open();
                    }
                    MouseButton::Right => {
                        self.right_dragging = pressed;
                        self.camera.set_rotation_active(pressed);
                    }
                    _ => {}
                }
            }

            // 鼠标移动（绝对位置）：Confined 回退路径 或 非捕获态拖拽旋转
            WindowEvent::CursorMoved { position, .. } => {
                if self.cursor_locked {
                    return;
                }
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
                    // 相对 MouseMotion 已接管视角：绝对位置只重基准，不再驱动视角，
                    // 避免 Xwayland 下 warp 回声（可能延迟 >150ms 到达）被当成位移导致乱转
                    if self.mouse_relative_ok {
                        self.last_cursor = (px, py);
                        return;
                    }
                    let dx = (px - self.last_cursor.0) as f32;
                    let dy = (py - self.last_cursor.1) as f32;
                    // 光标传送（回中 warp/服务端跳变）：忽略该事件并重基准，防止视角自转
                    if (px - self.last_cursor.0).abs() > MAX_LOOK_DELTA_PX
                        || (py - self.last_cursor.1).abs() > MAX_LOOK_DELTA_PX
                    {
                        self.last_cursor = (px, py);
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
                    // 回中光标，避免撞到屏幕边缘导致视角停顿
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
                        CameraMode::FirstPerson => {}
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
    }
}

/// 程序入口点
fn main() {
    // 初始化日志系统
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

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

    // 创建事件循环
    let event_loop = match EventLoop::new() {
        Ok(el) => el,
        Err(e) => {
            log::error!("创建事件循环失败: {:?}", e);
            return;
        }
    };

    // 设置控制流为 Poll（持续轮询，适合游戏）
    event_loop.set_control_flow(ControlFlow::Poll);

    // 创建并运行游戏应用
    let mut app = GameApp::new();

    // 网络对战模式（默认关闭，不破坏单机）：RV3D_NET=server|client，
    // RV3D_NET_ADDR=127.0.0.1:<port>（默认 127.0.0.1:27015）。
    // 服务器：权威模拟 + 每 tick 广播快照；客户端：输入上报 + 快照插值缓冲。
    // 无头回环集成测试在 net.rs / game.rs（不依赖 Vulkan/winit）；
    // 渲染远端实体、NAT 穿透、断线重连为后续 TODO。
    let net_role = std::env::var("RV3D_NET").unwrap_or_default();
    let net_addr =
        std::env::var("RV3D_NET_ADDR").unwrap_or_else(|_| "127.0.0.1:27015".to_string());
    match net_role.as_str() {
        "server" => match Server::bind(&net_addr) {
            Ok(server) => {
                let addr = server
                    .local_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| net_addr.clone());
                log::info!("net: 服务器模式，监听 {}", addr);
                app.game.set_net_server(server);
            }
            Err(e) => log::error!("net: 服务器绑定 {} 失败: {}", net_addr, e),
        },
        "client" => match Client::connect(&net_addr) {
            Ok(client) => {
                log::info!("net: 客户端模式，连接 {}", client.server_addr());
                app.game.set_net_client(client);
            }
            Err(e) => log::error!("net: 客户端连接 {} 失败: {}", net_addr, e),
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
