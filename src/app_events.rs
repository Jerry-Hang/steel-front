// 由 src/lib.rs 拆出（2026-11-12）：`ApplicationHandler::resumed` 的实现体。
//
// 为什么只搬"体"不搬回调本身：`impl ApplicationHandler for GameApp` 一个类型只能有一份，
// 回调必须留在原处。把 280 行的实现体抽成 `GameApp` 的普通方法后，回调只剩一行转发，
// trait 块就短了，而逻辑仍然只在一个地方。
use std::time::Instant;

use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::ui::RESOLUTIONS;
use crate::engine::renderer::Renderer;
use crate::{perf_log, window};
use crate::{window_physical_request, GameApp};

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
}
