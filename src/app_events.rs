// 由 src/lib.rs 拆出（2026-11-12）：`ApplicationHandler::resumed` 的实现体。
//
// 为什么只搬"体"不搬回调本身：`impl ApplicationHandler for GameApp` 一个类型只能有一份，
// 回调必须留在原处。把 280 行的实现体抽成 `GameApp` 的普通方法后，回调只剩一行转发，
// trait 块就短了，而逻辑仍然只在一个地方。
use std::time::{Duration, Instant};

use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window};

use crate::engine::camera::CameraMode;
use crate::engine::game::GameState;
use crate::engine::renderer::Renderer;
use crate::ui::{BindingAction, KeyBindings, RESOLUTIONS};
use crate::{
    perf_log, window, window_physical_request, GameApp, MAX_LOOK_DELTA_PX,
};

impl GameApp {
    /// 应用恢复/启动：建窗口与渲染器；Android 上还要从 surface 丢失里恢复。
    pub(crate) fn handle_resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            // Android：再次 resumed（surface 回来）⇒ 恢复渲染并重建交换链。
            // （桌面只在启动时进这里一次，且 suspended 恒 false，行为不变。）
            if self.suspended {
                log::info!("生命周期：Resumed（surface 可用），恢复渲染");
                self.suspended = false;
                // 两个字段的不相交借用（同 `render()` 里 `&mut self.renderer` + `&self.window` 的写法）
                if let (Some(renderer), Some(window)) = (&mut self.renderer, &self.window) {
                    if renderer.swapchain_recovery_allowed() {
                        // 🔴 2026-10-13：**失败要升级成重建 surface**，不能只记一条 warn。
                        // 实测（真机日志）：Android 切后台时系统回收原生窗口 ⇒
                        // `vkGetPhysicalDeviceSurfaceCapabilitiesKHR` 报 `ERROR_SURFACE_LOST_KHR`
                        // ⇒ 拿着死 surface 重建交换链**永远失败**，而"等后续 Resized"
                        // 在尺寸不变时**永远等不到** ⇒ 永久黑屏（用户报的那个 bug）。
                        let r = renderer
                            .recreate_swapchain()
                            .or_else(|e| {
                                log::warn!("Resumed 后重建交换链失败（{}）⇒ 升级为重建 Vulkan 表面", e);
                                renderer.recreate_surface(window)
                            });
                        if let Err(e) = r {
                            log::error!("Resumed 后连表面也重建失败（等后续事件重试）：{}", e);
                        }
                    }
                }
            }
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
        // 夹进显示器物理尺寸（防超屏 → 内容只显示左上角），并**保持物理像素语义**
        // —— 这里绝不能再出现 `/1.5` 那类按某台机器标定出来的缩放系数，理由见
        // `window_physical_request` 的文档（判据 `window_request_is_physical_and_clamped`）。
        // Wayland 下 `primary_monitor()` 恒为 None（没有主显示器概念）⇒ 不夹取；
        // 这不是缺陷，是这个后端的语义（同文件 `resumed` 上面的默认分辨率选择已同样处理）。
        let monitor_size = event_loop.primary_monitor().map(|m| {
            let s = m.size();
            (s.width, s.height)
        });
        let (w, h) = window_physical_request(w, h, monitor_size);
        if (w, h) != self.game.hud.resolution() {
            log::warn!(
                "请求分辨率 {}x{} 超过显示器 {:?}，已等比缩到 {}x{}",
                self.game.hud.resolution().0,
                self.game.hud.resolution().1,
                monitor_size,
                w,
                h
            );
        }
        // 无边框窗口：请求分辨率等于显示器物理尺寸时窗口恰好铺满屏幕，无标题栏/边框挤压
        // （否则窗口比屏幕略大 → 合成器裁切 → 内容偏左上角）。
        // 🔴 尺寸一律 `PhysicalSize`：`w`/`h` 与 `monitor.size()` **都是物理像素**，
        // 用 `LogicalSize` 等于让 winit 再乘一次 scale_factor（Windows 那台 scale=1.5 时
        // 就是靠手写的 `/1.5` 抵消的 —— 换个平台必然错）。
        // 自动化取证模式：窗口**从创建起就不可见**（`RV3D_HIDDEN=1`）。
        //
        // 动机（2026-10-22 用户要求）：无人值守截图每跑一次就把游戏窗口弹到最上层、
        // 抢焦点，打扰正在用机器的人。`with_visible(false)` 创建即隐藏，不抢焦点、不遮挡屏幕；
        // Vulkan 仍正常渲染（截帧读的是交换链图像，不是屏幕）。输入/计时/渲染路径不变。
        // 无头离屏（`RV3D_HEADLESS=1`）也把窗口隐藏：该模式根本不创建交换链/present，
        // 窗口只是 Vulkan surface 的载体，从不 attach buffer ⇒ 不会上屏。
        let headless = crate::engine::renderer::headless_mode();
        let hidden = crate::syscfg::is_set("RV3D_HIDDEN") || headless;
        if headless {
            log::info!("RV3D_HEADLESS=1：无窗口离屏取证（不创建交换链、不呈现，绝不弹出窗口）");
        } else if hidden {
            log::info!("RV3D_HIDDEN=1：窗口隐藏创建（无人值守取证，不抢焦点）");
        }
        let winit_attr = Window::default_attributes()
            .with_title(window::WINDOW_TITLE)
            .with_inner_size(winit::dpi::PhysicalSize::new(w, h))
            .with_position(winit::dpi::PhysicalPosition::new(0, 0))
            .with_decorations(false)
            .with_visible(!hidden);

