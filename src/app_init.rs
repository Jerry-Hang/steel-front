// 由 src/lib.rs 拆出（2026-11-13，纯移动 + 仅加 pub(crate)）：`GameApp` 的构造。
//
// 444 行的构造函数本来挤在 `lib.rs` 里：渲染器、游戏状态、HUD、联机、诊断计时器全在一处
// 串起来。搬出来之后 `lib.rs` 只留"类型 + 入口"，构造细节在这里读。
use std::time::Instant;

use winit::event_loop::ActiveEventLoop;
use winit::window::CursorGrabMode;

use crate::engine;
use crate::engine::camera::{Camera, KeyState};
use crate::engine::game::Game;
use crate::config;
use crate::engine::renderer::QualityPreset;
use crate::ui::RESOLUTIONS;
use crate::{env_f32, env_truthy, parse_shot_at, GameApp, GunSway};

impl GameApp {
    /// 创建游戏应用实例
    pub(crate) fn new() -> Self {
        let fatal = None;
        // 取证开关：`RV3D_SHOT_AT=5,15,30` ⇒ 进游戏后第 5/15/30 秒各截一张。
        // 不设 = 空列表 = 逐帧那次判断直接短路，玩家路径零开销、逐字节不变。
        let shot_pending = crate::syscfg::cfg("RV3D_SHOT_AT")
            .map(|v| parse_shot_at(&v))
            .unwrap_or_default();
        if !shot_pending.is_empty() {
            log::info!("自动截图已安排（RV3D_SHOT_AT）：{:?} 秒", shot_pending);
        }
        let shot_t0 = None;
        let mut game = Game::new();
        // 加载持久化配置（键位/音量/灵敏度）；文件缺失回退默认，见 config.rs
        let cfg = config::load();
        game.hud.volume = cfg.volume;
        game.hud.music_volume = cfg.music_volume;
        game.hud.sensitivity = cfg.sensitivity;
        game.hud.key_bindings = cfg.bindings;
        // PT 曝光标定值：配置文件 → （可被）RV3D_PT_EXPOSURE 覆盖做 A/B。**它不是玩家选项**，
        // 所以停在配置层与调试开关层，不进设置面板（理由见 ray_tracer.rs 的常量注释）。
        game.hud.pt_exposure = cfg.pt_exposure;
        if let Some(v) = crate::syscfg::cfg("RV3D_PT_EXPOSURE") {
            if let Ok(v) = v.trim().parse::<f32>() {
                game.hud.pt_exposure = v.clamp(
                    engine::ray_tracer::PT_EXPOSURE_MIN,
                    engine::ray_tracer::PT_EXPOSURE_MAX,
                );
                log::info!("PT 曝光被 RV3D_PT_EXPOSURE 覆盖为 {:.3}", game.hud.pt_exposure);
            } else {
                log::warn!("RV3D_PT_EXPOSURE 不是数字，忽略：{v}");
            }
        }
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
            shot_pending,
            shot_t0,
            fatal,
            window: None,
            renderer: None,
            camera: Camera::new(),
            key_state: KeyState::new(),
            #[cfg(target_os = "android")]
            touch_move: None,
            #[cfg(target_os = "android")]
            touch_look: None,
            #[cfg(target_os = "android")]
            touch_fire: None,
            screen_size: (1280.0, 720.0),
            cursor_evt_count: 0,
            cursor_evt_eaten: 0,
            cursor_evt_teleport: 0,
            cursor_evt_last: (0.0, 0.0),
            blank_cursor: None,
            dragging: false,
            right_dragging: false,
            ads_active: false,
            ads_blend: 0.0,
            last_shot_at: -1.0,
            gun_sway: GunSway::new(),
            hit_damage_popups: Vec::new(),
            last_cursor: (0.0, 0.0),
            last_frame: Instant::now(),
            last_cycle_us: 0,
            llm_cap_fps: {
                // 全局帧率上限（2026-08-23 防 GPU 驻停留态 device lost）：
                // RV3D_FPS 覆盖；默认 240；LLM 采集模式 90（留 GPU 余量）
                let llm_on = crate::syscfg::cfg("RV3D_LLM")
                    .map(|v| !(v.is_empty() || v == "0" || v == "off"))
                    .unwrap_or(false);
                // 2026-08-28 实测：128v128 无上限 311fps @99% GPU；默认 300（LLM 120）
                let cap = env_f32("RV3D_FPS").unwrap_or(if llm_on { 120.0 } else { 300.0 });
                cap.max(20.0)
            },
            // 0 = 引擎默认不限制后台帧率（压测/采集路径逐字节不变）；
            // 玩家路径由 SteelFront.bat 设 RV3D_BG_FPS=20（见 there 的注释与 AGENTS 铁律 B）
            bg_cap_fps: env_f32("RV3D_BG_FPS").unwrap_or(0.0).max(0.0),
            force_unfocused: env_truthy("RV3D_FORCE_UNFOCUSED"),
            logged_cap_fps: -1.0,
            last_update_us: 0,
            last_render_us: 0,
            fire_requested: false,
            fire_edge: false,
            cursor_captured: false,
            cursor_grab_logged: 0,
            cursor_locked: false,
            lock_observe: None,
            lock_retried: false,
            lock_fake_warned: false,
            abs_baseline_valid: false,
            // 必须从 false 起步，由 WindowEvent::Focused 驱动。
            // 写成 true 会让 sync_cursor 的 `want` 从第 1 帧就成立：
            // winit 只在收到 WM_SETFOCUS 时才发 Focused(true)，所以窗口若是被
            // 别的程序占着前台（启动瞬间很常见），本进程根本收不到该事件、
            // 也不会收到 Focused(false) —— 于是游戏在"自认为有焦点"的状态下
            // ClipCursor 把指针钉成 1×1，用户看到的就是整台机器像死机。
            focused: false,
            recenter_pending_until: None,
            last_cam_log: Instant::now(),
            game,
            running: true,
            suspended: false,
            event_proxy: None,
            resolution_explicit: cfg.resolution_explicit,
            anim_clock: 0.0,
            last_npc_snapshot: std::collections::HashMap::new(),
            last_fps: 0.0,
            frame_dt_us: 0,
            last_cull_diag: std::time::Instant::now(),
            last_gun_diag: std::time::Instant::now(),
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
                        crate::syscfg::parse::<usize>("RV3D_INSPECT")
                    })
                    .filter(|&n| (1..=35).contains(&n))
            },
            cam_spin: 0.0,
            gun_mesh_uploaded_key: None,
            obstacle_markers_cache: Vec::new(),
            obstacle_markers_gen: None,
            inspect_armed: false,
            cam_logged: false,
            cam_override: crate::syscfg::cfg("RV3D_CAM").and_then(|s| {
                let mut it = s.split(':');
                let _mode = it.next()?; // 模式标记（fly）
                let pos = it.next()?.trim();
                let rot = it.next()?.trim();
                let p: Vec<f32> = pos.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                let r: Vec<f32> = rot.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                // 可选第 4 段 = yaw 角速度（度/秒）。用于运动向量的解析验证：
                // 旋转的屏幕位移与深度无关 ⇒ 全屏均匀、可以用公式算出期望值。
                let spin = it
                    .next()
                    .and_then(|v| v.trim().parse::<f32>().ok())
                    .map(|d| d.to_radians())
                    .unwrap_or(0.0);
                if p.len() == 3 && r.len() == 2 {
                    Some((
                        glam::Vec3::new(p[0], p[1], p[2]),
                        r[0].to_radians(),
                        r[1].to_radians(),
                        spin,
                    ))
                } else {
                    None
                }
            }),
            command_buf: String::new(),
            gun_mesh_cache: None,
            gun_glbs: std::collections::HashMap::new(),
            prop_set: None,
            // 哨兵：保证第一帧就上传一次道具几何
            prop_map_gen: u64::MAX,
            switch_weapon_at: None,
        }
    }

    /// 设置面板鼠标点击：命中某行 → 选中该项（与 Tab 循环一致）；音量/灵敏度条内点击
    /// 按位置比例直接设值（x 比例 = 值）。
    ///
    /// 🔴 几何与行号全部取自 `ui::SettingsLayout` / `ui::settings_row_at`（绘制用的同一份）。
    /// 旧版在这里**又算了一遍**布局、点击循环还写死 `0..7` 行键位，而面板画了 8 行
    /// ⇒ **菜单键那一行鼠标点不中**（键盘 Tab 能到）。注释里那句"布局必须与 ui.rs 一致"
    /// 没能挡住这次漂移，所以改成同源。
    pub(crate) fn settings_click(&mut self, mx: f32, my: f32) {
        let s = self.game.hud.ui_scale();
        let dw = self.game.hud.screen_w / s;
        let dh = self.game.hud.screen_h / s;
        let layout = crate::ui::SettingsLayout::new(dw, dh);
        let mx_d = mx / s;
        let my_d = my / s;
        let Some(row) = crate::ui::settings_row_at(&layout, my_d) else {
            return;
        };
        self.game.hud.settings_selection = row;
        // 滑条行：点在条上按位置比例设值；点标签或行内空白只选中
        let on_bar = mx_d >= layout.bar_left() && mx_d <= layout.bar_left() + layout.bar_w;
        if (row as usize) < crate::ui::SETTINGS_SLIDER_ROWS && on_bar {
            let ratio = ((mx_d - layout.bar_left()) / layout.bar_w).clamp(0.0, 1.0);
            match row {
                0 => self.game.hud.volume = ratio,
                1 => self.game.hud.sensitivity = ratio,
                _ => self.game.hud.music_volume = ratio,
            }
            log::info!("settings: 鼠标点击设定 行{} = {:.0}%", row, ratio * 100.0);
        } else {
            log::info!("settings: 鼠标选中行 {}", row);
        }
    }


    /// 第一人称枪模程序化高模：按当前武器键名从 guns 库取 35 把枪的网格，
    /// 变换到视空间固定位置（view⁻¹ × 锚点 × 倾斜 × 缩放 × 俯角 × 翻转 180°：
    /// guns 库局部坐标枪口朝 +Z，翻转后朝屏幕外 -Z）。
    /// 开火后坐（相位脉冲）+ 行走晃动 + 腰射右倾/开镜扶正。
    /// 导入枪模（按武器 key 自动寻找 assets/guns/{key}.glb；不存在回退 ak12.glb）
    /// 2026-08-28 终局：使用原始模型材质本色（baseColorFactor 直出 × 忠实现光）
    /// 🪖 加载士兵 GLB（2026-09-13）。路径固定为 `assets/soldier/soldier.glb`。
    ///
    /// **刻意不做任何归一化**：`soldier.glb` 是按铁律 D 的约定生成的
    /// （1 单位 = 1 米、原点在底面中心、`export_yup=True`、单 mesh 无变换），
    /// 本来就是引擎要的尺度与朝向。枪模那套"缩放到 0.94m + 几何居中 + 长轴对齐 +Z"
    /// 是针对 Sketchfab 抠件的，**不要照抄到这里**。
    ///
    /// 返回 `None` 时（文件缺失/解析失败）渲染器保持 `soldier_vertex_count == 0`，
    /// 于是 NPC **继续用原来的 18 段箱体** —— 与改动前逐字节一致，不会退化。


    /// 武器键 → `assets/guns/` 里的文件名（不含扩展名）。
    ///
    /// 🔴 只有 `ak12m` 与文件名不同：键叫 `ak12m`，资产叫 `ak12.glb`。2026-10-31 就因为
    /// 少了这张表，`exists("assets/guns/ak12m.glb")` 恒为 false ⇒ **主力枪掉到程序化模型**，
    /// 日志里还写着"ak12m 无 GLB"（看起来像资产丢了）。判据
    /// `gun_key_maps_to_the_shipped_glb_name` 钉住它。


    /// 行走摆动的幅值包络（0..1）：`smoothstep(平滑速度) × 开火阻尼 × RV3D_GUN_SWAY 增益`。
    ///
    /// 单独抽出来是因为它同时被 `fp_gun_matrix` 与 `RV3D_GUN_DIAG` 的诊断行用 ——
    /// 诊断必须量**真正参与渲染的那个数**，不能另写一套近似（否则尺子和被测物不是一回事）。

    /// ESC 菜单鼠标点击命中检测：命中选项矩形则执行对应动作（0=退出 1=设置）。
    /// 矩形布局必须与 ui.rs `esc_menu_elements` 一致（面板 380x240 居中，
    /// 选项 y = py+90 / py+146，宽 pw-120=260 居中，高 34）。返回是否命中任何选项。
    pub(crate) fn menu_click_hit(&mut self, mx: f32, my: f32) -> bool {
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

    /// 把 WASD 按键状态转发给游戏（FPS 玩家移动）
    /// ESC 的语义（2026-10-11 从 `KeyboardInput` 分支里抽出）：
    /// 设置面板打开 → 关面板；ESC 菜单打开 → 关菜单；否则 → 打开 ESC 菜单并**立即**释放鼠标捕获。
    ///
    /// 抽出来是为了让 **Android 返回键**复用同一条路径 —— 手机上返回键必须等价于 ESC，
    /// 否则玩家退不出游戏（见下面 `NativeKeyCode::Android(4)` 那条 arm）。
    /// ⚠️ 「立即释放捕获」那段不能省：等下一帧 `sync_cursor` 的话，用户立刻移动点击时
    /// `last_cursor` 仍是捕获中心 ⇒ 菜单选项命中错位。
    pub(crate) fn toggle_escape_menu(&mut self) {
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
            if self.cursor_captured {
                self.release_cursor_immediately("ESC menu opened");
            }
        }
    }

    /// 🔴 **立即释放光标捕获**（唯一实现；2026-10-20 第 14 批复核第 3 条）。
    ///
    /// 为什么必须"唯一"：捕获/释放的状态位有 5 个（`cursor_captured` / `cursor_locked` /
    /// `abs_baseline_valid` / `recenter_pending_until` / `lock_observe`）+ 相机旋转标志，
    /// 之前两个释放点各写一半，漏一个就是"菜单里鼠标被锁死"或"视角仍在转"。
    ///
    /// 另一个关键点：Windows 路径捕获时用的是**1×1 全透明自定义光标**
    /// （见 `sync_cursor` 里的长说明），释放时**必须恢复 `CursorIcon::Default`** ——
    /// 原来只 `set_cursor_visible(true)`，玩家进过游戏再按 ESC，指针虽然能动但**完全隐形**。
    pub(crate) fn release_cursor_immediately(&mut self, reason: &str) {
        if let Some(window) = &self.window {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
            window.set_cursor(winit::window::CursorIcon::Default);
        }
        self.cursor_grab_logged = 0;
        self.cursor_captured = false;
        self.cursor_locked = false;
        self.abs_baseline_valid = false;
        self.recenter_pending_until = None;
        self.lock_observe = None;
        self.camera.set_rotation_active(false);
        log::info!("input: cursor released ({reason})");
    }

    /// 每局重开/切关时复位 **GameApp 层**的瞬时状态。
    ///
    /// 🔴 2026-10-20（第 14 批复核第 2 条）：`Game::start_run` 只能复位 `Game` 自己的字段，
    /// 而按键锁存、开火意图、命令框、尸体快照、伤害飘字、枪摆动都活在 `GameApp` 上。
    /// 重开后它们带着上一局的残留：
    /// * `key_state.forward` 仍为 true ⇒ 下一次按任意移动键时 `sync_game_movement`
    ///   会把残留的前进重新同步给游戏（"重开后自己往前跑"）；
    /// * `fire_requested` 残留 ⇒ 重开首帧自动开一枪；
    /// * `corpses` / `hit_damage_popups` / `last_npc_snapshot` ⇒ 上一局的尸体与伤害数字
    ///   留在新一局（快照不清理还会把旧 NPC 当"刚死"再补一批尸体）。
    pub(crate) fn reset_session_transients(&mut self) {
        self.key_state.reset();
        self.fire_requested = false;
        self.ads_active = false;
        self.command_open = false;
        self.command_buf.clear();
        self.corpses.clear();
        self.last_npc_snapshot.clear();
        self.hit_damage_popups.clear();
        self.gun_sway.reset_for_new_run();
        self.game.set_movement(false, false, false, false);
    }

    pub(crate) fn sync_game_movement(&mut self) {
        let k = &self.key_state;
        self.game.set_movement(k.forward, k.backward, k.left, k.right);
    }

    /// **退出前的统一收尾 —— 所有退出路径共用这一个。**
    ///
    /// 🔴 存在理由（2026-10-15，外部复核指出第 4 条，已对着代码确认）：
    /// 原来**三个出口各做一半**，而且缺的还不是同一样
    /// （典型的"多出口不同构"）：
    ///
    /// | 出口 | `send_leave()` | `perf_log.finish()` |
    /// |---|---|---|
    /// | ① ESC 菜单鼠标点击退出（经 `user_event`）| ❌ 缺 | ✅ |
    /// | ② 窗口关闭按钮（`CloseRequested`）| ✅ | ❌ 缺 |
    /// | ③ ESC 菜单键盘 `Enter` 确认退出 | ❌ 缺 | ❌ 缺 |
    ///
    /// 各自的代价：
    /// * 缺 `send_leave` ⇒ 服务端要等 `SERVER_TIMEOUT`(5s) 才摘掉我们，
    ///   而这 5 秒里**每个客户端都还看得见我们站在原地**；
    /// * 缺 `perf_log.finish()` ⇒ 性能日志**最后一段没落盘**（而退出前那段往往正是要看的）。
    ///
    /// `send_leave` 在单机/服务端模式下是空操作，重复调用也无害
    /// （服务端收到第一条 `Leave` 就摘人），所以这里可以无条件调。
    pub(crate) fn begin_exit(&mut self, event_loop: &ActiveEventLoop) {
        self.game.send_leave();
        if let Some(pl) = self.perf_log.as_mut() {
            pl.finish();
            self.perf_log = None;
        }
        self.running = false;
        event_loop.exit();
    }

    /// 把当前分辨率应用到窗口（尺寸相同则跳过；`Resized` 事件会触发渲染器重建交换链）
    pub(crate) fn apply_resolution(&self) {
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
    pub(crate) fn apply_quality(&mut self) {
        // RV3D_QUALITY=0/1/2 覆盖面板画质（A/B / 无窗口取证要能强制低档或高档）。
        let quality_index =
            crate::syscfg::parse::<usize>("RV3D_QUALITY").unwrap_or(self.game.hud.quality_index as usize);
        let preset = match quality_index {
            0 => QualityPreset::Low,
            1 => QualityPreset::Medium,
            _ => QualityPreset::High,
        };
        log::info!("settings: 应用画质 {}", preset.label());
        // 2026-08-28：枪实例矩阵预计算（进入 renderer 借用前——防借用冲突 + 每帧一次）
        if let Some(renderer) = &mut self.renderer {
            renderer.set_quality(preset);
        }
    }

    /// F12 截图：调渲染器把当前帧保存到 <平台截图目录>/steel_front_<秒时间戳>.png
    /// （Windows = 当前目录 screenshots/；Linux 原生写 /tmp —— 本条原写作"保持 WSL2 行为"，
    ///   而 WSL2 材料已作废（AGENTS.md 环境铁律），落盘位置本身不变。）
    pub(crate) fn capture_screenshot(&mut self) {
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
}
