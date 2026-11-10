// 由 src/lib.rs 拆出（2026-11-10，纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
//
// 这里放**与窗口/焦点/截图/帧率上限有关**的一组纯函数：它们只依赖 winit 与 crate 内的
// 几个类型，不碰渲染器、不碰游戏状态 ⇒ 搬出来不改变任何行为，却让 lib.rs 少 300 余行。
//
// `lib.rs` 里用 `pub(crate) use app_window::*;` 把名字再导出，调用点一个字都不用改。

/// 这不是可选优化，而是捕获态**唯一**的相对视角来源：`window_event` 的
/// `CursorMoved` 分支在 `cursor_locked` 时直接 `return`（"raw 增量已驱动视角，
/// 绝对位置只更新基准"），而 `device_event` 的 `MouseMotion` 分支只在
/// `cursor_locked` 时生效 —— 两条路径互斥且各自是对方的唯一出口。
/// 于是 `Locked` 一旦成功，绝对位置路径就被关掉；若该平台又从不投递
/// `MouseMotion`，视角就完全无输入，且编译期与运行期都不报错。
///
/// winit 0.30 的 Windows 后端只构造 `DeviceEvent::Added` / `Removed`
/// （`platform_impl/windows/` 下没有 `MouseMotion` 的构造点），该事件目前
/// 只由 X11 / Wayland / macOS / web 后端发出。引入本分支的 `5373a08` 正是为
/// XInput2（X11）写的；2026-08-15 迁到 Windows 原生后该前提不再成立，
/// 但这里没跟着改，于是在 Windows 上 `Locked` 成功 = 视角失效。
use std::time::{Duration, Instant};

use winit::window::CursorGrabMode;

use crate::engine::game::GameState;
use crate::GameApp;

#[cfg(target_os = "windows")]
pub(crate) const RAW_MOUSE_MOTION: bool = false;
#[cfg(not(target_os = "windows"))]
pub(crate) const RAW_MOUSE_MOTION: bool = true;

/// 🔴🔴 2026-09-13：**直接问操作系统**"我们的进程是不是前台"。
///
/// 为什么前面两条路都不行（用户实测"键盘能用、鼠标完全转不了视角"，两次修复都无效）：
///
/// 1. **`WindowEvent::Focused` 不够**：它只在收到 `WM_SETFOCUS` 时发出。而
///    **窗口在创建时就已经有焦点**是启动的常见情形 —— 此时 Windows **不会再发一次
///    `WM_SETFOCUS`** ⇒ winit 永远不发 `Focused(true)` ⇒ `self.focused` 停在初值
///    `false` ⇒ `want` 恒假 ⇒ **光标永不抓取**。键盘与左右键不经过 `want`，所以照常工作。
///
/// 2. **`Window::has_focus()` 也不行** —— winit 0.30 的文档自己写着
///    "This queries the same state information as WindowEvent::Focused"，
///    Windows 实现是 `window_state.has_active_focus()`，读的**就是同一个内部标志**。
///    事件不来，它同样是 false。**2026-09-13 我第一版修复正是栽在这里。**
///
/// **判据用"前台窗口属于本进程"而不是"HWND 相等"**：本工程的 winit 程序有多个窗口
/// （AGENTS.md 记录过 `Process.MainWindowHandle` 拿到的**不是**接收输入的那个），
/// 拿 HWND 做相等比较会漏判。比进程 ID 与窗口身份无关，稳得多。
#[cfg(target_os = "windows")]
pub(crate) fn window_is_foreground(_window: &winit::window::Window) -> Option<bool> {
    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> *mut core::ffi::c_void;
        fn GetWindowThreadProcessId(hwnd: *mut core::ffi::c_void, pid: *mut u32) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> u32;
    }
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_null() {
        last_fg_pid().store(0, std::sync::atomic::Ordering::Relaxed);
        return Some(false); // 没有任何前台窗口（极少见）：按未聚焦处理
    }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(fg, &mut pid) };
    last_fg_pid().store(pid, std::sync::atomic::Ordering::Relaxed);
    if pid == 0 {
        return None; // 查询失败：让调用方回退
    }
    Some(pid == unsafe { GetCurrentProcessId() })
}