        let window = match event_loop.create_window(winit_attr) {
            Ok(w) => w,
            Err(e) => {
                let msg = format!("创建窗口失败: {e:?}");
                log::error!("{msg}");
                self.fatal = Some(msg);
                event_loop.exit();
                return;
            }
        };

        log::info!("窗口创建成功: {}x{}", w, h);

        // 🔴 2026-09-13：造一个 **1×1 全透明光标**，捕获时用它。
        // 为什么需要它（两个约束互相冲突，这是唯一的交集）：
        //   * winit 若认为光标"已隐藏"（`CursorFlags::HIDDEN`），在 Confined 模式下会把指针
        //     **钉在窗口中心 1×1**（`window_state.rs::refresh_os_cursor`）⇒ 我们靠
        //     `CursorMoved` 增量算视角，`dx` 恒 0 ⇒ **视角纹丝不动**（用户实测）。
        //   * 但把光标设成可见，屏幕上就**一直浮着一个箭头**（用户 2026-09-13 反馈）。
        // ⇒ winit **只看自己的标志位，从不看光标图像**。所以"不隐藏 + 图像全透明"两边都满足。
        // `CustomCursor` 必须由事件循环创建（`create_custom_cursor`），所以在这里做一次。
        self.blank_cursor =
            match winit::window::CustomCursor::from_rgba(vec![0u8, 0, 0, 0], 1, 1, 0, 0) {
            Ok(src) => Some(event_loop.create_custom_cursor(src)),
            Err(e) => {
                log::warn!("无法创建透明光标（{e:?}），捕获时会显示系统箭头");
                None
            }
        };
        log::info!(
            "winit inner_size: {}x{} scale_factor={:.2}",
            window.inner_size().width,
            window.inner_size().height,
            window.scale_factor()
        );

