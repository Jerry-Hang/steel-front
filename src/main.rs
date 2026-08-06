//! 钢铁前线 (Steel Front) - 程序入口
//!
//! 游戏主循环：
//! 1. 初始化窗口（winit）
//! 2. 初始化 Vulkan 渲染器（ash）
//! 3. 事件循环处理输入
//! 4. 每帧更新相机并渲染

mod engine;

use std::time::{Duration, Instant};

use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use engine::camera::{Camera, KeyState};
use engine::renderer::Renderer;
use engine::window::{self, lock_cursor, unlock_cursor, WINDOW_HEIGHT, WINDOW_WIDTH};

/// 帧率上限（present 节流）：300 FPS，避免空闲轮询无意义打满 GPU
const MAX_FPS: u64 = 300;
/// 单帧预算（纳秒）
const FRAME_BUDGET: Duration = Duration::from_nanos(1_000_000_000 / MAX_FPS);

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
    /// 上一帧时间戳（用于 delta_time 计算）
    last_frame: Instant,
    /// 程序是否正在运行
    running: bool,
}

impl GameApp {
    /// 创建游戏应用实例
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            camera: Camera::new(),
            key_state: KeyState::new(),
            last_frame: Instant::now(),
            running: true,
        }
    }

    /// 更新逻辑（每帧调用）
    fn update(&mut self) {
        // 计算帧时间差
        let now = Instant::now();
        let delta_time = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        // 确保 delta_time 不会太大（防止卡顿时大跳）
        let delta_time = delta_time.min(0.1);

        // 更新相机位置（处理 WASD 输入）
        self.camera.process_keyboard(&self.key_state, delta_time);
    }

    /// 渲染一帧
    fn render(&mut self) {
        if let Some(renderer) = &mut self.renderer {
            let aspect = 1280.0_f32 / 720.0_f32;
            let view_proj = self.camera.view_proj_for_vulkan(aspect);

            if let Err(e) = renderer.render(view_proj) {
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

        // ---- 创建窗口 ----
        let winit_attr = Window::default_attributes()
            .with_title(window::WINDOW_TITLE)
            .with_inner_size(winit::dpi::PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));

        let window = match event_loop.create_window(winit_attr) {
            Ok(w) => w,
            Err(e) => {
                log::error!("创建窗口失败: {:?}", e);
                event_loop.exit();
                return;
            }
        };

        // 锁定鼠标为 FPS 模式
        lock_cursor(&window);
        log::info!("窗口创建成功: {}x{}", WINDOW_WIDTH, WINDOW_HEIGHT);

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
        self.last_frame = Instant::now();
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

                match key_code {
                    KeyCode::Escape => {
                        if pressed {
                            log::info!("ESC 键按下，退出程序");
                            self.running = false;
                            event_loop.exit();
                        }
                    }
                    // WASD 移动键
                    KeyCode::KeyW => self.key_state.forward = pressed,
                    KeyCode::KeyS => self.key_state.backward = pressed,
                    KeyCode::KeyA => self.key_state.left = pressed,
                    KeyCode::KeyD => self.key_state.right = pressed,
                    _ => {}
                }
            }

            // 窗口获得焦点时重新锁定鼠标
            WindowEvent::Focused(true) => {
                if let Some(window) = &self.window {
                    lock_cursor(window);
                }
            }

            // 窗口失去焦点时解锁鼠标
            WindowEvent::Focused(false) => {
                if let Some(window) = &self.window {
                    unlock_cursor(window);
                    // 失去焦点时重置按键状态，防止"卡键"
                    self.key_state.reset();
                }
            }

            // 窗口大小变化时重建交换链
            WindowEvent::Resized(new_size) => {
                if new_size.width == 0 || new_size.height == 0 {
                    return; // 窗口最小化
                }
                log::info!("窗口大小变化: {}x{}", new_size.width, new_size.height);
                if let Some(renderer) = &mut self.renderer {
                    let _ = renderer.recreate_swapchain();
                }
            }

            // 鼠标移入窗口
            WindowEvent::CursorEntered { .. } => {
                if let Some(window) = &self.window {
                    lock_cursor(window);
                }
            }

            // 鼠标移出窗口
            WindowEvent::CursorLeft { .. } => {
                if let Some(window) = &self.window {
                    unlock_cursor(window);
                }
            }

            _ => {}
        }
    }

    /// 处理设备事件（鼠标相对运动等）
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        // 当鼠标处于 Locked 模式时，MouseMotion 提供原始相对位移
        if let DeviceEvent::MouseMotion { delta } = event {
            // 更新相机视角
            self.camera.process_mouse(delta.0 as f32, delta.1 as f32);
        }
    }

    /// 事件队列空闲时调用（主循环体）
    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if !self.running || self.window.is_none() {
            return;
        }

        // 帧率上限（present 节流）：低于单帧预算则补齐
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

        // 更新逻辑（相机、物理等）
        self.update();

        // 渲染当前帧
        self.render();
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
    if let Err(e) = event_loop.run_app(&mut app) {
        log::error!("应用运行错误: {:?}", e);
    }

    log::info!("程序正常退出");
}