/// 最近一次查询到的**前台窗口所属进程 ID**。只用于日志：
/// 用户报告"鼠标转不了视角"时，`cam:` 行里的 `fgpid=` 与 `mypid=` 一比就知道
/// 是"窗口真没在前台"还是"判据本身错了"（2026-09-13 连续两次误判的教训）。
#[cfg(target_os = "windows")]
pub(crate) fn last_fg_pid() -> &'static std::sync::atomic::AtomicU32 {
    static V: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    &V
}
#[cfg(not(target_os = "windows"))]
pub(crate) fn last_fg_pid() -> &'static std::sync::atomic::AtomicU32 {
    static V: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    &V
}

#[cfg(target_os = "windows")]
pub(crate) fn my_pid() -> u32 {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> u32;
    }
    unsafe { GetCurrentProcessId() }
}
#[cfg(not(target_os = "windows"))]
pub(crate) fn my_pid() -> u32 {
    std::process::id()
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn window_is_foreground(_window: &winit::window::Window) -> Option<bool> {
    None
}

/// 捕获方式决策：返回 `(cursor_locked, grabbed)`。
///
/// `Locked` 只在 raw 相对增量真会到达时才有意义（见 [`RAW_MOUSE_MOTION`]），
/// 否则退到 `Confined` + 绝对位置路径（该路径靠 `CursorMoved` 里的回中维持，
/// 详见 `window_event`）。抽成纯函数是为了让"raw 不可用的平台连试都不试
/// `Locked`"这条不变量能被单测钉住 —— 选错的后果是视角静默失效。
pub(crate) fn cursor_grab_plan(
    raw_motion: bool,
    locked_ok: impl FnOnce() -> bool,
    confined_ok: impl FnOnce() -> bool,
) -> (bool, bool) {
    if raw_motion && locked_ok() {
        return (true, true);
    }
    (false, confined_ok())
}

/// 帧率上限（present 节流）：0 = 无上限（压测模式，主循环全速跑以暴露渲染瓶颈）。
/// 设回正数（如 300）即恢复帧率门控。
pub(crate) const MAX_FPS: u64 = 0;

/// 有效帧率上限（fps）：前台用前台上限，**未聚焦**时再夹上「后台上限」。
///
/// 为什么需要这一条（2026-09-27 实测）：窗口失焦后主循环**照样全速渲染** ——
/// 探针实测 `steel-front` 自己占 99–105% 的 GPU 3D 引擎份额、dwm 只剩 0–1%，
/// 于是"游戏挂在后台"时整个桌面的合成、拖动窗口、打字回显都在排队，
/// 用户报的「一运行游戏整机就像卡死」就是这个（不是 CPU 降频：同一轮实测
/// CPU 频率 3.1–4.8 GHz、GPU 45–70W/115W、无任何节流标志）。
///
/// `RV3D_SHOT_AT` 的解析（纯函数，可单测）：以 `,` 分隔的秒数 ⇒ **去重升序**列表。
///
/// 🔴 2026-10-03 加。存在的理由：**Linux 上原来没有任何自动截图手段** ——
/// 唯一的触发是 F12 按键，而 Wayland 下注入按键要抢焦点（`ydotool`/XTEST 那一类），
/// 正好违反铁律 C 的鼠标安全协议。于是 `resize_probe.sh` 只能停在"没有画面取证"，
/// 而 Windows 侧的同一探针是能 F12 的。给一个**不需要输入**的触发就把这个不对称补平，
/// 顺带 Windows 上做无人值守取证也一样用得上。
///
/// 非法项（非数字、NaN、<=0）**直接丢弃**：这是取证开关，不该因为写错一个数就让整局启动失败。
pub(crate) fn parse_shot_at(raw: &str) -> Vec<f32> {
    let mut v: Vec<f32> = raw
        .split(',')
        .filter_map(|t| t.trim().parse::<f32>().ok())
        .filter(|t| t.is_finite() && *t > 0.0)
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v.dedup();
    v
}

/// 取走**已到点**的时刻（纯函数，可单测）：把 `pending` 里所有 `<= elapsed` 的项移出，
/// 返回移出的个数（= 这一帧该截几张）。
///
/// 🔴 用「取走」而不是「判等」：某一帧可能跨过两三个时刻（长卡顿、低帧率、
/// 或首个窗口还没渲染的那些时刻），判等会**永久漏掉**被跨过的那些 —— 而截图是取证用的，
/// 漏一张就少一份证据，且**不会报错**。
pub(crate) fn take_due_shots(pending: &mut Vec<f32>, elapsed: f32) -> usize {
    let before = pending.len();
    pending.retain(|t| *t > elapsed);
    before - pending.len()
}

/// 语义（判据 `frame_cap_*` 四条测试）：
/// - `0` = 不设上限；两侧都为 0 时返回 0（保持压测路径逐字节不变）。
/// - 未聚焦且 `bg > 0` 时取 `min(fg, bg)`（`fg == 0` 视作无穷大）⇒ **后台只会更严，绝不会放宽**。
/// - 聚焦时忽略 `bg`，只受 `fg` 约束。
pub(crate) fn effective_frame_cap(fg_fps: f32, bg_fps: f32, focused: bool) -> f32 {
    if focused || bg_fps <= 0.0 {
        return fg_fps.max(0.0);
    }
    if fg_fps <= 0.0 {
        return bg_fps;
    }
    fg_fps.min(bg_fps)
}

/// 环境变量真值解析（"1"/"true"/"on" = 真；其余为假）
pub(crate) fn env_truthy(name: &str) -> bool {
    crate::syscfg::cfg(name)
        .map(|v| matches!(v.as_str(), "1" | "true" | "on" | "TRUE" | "ON" | "True"))
        .unwrap_or(false)
}

/// 环境变量浮点读取（解析失败返回 None）
pub(crate) fn env_f32(name: &str) -> Option<f32> {
    crate::syscfg::cfg(name).and_then(|s| s.parse::<f32>().ok())
}

/// 事件循环后端选择（只有 Linux 有两种 WSI 后端，其它平台恒为 `Auto`）。
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendChoice {
    /// 不强制：winit 自己在有 `WAYLAND_DISPLAY` 时选 Wayland，否则选 X11。
    Auto,
    /// 经 `EventLoopBuilderExtX11::with_x11()` 强制 X11 / XWayland。
    X11,
}

/// `RV3D_BACKEND` + WSL 自动判定 → 后端选择（纯函数，可单测）。
///
/// 优先级：**显式环境变量 > WSL 自动判定 > Auto**。
///
/// - `x11` / `xwayland` ⇒ `X11`，**与是不是 WSL 无关** —— 这正是修掉的那条：
///   旧代码把「强制 X11」写死在 `is_wsl` 分支里，原生 Linux 用户**没有任何手段**
///   退回 XWayland（winit 0.30 已删 `WINIT_UNIX_BACKEND`）。
/// - `wayland` ⇒ `Auto`。winit 0.30 没有「强制 Wayland」的入口，
///   而 `Auto` 在有 `WAYLAND_DISPLAY` 时就是 Wayland ⇒ **语义等价**，
///   且不承诺一个做不到的事（写成 `Wayland` 变体也只能是空操作）。
/// - 其它 / 未设 ⇒ `auto_x11`（WSL **且** 有 Wayland 会话）为真时 `X11`，否则 `Auto`。
///
/// 判据 = `backend_choice_x11_is_reachable_on_native_linux`：**非 WSL 的原生 Linux
/// 也必须能选到 X11**（旧逻辑唯一做不到、而用户最需要的那一件事）。
#[cfg(target_os = "linux")]
pub(crate) fn backend_choice(env: Option<&str>, auto_x11: bool) -> BackendChoice {
    match env.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("x11") | Some("xwayland") => BackendChoice::X11,
        Some("wayland") => BackendChoice::Auto,
        _ => {
            if auto_x11 {
                BackendChoice::X11
            } else {
                BackendChoice::Auto
            }
        }
    }
}