        // ---- 初始化 Vulkan 渲染器 ----
        match Renderer::new(&window) {
            Ok(mut renderer) => {
                log::info!("Vulkan 渲染器初始化成功");
                // 🪖 士兵 GLB（2026-09-13）：**在这里上传，只做一次**。
                //
                // 🔴 为什么不放在每帧的 `render()` 里（我放错过两次）：
                // 那个调用点在 `MainWindow::render()` 的某个状态分支内，**并不是每帧都执行** ——
                // 用 `RV3D_NPC_CAM` 起调试机位时就走不到，于是 `soldier_vertex_count` 恒为 0，
                // 而 `set_npc_visuals` / `set_dead_bodies` 都靠它判断走 GLB 还是回退箱体
                // ⇒ **整个画面悄悄退回 18 段箱体**（实测 HUD `npc:` 从 17 跳回 1255），
                // 而且不报任何错。**放在这里就结构上不可能被跳过。**
                //
                // 与枪模同源的做法：启动时上传一次，之后 `set_soldier_mesh` 的幂等守卫保证
                // 再调用也无害（它现在只会在 `soldier_vertex_count == 0` 时才真的分配）。
                //
                // ⚠️ 顶点**不做任何归一化**：`soldier.glb` 是按铁律 D 的约定导出的
                // （1 单位 = 1 米、原点在底面中心、`export_yup=True`），本来就是引擎要的尺度与朝向；
                // 枪模那套"缩放到 0.94m + 居中 + 长轴对齐 +Z"是针对 Sketchfab 抠件的，**不要照抄**。
                if let Some((sv, si)) = Self::load_soldier_glb() {
                    renderer.set_soldier_mesh(&sv, &si);
                }
                // ---- RT core 纯求交吞吐基准（RV3D_PT_BENCH=1）----
                if crate::syscfg::cfg("RV3D_PT_BENCH").as_deref() == Some("1") {
                    let boxes = vec![
                        crate::engine::ray_tracer::PtBox { center: [0.0, -0.5, 0.0], half: [50.0, 0.5, 50.0], material: 0 },
                        crate::engine::ray_tracer::PtBox { center: [1.0, 1.0, 0.0], half: [2.0, 2.0, 1.0], material: 1 },
                        crate::engine::ray_tracer::PtBox { center: [-4.0, 1.5, -2.0], half: [1.5, 1.5, 1.5], material: 2 },
                        crate::engine::ray_tracer::PtBox { center: [0.5, 1.0, 5.0], half: [0.8, 0.8, 0.8], material: 3 },
                    ];
                    match renderer.run_pt_bench(&boxes, 1 << 20, 200) {
                        Ok((mrays, hits)) => log::info!(
                            "RT-BENCH: 1M射线 x 200 = 2亿射线, 命中 {hits}, {mrays:.1} Mrays/s"
                        ),
                        Err(e) => log::error!("RT-BENCH 失败: {e}"),
                    }
                }
                // PT 参考帧（RV3D_PT_VIEW=1）
                if crate::syscfg::cfg("RV3D_PT_VIEW").as_deref() == Some("1") {
                    let boxes = vec![
                        crate::engine::ray_tracer::PtBox { center: [0.0, -0.5, 0.0], half: [8.0, 0.5, 8.0], material: 0 },
                        crate::engine::ray_tracer::PtBox { center: [0.0, 0.5, -2.0], half: [0.8, 0.8, 0.8], material: 1 },
                        crate::engine::ray_tracer::PtBox { center: [-2.5, 0.7, 1.0], half: [0.5, 0.7, 0.5], material: 2 },
                        crate::engine::ray_tracer::PtBox { center: [2.5, 0.5, 1.5], half: [0.6, 0.4, 0.6], material: 3 },
                    ];
                    // 取景：相机在 +Z 侧看向原点，与上面的玩具盒场景（地面 8m 见方）对得上
                    renderer.set_pt_params(crate::engine::ray_tracer::PtParams {
                        cam: glam::Vec3::new(0.0, 1.7, 6.5),
                        fwd: glam::Vec3::new(0.0, -0.18, -0.98),
                        tan_half_fov: (60f32.to_radians() * 0.5).tan(),
                        bounces: 6,
                        sun_dir: crate::engine::ray_tracer::PT_SUN_DIR.into(),
                        sun_color: glam::Vec3::splat(1.0) * crate::engine::ray_tracer::PT_SUN_INTENSITY,
                        exposure: 0.5,
                    });
                    match renderer.run_pt_view(&boxes, 256) {
                        Ok(()) => log::info!("PT-VIEW: 参考帧已输出 screenshots/pt_ref.bmp (256x256)"),
                        Err(e) => log::error!("PT-VIEW 失败: {e}"),
                    }
                }
                // 2026-08-29：路径追踪全景开关（设置面板 pt_enable；默认开）
            let mut renderer = renderer;
            // RV3D_PT_LIVE: 0=强制关 1=强制开 未设=跟随配置
            // 🔴 常驻资源与这个开关**同源**（判据 = `pt_resident_needed` 的两个单测）：
            // 以前 `RV3D_PT_LIVE=1` 只改 `pt_live_enabled`，常驻资源却只在配置开着时构建，
            // 而 PT 出画还要求 `pt_resident.is_some()` ⇒ 那个"强制开"实际**一帧都跑不出来**，
            // 外面只看到"开着、画面没变"（2026-09-25 的 PT 验证就是这么空跑一次）。
            let pt_cfg = crate::config::load().pt_enable;
            let pt_live_env = crate::syscfg::cfg("RV3D_PT_LIVE");
            renderer.pt_live_enabled = match pt_live_env.as_deref() {
                Some("1") => true,
                Some("0") => false,
                _ => pt_cfg,
            };
            if crate::engine::renderer::pt_resident_needed(pt_cfg, pt_live_env.as_deref()) {
                // RV3D_PT_SIZE：实时 PT 渲染分辨率（**等比**缩放到该宽度；调高可验证 RT 通路真在算，
                // 也可为光照烘焙取更高分辨率参照帧）
                // 未设时对齐窗口物理尺寸（2560×1600！）；两者都对齐 8 的倍数
                // （判据 = `engine::renderer::pt_render_extent` 的单测）
                let win_sz = window.inner_size();
                let size_env = crate::syscfg::parse::<u32>("RV3D_PT_SIZE");
                let (pt_w, pt_h) =
                    crate::engine::renderer::pt_render_extent(win_sz.width, win_sz.height, size_env);
                if let Err(e) = renderer.init_pt_resident(pt_w, pt_h) {
                    log::info!("PT-RESIDENT init: {e}");
                }
            }
            log::info!("RT: 路径追踪全景 = {}", if renderer.pt_live_enabled { "开启" } else { "关闭" });
            self.renderer = Some(renderer);
            }
            Err(e) => {
                let msg = format!("渲染器初始化失败: {e}");
                log::error!("{msg}");
                self.fatal = Some(msg);
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

    /// `ApplicationHandler::window_event` 的实现体（纯移动）。
    ///
    /// 回调本身留在 `lib.rs`（trait impl 一个类型只能有一份），这里只做转发。
    pub(crate) fn handle_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        match event {
            // 关闭窗口请求
            WindowEvent::CloseRequested => {
                log::info!("窗口关闭请求，退出程序");
                // 🔴 走统一收尾（原来这里只 `send_leave()`，**漏了 `perf_log.finish()`**
                // ⇒ 性能日志最后一段没落盘）
                // 2026-09-26 加的 `send_leave` 理由：不发的话服务端要等
                // SERVER_TIMEOUT(5s) 才摘掉我们，而这 5 秒里**每个客户端都还看得见我们
                // 站在原地**（远端玩家实体无条件进画面）。UDP 同步发送，退出前能发出去。
                self.begin_exit(event_loop);
            }

            // 🔴 Android 返回键（2026-10-11 实测 + 修）：**它永远匹配不到下面那条 arm**。
            //
            // winit 0.30 把返回键投成 `PhysicalKey::Unidentified(NativeKeyCode::Android(4))`
            // —— 物理键白名单里没有 `Back`（见 winit `keycodes.rs` 的 fallback），
            // 而下面那条 pattern 写死是 `PhysicalKey::Code(key_code)` ⇒ 返回键掉进 `_ => {}`
            // **被静默丢弃**。更糟的是 winit 已把它标成 `InputStatus::Handled`
            // ⇒ `android-activity` 调 `finishEvent(handled=true)` ⇒ **系统也不做默认 finish()**。
            //
            // ⇒ 后果：**玩家在手机上退不出游戏**（也打不开设置、打不开菜单）。
            // 这正是铁律 C 那条原则 ——「**别让游戏夺走用户对设备的控制**」——
            // 在手机上的对偶问题：桌面上是别锁死鼠标，手机上就是**别锁死返回键**。
            //
            // ⇒ 对策：接到与 ESC **完全相同**的路径（`toggle_escape_menu`），
            //    语义完全一致，不新增分支行为。退出本身仍走 ESC 菜单里的「退出游戏」，
            //    所以**不会出现"按一下返回键就闪退"**（那也会是另一种"夺走控制"）。
            #[cfg(target_os = "android")]
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Unidentified(
                            winit::keyboard::NativeKeyCode::Android(4),
                        ),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                log::info!("input: Android 返回键 → 按 ESC 语义处理");
                self.toggle_escape_menu();
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
                    self.game.on_any_key();
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
                    self.toggle_escape_menu();
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
                            // 🔴 走统一收尾（原来这条路径**两个收尾都没做**）
                            self.begin_exit(event_loop);
                        } else {
                            log::info!("ESC 菜单：打开设置");
                            self.game.hud.esc_menu_open = false;
                            self.game.toggle_settings();
                        }
                        return;
                    }
                    if pressed && key_code != KeyCode::Escape {
                        // 🔴 **必须 return**（2026-10-20 第 14 批复核第 7 条）：
                        // 关闭菜单后 `in_gameplay` 立刻变 true，同一按键会继续往下穿透 ——
                        // 按 R 关菜单会同时"在结算界面重开一局"、按 X 关菜单会**吃掉一个医疗包**、
                        // 按 G 会当场扔手雷。
                        self.game.hud.esc_menu_open = false;
                        return;
                    }
                }