/// 是否允许抓取光标。`RV3D_NO_CAPTURE=1` ⇒ **永不允许**。
///
/// 为什么需要它（2026-09-28，Linux 适配）：Windows 侧的「鼠标安全协议」（用户 2026-09-03
/// 明确要求）靠的是 **`PostMessage` 投键 + 永不抢前台** —— 于是自动化能在**不抓光标**的
/// 前提下驱动游戏。Linux **没有** PostMessage 这条路，而本引擎在 `Playing` 态**一定会**
/// 抓光标（`Locked` 或 `Confined`）：自动化跑一局 = 用户的指针被锁进游戏窗口。
/// 抓取会随进程退出而释放，但"跑测试期间桌面指针被夺走"本身就不该是自动化的副作用。
///
/// ⇒ Linux 侧的正确对偶不是"换个注入方式"，而是**把捕获关掉**：
/// `RV3D_NO_CAPTURE=1` 时 `capture_wanted` 恒假，无论焦点/状态如何。
/// 冒烟与性能脚本一律带上它 —— 这样"自动化不会碰用户输入"就成了**代码保证**，
/// 而不是靠调用方自己记得别抢焦点。
///
/// ⚠️ 它只关**抓取**，不关视角输入：非捕获态本来就有左键拖拽转视角那条路
/// （`dragging`），所以关掉捕获之后游戏依然可玩、可被脚本驱动。
pub(crate) fn no_capture() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| env_truthy("RV3D_NO_CAPTURE"))
}

/// 捕获策略（纯函数，可单测）：`allowed && focused && Playing && 无面板`。
///
/// 抽出来的理由与 `cursor_grab_plan` 相同 —— 这条策略选错的后果是**静默**的：
/// 漏掉 `allowed` 就是关不掉的指针锁定，漏掉 `esc_menu_open` 就是菜单里鼠标被锁死
/// （只能 Alt+F4）。写成纯函数才能把这两个方向都钉进测试。
pub(crate) fn capture_wanted(
    focused: bool,
    playing: bool,
    settings_open: bool,
    esc_menu_open: bool,
    allowed: bool,
) -> bool {
    allowed && focused && playing && !settings_open && !esc_menu_open
}

/// 锁定态「相对增量到底有没有来」的判定结果（纯函数 `lock_observation` 的值域）。
///
/// 为什么需要它（2026-09-28，Linux 适配 —— 这是本仓在 Windows 上踩过两次的**同一个**
/// 形态第三次出现）：`set_cursor_grab(Locked)` 返回 `Ok` **不等于锁生效了**。
/// - **Wayland**：winit 的 `apply_on_pointer` 只对**已经 `wl_pointer::enter` 过**的指针
///   生效 —— 指针还没进窗口时它**什么都没做也返回 `Ok`**；而且 winit 完全忽略合成器的
///   确认事件（`ZwpLockedPointerV1` 的 Dispatch 函数体是空的）⇒ 应用层**无法**从
///   返回值或事件里知道锁有没有生效。
/// - 而 Wayland 下 `DeviceEvent::MouseMotion` 的唯一来源是 `zwp_relative_pointer_v1`，
///   它**只在 `lock_pointer` 里一起创建** ⇒ **没锁 = 零 raw 事件 = 视角彻底不动**。
/// - 绝对位置那条路在 Wayland 上也不通：`set_cursor_position` 只在已 `Locked` 时成功
///   ⇒ `Confined` 下指针撞到窗口边就再也转不动。**所以不能"降级"了事**，
///   必须让这个失败**可见**（症状与"鼠标坏了"完全一样，用户无从判断）。
///
/// ⇒ 唯一可信的判据是**行为证据**：锁定之后真的收到过相对增量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LockObservation {
    /// 还在观察窗内、且尚未收到增量 —— 结论未到，什么都别做
    Pending,
    /// 收到过相对增量 ⇒ 锁**确实生效**（Wayland 下这是唯一可信的证据）
    Confirmed,
    /// 观察窗已过、一个增量都没有 ⇒ 锁很可能是**假成功**
    NoMotion,
}