                // 姿态与冲刺（2026-09-12）。这三个键**暂未并入 `BindingAction` 可重绑定表**：
                // 并入要同步 ui.rs 的枚举 + 默认表 + getter/label/slot 四处 match + 键表测试，
                // 先按固定键把功能落地；重绑定与 HUD 提示见 docs/PROGRESS.md 待办。
                // Shift 是按住类（冲刺条件是"按住 + 前进 + 站立 + 未开镜 + 在地面"），
                // C/Z 是按下即切换（松开不改变姿态）。
                //
                // 🔴 **必须带玩法态门控**（2026-10-15，外部复核指出第 4 条，已确认）：
                // 这几条原来**一个门控都没有** ⇒ 在**设置面板打开**时按 X
                // 会直接**消耗一个医疗包**（`use_medkit` 真的扣道具），
                // 按 C / Z 会切蹲/趴，而玩家眼前只有设置面板、完全看不到发生了什么。
                // ⇒ 与 `/` 那条同一个形态：**改状态的按键漏了"当前在哪个界面"的判断**。
                let in_gameplay = self.game.state() == GameState::Playing
                    && !self.game.settings_open()
                    && !self.game.hud.esc_menu_open;
                match key_code {
                    KeyCode::ShiftLeft | KeyCode::ShiftRight if in_gameplay => {
                        self.game.set_sprint(pressed)
                    }
                    KeyCode::KeyC if pressed && in_gameplay => self.game.toggle_crouch(),
                    KeyCode::KeyZ if pressed && in_gameplay => self.game.toggle_prone(),
                    // X = 打药（WX 里 X 的 VK 是 88）
                    KeyCode::KeyX if pressed && in_gameplay => self.game.use_medkit(),
                    _ => {}
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
                                    self.game.request_restart();
                                    self.reset_session_transients();
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
                                // 🔴 **立即释放捕获**（2026-10-20 第 14 批复核第 3 条）：
                                // 等下一帧 `sync_cursor` 的话，同一批事件里紧跟的
                                // `CursorMoved`/鼠标点击仍按"捕获中心"消费 ⇒ 设置面板点不中。
                                if self.cursor_captured {
                                    self.release_cursor_immediately("settings opened (Menu key)");
                                }
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
                    // 数字键 1..9：切换到对应武器槽位（0..8）。
                    //
                    // 🔴🔴 2026-09-13 修：原先**只有 `Digit1` / `Digit2` 两个分支**，
                    // 而它们的注释还停留在「M1 Rifle / Thompson SMG」—— 那是**二战时代的
                    // 遗留**（本作早已是现代装备，武器表也远不止两把）。这段代码从未随
                    // 武器表增长而更新，于是：
                    //   * 按 3/4/5/… **完全没有分支**，什么都不发生
                    //   * 按 1/2 也只能到前两个槽位
                    // 用户 2026-09-13 实测「输入数字用指令切枪的时候，枪的模型没有变化」，
                    // 我用 `scripts/probe_weapons.ps1` 复现：按 1 后画面与基准**逐像素相同**
                    // （`tools/diff_gun_region.py` 报 0.00% 差异）。⚠️ 那个脚本 2026-09-26 已删除
                    // —— 它会 `SetForegroundWindow` **抢焦点**（违反鼠标安全协议），
                    // 由 `scripts/run_weapon_probe.ps1` 取代（PostMessage + 验证层 + 两条切枪路都计数）。
                    //
                    // 槽位越界由 `WeaponSystem::switch_weapon` 自己忽略，这里不必再判。
                    KeyCode::Digit1
                    | KeyCode::Digit2
                    | KeyCode::Digit3
                    | KeyCode::Digit4
                    | KeyCode::Digit5
                    | KeyCode::Digit6
                    | KeyCode::Digit7
                    | KeyCode::Digit8
                    | KeyCode::Digit9 => {
                        if pressed
                            && self.game.state() == GameState::Playing
                            && !self.game.settings_open()
                        {
                            let slot = match key_code {
                                KeyCode::Digit1 => 0,
                                KeyCode::Digit2 => 1,
                                KeyCode::Digit3 => 2,
                                KeyCode::Digit4 => 3,
                                KeyCode::Digit5 => 4,
                                KeyCode::Digit6 => 5,
                                KeyCode::Digit7 => 6,
                                KeyCode::Digit8 => 7,
                                _ => 8,
                            };
                            self.game.switch_weapon(slot);
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
                    // F：手电开关（电量耗尽时拒绝打开）
                    KeyCode::KeyF => {
                        if pressed {
                            self.game.toggle_flashlight();
                        }
                    }
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
                            // 🔴 **不能漏 `settings_open`**（2026-10-15，外部复核指出第 4 条，
                            // 已确认）：上面那条分支只处理"设置面板打开时的 **Enter**"，
                            // 于是设置面板打开时按 `/` 会掉进这里 ⇒ **在设置面板背后打开命令窗口**，
                            // 而命令窗口会吃掉后续所有按键（含 `Esc`）⇒ 设置面板再也关不掉。
                            && !self.game.settings_open()
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
                                self.game.request_restart();
                                self.reset_session_transients();
                            }
                        }
                    }
                    // 设置面板调试补给（N 键补满弹匣）；胜利结算 N 键进入下一关
                    KeyCode::KeyN => {
                        if pressed && self.game.settings_open() {
                            log::info!("settings: N 键补给弹药");
                            self.game.give_ammo();
                        } else if pressed && matches!(self.game.state(), GameState::Victory(_)) {
                            if self.game.advance_level() {
                                self.reset_session_transients();
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
                                self.game.on_any_key();
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
                // 🔴 2026-09-13 诊断计数：回答"鼠标移动事件到底有没有到"。
                // 用户实测"鼠标完全转不了视角"时，必须先分清是
                //   ① 事件根本没来（winit/窗口/焦点层）
                //   ② 事件来了但被某条守卫丢掉（recenter / teleport / dragging=false）
                // 这两个原因的修法完全不同，靠推理分不开（教训 20）。
                self.cursor_evt_count = self.cursor_evt_count.wrapping_add(1);
                self.cursor_evt_last = (px, py);
                // warp 回声事件吞噬窗口：recenter 后短时间内的下一个 CursorMoved
                // 只是回中回声，把它作为新基准并跳过，防止回声环把落点偏移当视角位移
                if let Some(until) = self.recenter_pending_until {
                    self.recenter_pending_until = None;
                    if Instant::now() < until {
                        self.last_cursor = (px, py);
                        self.cursor_evt_eaten = self.cursor_evt_eaten.wrapping_add(1);
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
                    if teleported {
                        self.cursor_evt_teleport = self.cursor_evt_teleport.wrapping_add(1);
                    }
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
                            // 🔴 2026-09-13 修：**必须看返回值**。原写法
                            // `let _ = set_cursor_position(center); last_cursor = center;`
                            // 无条件把基准设成中心 —— 而窗口没焦点时这个 warp 会失败，
                            // 真实指针并不在中心，于是下一个事件算出的 dx 是一个大跳变，
                            // 被 MAX_LOOK_DELTA_PX 当"光标传送"**全部丢弃** ⇒
                            // 表现就是"按住左键拖拽也转不了视角"（用户 2026-09-13 实测）。
                            // 捕获态那条路早已修过同一个 bug（见上面 3036 行的注释），
                            // 这条漏了。
                            if window.set_cursor_position(center).is_ok() {
                                self.last_cursor = (center.x, center.y);
                                self.recenter_pending_until =
                                    Some(Instant::now() + Duration::from_millis(150));
                            } else {
                                // warp 失败：基准留在真实位置，下一事件才能算出正确增量
                                self.last_cursor = (px, py);
                            }
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
            // Android 触控：左半屏方向杆 / 右半屏视角 / 右下角开火。
            #[cfg(target_os = "android")]
            WindowEvent::Touch(t) => {
                self.handle_touch(t);
            }
            WindowEvent::Resized(new_size) => {
                if new_size.width == 0 || new_size.height == 0 {
                    return; // 窗口最小化
                }
                log::info!("窗口大小变化: {}x{}", new_size.width, new_size.height);
                self.screen_size = (new_size.width as f32, new_size.height as f32);
                self.game
                    .hud
                    .set_screen_size(new_size.width as f32, new_size.height as f32);
                if let Some(renderer) = &mut self.renderer {
                    // 🔴 必须**先**把新尺寸告诉渲染器再重建：Wayland 下
                    // `surface.currentExtent` 是未定义的，交换链尺寸只能来自这个字段
                    // （见 `Renderer::window_extent` / `swapchain_extent_choice`）。
                    // 顺序反了 = 重建出来的还是旧尺寸，而且**不报错**。
                    renderer.set_window_extent(new_size.width, new_size.height);
                    // 降级/设备丢失时不重试（判据 = `swapchain_recovery_allowed`）
                    if renderer.swapchain_recovery_allowed() {
                        if let Err(e) = renderer.recreate_swapchain() {
                            log::error!("窗口尺寸变化后重建交换链失败：{}", e);
                        }
                    }
                }
            }

            _ => {}
        }
    }
}