/// 观察窗长度（毫秒）。取值理由：短了会把"用户刚锁定还没来得及动鼠标"误判成假成功；
/// 长了用户要看着一个不动的视角干等。1.5s 够人手自然移动一次，也不至于让人以为卡死。
pub(crate) const LOCK_OBSERVE_MS: u128 = 1500;

/// 锁定态观察判定（纯函数，可单测）。
///
/// **增量优先**：只要收到过只要有 1 个相对增量就立刻 `Confirmed`，不必等满观察窗 ——
/// 那是最强的正面证据，没有理由再等。
pub(crate) fn lock_observation(elapsed_ms: u128, motion_delta: u64, window_ms: u128) -> LockObservation {
    if motion_delta > 0 {
        return LockObservation::Confirmed;
    }
    if elapsed_ms >= window_ms {
        LockObservation::NoMotion
    } else {
        LockObservation::Pending
    }
}

/// 把「请求的分辨率」夹进显示器物理尺寸，返回窗口要请求的**物理**像素尺寸。
///
/// 🔴 **2026-09-28 修（Linux 适配）**：调用方原来写的是
/// `.with_inner_size(LogicalSize::new(w as f64 / 1.5, h as f64 / 1.5))` ——
/// 那个 `/1.5` 是 Windows 那台机器 `scale_factor = 1.5` 的**硬编码补偿**。
/// 它带来两个后果，都只在非 Windows 上暴露：
///
/// 1. **启动尺寸与设置里改出来的尺寸不一致**：配置 2560x1600、scale=1.0 的 Linux 上
///    窗口实际是 1706x1066；而设置面板里改分辨率走的是 `PhysicalSize`（见 `main.rs`
///    里应用新分辨率那条路）⇒ 同一个分辨率有两套语义。Wayland 下更糟：窗口尺寸直接
///    决定交换链尺寸（`currentExtent` 未定义，见 `Renderer::window_extent`），
///    于是错的是**整条渲染尺寸链**，不只是窗口。
/// 2. 尺寸被夹小了，而日志照样打印"窗口创建成功: 2560x1600" ⇒ **日志与事实不符**。
///
/// ⇒ 语义定死：**这里的 `w`/`h` 与 `monitor` 全是物理像素**（`RESOLUTIONS` 与配置项
/// 本来就是物理分辨率，`monitor.size()` 也是物理），所以一律用 `PhysicalSize`，
/// **不再有任何魔法缩放系数**。判据 = `window_request_is_physical_and_clamped`
/// （它同时钉住"不许再乘除任何常数"与"超屏必须等比缩"）。
///
/// `monitor = None`（Wayland 下 `primary_monitor()` 可能为 None）时不做夹取 ——
/// 与其按一个猜出来的尺寸缩，不如如实请求，让合成器去处理。
pub(crate) fn window_physical_request(w: u32, h: u32, monitor: Option<(u32, u32)>) -> (u32, u32) {
    // 下限与 winit/合成器能接受的最小窗口一致，避免请求 0 尺寸
    let (mut w, mut h) = (w.max(320), h.max(200));
    if let Some((mw, mh)) = monitor {
        if w > mw || h > mh {
            let scale = (mw as f32 / w as f32).min(mh as f32 / h as f32);
            w = ((w as f32 * scale) as u32).max(320);
            h = ((h as f32 * scale) as u32).max(200);
        }
    }
    (w, h)
}

impl GameApp {
    pub(crate) fn sync_cursor(&mut self) {
        // 🔴🔴 2026-09-13 修（用户实测"鼠标完全转不了视角"的根因）：
        // **焦点不能只靠 `WindowEvent::Focused` 事件**。
        // 窗口在**创建时就已获得焦点**是启动的常见情形，此时 Windows **不会再发
        // 一次 `WM_SETFOCUS`** ⇒ winit 不发 `Focused(true)` ⇒ `self.focused` 永远停在
        // 初值 `false` ⇒ `want` 永远为假 ⇒ **光标永不抓取、视角永不响应鼠标**，
        // 而键盘与左右键照常工作（它们不经过 `want`）—— 正是用户描述的现象。
        // 2026-09-12 把初值从 `true` 改成 `false` 修掉了"没焦点却自认为有焦点"，
        // 但代价就是这一条：**只信事件就永远抓不住**。
        // ⇒ 直接问操作系统（`Window::has_focus()`），事件只作快速路径。
        let actually_focused = match &self.window {
            // 优先直接问系统（见 window_is_foreground 的注释：事件与 has_focus() 都不可靠）
            Some(w) => window_is_foreground(w).unwrap_or_else(|| w.has_focus()),
            None => return,
        };
        self.focused = actually_focused;
        let Some(window) = &self.window else {
            return;
        };
        let want = capture_wanted(
            self.focused,
            self.game.state() == GameState::Playing,
            self.game.settings_open(),
            self.game.hud.esc_menu_open,
            !no_capture(),
        );
        // ESC 菜单/设置面板打开或失焦时释放鼠标（2026-08-15：菜单需鼠标点选）
        if want && !self.cursor_captured {
            // Locked：系统级指针锁定 + 相对 MouseMotion，光标不会飞出窗口。
            // 仅在 raw 相对增量真会到达的平台才用它（见 RAW_MOUSE_MOTION）：
            // 其余平台退 Confined + 绝对位置路径（Xwayland 与本机 Windows 同路）。
            let (locked, grabbed) = cursor_grab_plan(
                RAW_MOUSE_MOTION,
                || window.set_cursor_grab(CursorGrabMode::Locked).is_ok(),
                || window.set_cursor_grab(CursorGrabMode::Confined).is_ok(),
            );
            // 🔴🔴🔴 2026-09-13 定案（用户"鼠标完全转不了视角"的真正根因）：
            // **在绝对位置路径上绝不能隐藏光标。**
            //
            // winit 0.30 的 Windows 后端（`window_state.rs` 的 `refresh_os_cursor`）这么写：
            //     if locked        { 钉在窗口中心 1x1 }
            //     else if HIDDEN   { 钉在窗口中心 1x1 }   ← ← ← 就是这条
            //     else             { 限制在 client_rect 内 }
            // 注释说这是为了"防止隐藏的光标去激活任务栏"。对**真相对鼠标**的游戏合理，
            // 但我们的 Windows 路径是 **Confined + 绝对位置**（见 `RAW_MOUSE_MOTION`）——
            // 视角完全靠 `CursorMoved` 的 `dx` 算出来。**光标被钉在一个像素上，
            // `dx` 恒为 0，视角自然纹丝不动**，而键盘与左右键照常工作。
            //
            // 实测证据（2026-09-13，在用户真实会话的进程还活着时量的）：
            //   `GetClipCursor` = 853,533 - 854,534   ⇒ **1×1 像素**
            //   我自己的诊断里 `mouse=15` 但 `at=(1280,800)` **五条采样完全相同**（没动过）
            //
            // ⇒ 只在**真 Locked**（相对鼠标，`DeviceEvent::MouseMotion` 驱动视角）时才隐藏光标。
            // 绝对路径**不能隐藏**（否则 winit 把指针钉成 1×1），但又不能给玩家看一个箭头 ——
            // 用户 2026-09-13 反馈"鼠标图标一直浮在中心"。
            //
            // ⇒ 第三解：**设一个 1×1 全透明的自定义光标**。
            // winit 只检查自己的 `CursorFlags::HIDDEN` 标志，**从不看光标图像本身**，
            // 所以"未隐藏 + 图像透明"同时满足两边的要求：
            //   * winit 侧 ⇒ 走 `Some(client_rect)`，光标能在窗口内自由移动（视角能转）
            //   * 玩家侧 ⇒ 屏幕上什么都看不见
            // 释放时用 `CursorIcon::Default` 恢复系统箭头（菜单里要用）。
            if locked {
                window.set_cursor_visible(false);
            } else {
                // 用启动时造好的那个 1x1 全透明光标（见 `resumed` 里的注释）。
                // 它必须由事件循环创建，所以在这里只做 `set_cursor`。
                match self.blank_cursor.clone() {
                    Some(c) => {
                        window.set_cursor_visible(true);
                        window.set_cursor(c);
                    }
                    // 造不出来（驱动拒绝 1×1）就退回可见箭头 —— 能玩优先
                    None => window.set_cursor_visible(true),
                }
            }
            self.cursor_captured = grabbed || locked;
            self.cursor_locked = locked;
            self.abs_baseline_valid = false;
            // 开始观察这次的锁到底有没有生效（见 `LockObservation`）。
            // 基准取**当前的** `cursor_evt_count`，所以这个窗口内来一个增量就算确认。
            if locked {
                self.lock_observe = Some((Instant::now(), self.cursor_evt_count));
                self.lock_retried = false;
            } else {
                self.lock_observe = None;
            }
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
            // 只在**模式变化**时打（见 `cursor_grab_logged` 的注释）：
            // 抓取持续失败时这段代码每帧都会走到，逐帧打日志会把 logcat 刷爆。
            let mode: u8 = if locked {
                2
            } else if grabbed {
                1
            } else {
                0
            };
            if mode != self.cursor_grab_logged {
                self.cursor_grab_logged = mode;
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
            }
        // ---- 锁定态观察：锁到底有没有真的生效（见 `LockObservation`）----
        //
        // 这一段是**诊断 + 一次补救**，不是降级：Wayland 下不能退回 Confined ——
        // `set_cursor_position` 只在已 Locked 时才成功，Confined 下指针撞到窗口边
        // 就再也转不动（比"完全不转"更难排查）。所以对 NoMotion 的处理是
        // **补抓一次**（指针此时多半已经 enter 了，而 `apply_on_pointer` 只对
        // 已 enter 的指针生效 ⇒ 这次多半能成），再不行就**给出明确的提示**。
        if self.cursor_captured && self.cursor_locked {
            if let Some((started, motion_at_start)) = self.lock_observe {
                let delta = self.cursor_evt_count.saturating_sub(motion_at_start);
                match lock_observation(started.elapsed().as_millis(), delta, LOCK_OBSERVE_MS) {
                    LockObservation::Pending => {}
                    LockObservation::Confirmed => {
                        // 锁确实生效 —— 收工，之后不再观察（也不再补抓）
                        self.lock_observe = None;
                    }
                    LockObservation::NoMotion => {
                        if !self.lock_retried {
                            // 指针很可能在抓取之后才进入窗口 ⇒ 补抓一次
                            self.lock_retried = true;
                            let ok = window.set_cursor_grab(CursorGrabMode::Locked).is_ok();
                            self.lock_observe = Some((Instant::now(), self.cursor_evt_count));
                            log::warn!(
                                "input: 锁定后 {}ms 内一个相对增量都没收到 —— 锁可能没生效，\
                                 已补抓一次（set_cursor_grab(Locked) -> {}）。\
                                 原因：winit 在指针尚未 enter 窗口时调 lock_pointer 会\
                                 **什么都没做却返回 Ok**，且它忽略合成器的确认事件，\
                                 所以失败无法从返回值看出来。",
                                LOCK_OBSERVE_MS,
                                if ok { "Ok" } else { "Err" }
                            );
                        } else if !self.lock_fake_warned {
                            self.lock_fake_warned = true;
                            self.lock_observe = None;
                            log::error!(
                                "input: 🔴 补抓之后仍然零相对增量 ⇒ 本会话的鼠标视角**不会工作**\
                                 （锁定是假成功）。这不是鼠标故障，是后端差异：Wayland 下\
                                 Locked 的失败是静默的，且 Confined 也无法回转指针。\
                                 ⇒ 请用 RV3D_BACKEND=x11 走 Xwayland 重开一局 —— \
                                 那里 set_cursor_grab(Locked) 会如实返回 NotSupported，\
                                 引擎会自动落到 Confined + 绝对位置路径，而 X11 的\
                                 set_cursor_position 走 XWarpPointer 是真的生效的。\
                                 （本提示只发一次；`cam:` 日志里的 evt 计数可继续观测。）"
                            );
                        }
                    }
                }
            }
        }
        } else if !want && self.cursor_captured {
            self.release_cursor_immediately("sync_cursor");
        }


    }
}
