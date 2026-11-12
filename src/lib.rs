//! 钢铁前线 (Steel Front) - 程序入口
//!
//! 游戏主循环：
//! 1. 初始化窗口（winit）
//! 2. 初始化 Vulkan 渲染器（ash）
//! 3. 事件循环处理输入
//! 4. 每帧更新相机并渲染

// 🔴🔴 2026-09-13：**release 构建不创建控制台窗口**。
//
// 病根：Rust 的 bin 默认是 **console 子系统**，所以双击 exe 时 Windows 会**先创建一个
// 控制台窗口并让它成为前台**，游戏窗口随后才创建。实测枚举本进程的窗口可见：
//     class='Window Class'        title='Steel Front - Vulkan'   ← 游戏窗口
//     class='ConsoleWindowClass'  title='...steel-front.exe'     ← 控制台，可见
// 两个同属一个进程的顶层窗口互相争前台。而 `sync_cursor` 要求
// "本进程是前台进程" 才抓光标（见 `window_is_foreground`）—— 于是**光标永不抓取**，
// 表现就是"键盘与左右键有反应、鼠标完全转不了视角"（用户 2026-09-13 反复报告）。
//
// 改成 windows 子系统后根本不存在控制台 ⇒ 没有可争的前台 ⇒ 抓取成立。
// **调试构建保留控制台**（`debug_assertions` 时保留），否则 `cargo run` 时看不到日志。
// 发布版的日志走 stderr 重定向：用 `SteelFront.bat` 启动会落到
// `logs/play_latest.log.err`。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 🔴 2026-10-03：**顶部那条 `#![cfg_attr(not(windows), allow(dead_code))]` 已删除。**
//
// 它是 2026-09-26 加的，当时的理由是：非 Windows **只是"只编译不运行"的交叉验证目标**
// （铁律 E 的 `cargo check --target aarch64-unknown-linux-gnu`）；拆掉后剩下的警告
// 全是"按平台设计就只在 Windows 用得到"的东西（waveOut 常量、CJK 字模表）。
//
// **那个前提现在不成立了**：Linux 已经是原生支持、而且**要真的跑起来**的平台
// （见 `docs/linux-native.md`，PR #1 已合并）。于是这个 blanket allow 会在 Linux 上
// **藏住真正的死代码** —— 而 Linux 恰恰是现在必须保证干净的一侧。
//
// 拆掉之后实测浮出 4 条，全部在 `audio_out.rs`：
//   * `WaveOutSink` 的字段与 `new`（3 条）：它只在 Windows 上被构造
//     （`DefaultSink` 在 Linux 是 `AlsaSink`），而原来只有**字段**带 cfg，
//     结构体与 impl 在 Linux 上也编 ⇒ 整条链没人用；
//   * `submit_plan` / `warn_submit_truncation_once`：只被 `WaveOutSink::submit` 调用。
//
// 修法是**按平台门控**，不是加 `allow`（铁律 F：看到死代码必须回答"为什么没被接线"）：
// 整个 `WaveOutSink` 及其 impl 加 `#[cfg(target_os = "windows")]`；两个**纯函数**用
// `#[cfg(any(target_os = "windows", test))]` —— 生产代码只有 Windows 用得到，但
// `submit_plan_never_exceeds_source_or_capacity` 是纯函数判据，不该因为平台丢掉
// Linux 上的覆盖。
//
// ⇒ 现在两侧都是**真的 0 警告**，不再是"靠 allow 压出来的 0"。

/// 构建期内嵌着色器（build.rs 生成 OUT_DIR/shaders.rs）
pub mod shaders {
    include!(concat!(env!("OUT_DIR"), "/shaders.rs"));
}

mod engine;
mod audio;
mod audio_out;
mod llm_cmd;
mod net;
mod ui;
mod log_util;
mod config;
mod app_window;
pub(crate) use app_window::*;
mod app_gun;
pub(crate) use app_gun::*;
mod app_assets;
mod app_touch;
mod app_render;
mod app_update;
mod app_events;
mod perf_log;
mod syscfg;

use std::time::{Duration, Instant};

use winit::{
    application::ApplicationHandler,
    event::{
        DeviceEvent, DeviceId, WindowEvent,
    },
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

use engine::camera::{Camera, CameraMode, KeyState};
use engine::game::{Game, GameState};
use engine::renderer::{QualityPreset, Renderer};
use engine::window;
use net::{Client, Server};
use ui::RESOLUTIONS;
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

/// 本平台是否会投递 `DeviceEvent::MouseMotion`（raw 相对增量）。
///

/// 单帧预算（纳秒；`MAX_FPS = 0` 表示不设上限，预算为 0，不做 sleep/spin 节流）。
///
/// 写成 `match` 而不是 `if MAX_FPS > 0`：后者在 `MAX_FPS` 当前取值 0 下，比较的
/// 真分支永远不可达（clippy 以 deny 级报 `unnecessary_sanity_check`），而这里
/// "0 = 无上限"是**约定**、不是运行时输入，用模式匹配把它摊开更直白，
/// 也仍然挡住 `1_000_000_000 / 0`。改 `MAX_FPS` 为非 0 时两个分支都照旧工作。
const FRAME_BUDGET: Duration = match MAX_FPS {
    0 => Duration::ZERO,
    fps => Duration::from_nanos(1_000_000_000 / fps),
};

// ============================================================
// 第一人称枪摆动（viewmodel sway）参数 —— 2026-09-01 平滑化重写
// ============================================================
/// 步态相位推进速率（弧度 / 米水平行程）。取值是从"视觉频率"反推的，不是真实步幅：
/// 本作玩家水平速度只有两档——6.0 m/s（game.rs:68 PLAYER_SPEED）与开镜
/// 3.9 m/s（game.rs move_first_person 的 ads_factor 0.65）。1.047 rad/m × 6.0 m/s
/// = 6.28 rad/s = **侧向 1.0 Hz / 上下 2.0 Hz**，这是"人在跑"的观感上限。
/// 若按真实步幅（1.2 m 一周期）会得到 5 Hz 上下 + 2.5 Hz 左右的高频抖，
/// 正是玩家报的"高频小幅度摆动"。
/// 旧实现是 `sin(anim_clock*7.5)`：频率与速度无关（固定 1.2 Hz），停下仍继续晃，
/// 通断瞬间相位随机 → 每次开关都是一次位置跳变。改按行程推进后，停止即冻结相位，
/// 包络再平滑收到 0，起步从同一相位平滑起振。
const GUN_GAIT_PHASE_PER_M: f32 = 1.047;
/// 相位推进的角速度上限（弧度/秒）。8.0 → 侧向 ≤1.27 Hz、上下 ≤2.54 Hz。
/// 有了这道上限，即使日后加入冲刺/载具把速度推到 20 m/s 以上，摆频也不会
/// 爬进"读成振动"的区间（>3 Hz）；当前最高速度 6 m/s 只用到 6.28 rad/s，不触发。
const GUN_GAIT_MAX_RATE: f32 = 8.0;
/// 侧向摆幅上限（视空间米，饱和时）。0.010 m 在腰射锚距 0.60 m、垂直 FOV 70°
/// 下的屏幕位移 = 0.010/(0.60×tan35°) = 2.4% 半屏高 → 1080p 约 ±13 px（峰峰 26 px），
/// 与旧实现的 0.009 同量级——玩家抱怨的是"高频"，不是"大幅度"，故幅值保持原档；
/// 再大就开始读成"枪在飘"而不是"人在走"。
const GUN_SWAY_SIDE_M: f32 = 0.010;
/// 上下摆幅（米）：落地冲击约为侧向的 80%（人体质心垂向位移 ~5 cm、侧向 ~4 cm
/// 的比例），取 0.008 → 1080p 约 ±10 px。
const GUN_SWAY_BOB_M: f32 = 0.008;
/// 前后摆幅（米）：枪随手臂前后拖拽。取侧向的一半（0.005）——沿视轴方向的位移
/// 只改变成像大小，同样的米数在视觉上比侧向更抢眼，故幅值必须更小。
const GUN_SWAY_FORE_M: f32 = 0.005;
/// 侧向"惯性滞后"偏移（米）：与侧移速度同幅反号（向右跨步时枪相对身体留在后面），
/// 这是 strafe 时唯一有重量感的线索。取侧向摆幅的 40%（0.004）——它可与步态侧摆
/// 同相叠加，最坏合计 0.014 m（约 ±18 px），仍贴近旧实现意图中的 0.009 m 档；
/// 再大就会盖过步态本身，左右跨步看着像"枪在横扫"。
const GUN_SWAY_LEAN_M: f32 = 0.004;
/// 速度→摆幅的 smoothstep 区间下界（m/s）：0.5 m/s 以下视为静止。
/// 旧实现是 `speed > 0.6` 的**布尔**判据，速度在阈值附近逐帧来回穿越时
/// 摆动偏移在"满幅"和"0"之间硬跳（跳变频率 = 帧率）→ 高频小振幅抖动 + 残影。
const GUN_SWAY_SPEED_LO: f32 = 0.5;
/// 速度→摆幅的 smoothstep 区间上界（m/s）：取 game.rs PLAYER_SPEED = 6.0，
/// 即本作最高水平速度才饱和——摆幅因此严格有上界，且在开镜移速 3.9 m/s 时
/// 自然落到 smoothstep=0.67 包络（叠加按轴 ADS 因子后侧向只剩 0.67×0.12≈8%）。
/// 旧实现的幅值虽恒定但通断无界，起步瞬间从 0 跳到满幅。
const GUN_SWAY_SPEED_HI: f32 = 6.0;
/// 速度低通的时间常数（秒）。0.06 s ≈ 165 fps 下的 10 帧：足以抹掉逐帧位移噪声
/// （玩家本作是**瞬时无惯性**位移——game.rs move_first_person 直接按 dt 改 pos，
/// 所以按键的那一帧速度就从 0 跳到 6 m/s，低通是唯一的连续化手段），
/// 又远小于一次落脚间隔（6 m/s 下 ≈0.5 s），不会让人感到"枪跟不上脚"。
/// 超过 0.15 s 开始出现脱节感。
const GUN_SWAY_SMOOTH_TAU: f32 = 0.06;
/// 后坐冲量衰减时间常数（秒）。0.075 s → 单发在 0.2 s 内衰到 7%（视觉上一次
/// 干脆的"上抬→回落"）；连发按 0.1 s 间隔时包络回不到 0，自然叠成持续抬升。
/// 旧实现用 `(1-t)²` 抛物线 + 0.30 s 硬截止，且阻尼系数在 0.25 s 整点从 0.15
/// 阶跃到 1.0 —— 两处都是位置不连续。
const GUN_RECOIL_TAU: f32 = 0.075;
/// 开火瞬间的摆动阻尼（连续量，随 kick 指数回升到 1.0）：0.45 = 枪在后坐的一瞬
/// 摆幅降到 45%。旧实现是 0.15 → 1.0 的阶跃。
const GUN_SWAY_FIRE_DAMP: f32 = 0.45;
/// ADS（开镜）各轴的摆幅保留比例：侧向 12% / 上下 25% / 前后 15%。
/// 机瞄贴腮时身体运动仍会传到手上传感器上，但必须显著小于腰射。
const GUN_SWAY_ADS_SIDE: f32 = 0.12;
const GUN_SWAY_ADS_BOB: f32 = 0.25;
const GUN_SWAY_ADS_FORE: f32 = 0.15;
/// 腰射锚距（米）：与 `fp_gun_matrix` 的 hip_pos.z 同值，改一处必须改两处。
/// 它同时是「屏幕等幅」归一化的分母之一——视空间平移在屏幕上的位移
/// ∝ offset / (锚距 × tan(fov/2))，故开镜（锚距 0.42 m、fov 55°）会把同样的
/// offset 视觉放大 (0.60/0.42)×(tan35/tan27.5) = 1.43×1.35 ≈ **1.92 倍**。
/// 不补这个几何增益，就出现"越是开镜精确瞄准、左右移动时枪甩得越凶"。
const GUN_HIP_DEPTH_M: f32 = 0.60;
/// tan(腰射半视角) —— FOV 70° 的一半 35° 的正切（tan 35° = 0.7002075）。
/// 与 GUN_HIP_DEPTH_M 一起构成屏幕等幅基准；`gun_scale` 与摆动偏移共用
/// 同一个 `fov_gain`，保证"模型缩放"和"摆动平移"两条通道对 FOV 的补偿完全一致
/// （旧实现只有缩放补了 tan(fov/2)，平移没补——两条通道不一致就是 bug #2）。
const GUN_HIP_HALF_TAN: f32 = 0.700_208;

/// 🔴 **切枪动画**（2026-09-15）：切枪此前只是"禁止开火"的计时器，枪是直接跳变的。
///
/// 包络用 `sin(π·t)`：`t=0` 与 `t=1` 处都为 0、中点最大 —— **两端连续**是关键，
/// 直接对 `t` 做线性（或对 `1-t` 做三角波）会在起止两端留下速度阶跃，
/// 那正是本仓枪模历史上"残影/抖动"的成因（见 `GunSway` 上那段注释的同类教训）。
///
/// 幅值：0.18 m 下坠 + 12° 前倾 + 6° 侧转（视空间）。下坠量按 `screen_gain` 补偿，
/// 与后坐/摆动走同一套换算，否则开镜切枪时视觉幅度会放大近 2 倍。
const GUN_SWITCH_DROP_M: f32 = 0.18;
const GUN_SWITCH_PITCH_RAD: f32 = 0.21; // ≈12°
const GUN_SWITCH_ROLL_RAD: f32 = 0.10; // ≈6°

/// 冲刺持枪姿态（2026-09-26 补）：冲刺时枪**压低 + 前倾 + 侧转**。
///
/// 为什么以前没有：摆动/后坐/切枪/ADS 都做了，唯独"按住 Shift 跑"这个**最常见**的状态
/// 完全没有姿态变化 —— 玩家冲刺时枪仍端在瞄准线上，读起来像"滑行"而不是"奔跑"。
/// 幅度取真实持枪的样子：0.10 m 下坠（视空间，走 `screen_gain` 补偿）、24° 前倾、
/// 10° 侧转（枪身向身体中线内收）。这是**姿态**不是动画，靠 `GunSway::sprint` 的
/// 指数低通平滑过渡（与速度包络同一套 τ），所以起止无位置跳变。
const GUN_SPRINT_DROP_M: f32 = 0.10;
const GUN_SPRINT_PITCH_RAD: f32 = 0.42; // ≈24°
const GUN_SPRINT_ROLL_RAD: f32 = 0.18; // ≈10°

/// 换弹动作（2026-09-26 补）：换弹时枪下沉、枪口下压、向身体侧倾，中点最大。
/// 包络 = `reload_envelope(1 - progress)`：进度 1→0 的整段里**两端位移为 0**、中点为 1
/// （与切枪同一套 `sin(π·p)` ⇒ 起止不出现位置跳变；该包络两端速度最大，
/// 视觉上是"很快沉下去、再抬回来"）。
/// 幅度 0.07 m / 17° / 8°：比切枪小一号（换弹是手里的动作，切枪是整支枪出画）。
const GUN_RELOAD_DROP_M: f32 = 0.07;
const GUN_RELOAD_PITCH_RAD: f32 = 0.30; // ≈17°
const GUN_RELOAD_ROLL_RAD: f32 = 0.14; // ≈8°

/// 静止呼吸微摆（2026-09-26 补）：站立不动时枪口极缓慢地画 ∞ 字（0.22 Hz），
/// 幅值 2.2 mm / 0.02 rad。**只有"几乎不动"时才出现**（乘 `1 - 行走包络`），
/// 且开镜时再乘 0.25 —— 否则瞄准时枪口在漂，精确射击就不可信。
const GUN_IDLE_SWAY_M: f32 = 0.0022;
const GUN_IDLE_ROLL_RAD: f32 = 0.020;
const GUN_IDLE_HZ: f32 = 0.22;



/// 弹孔方片的边长（米）。marker 模板是 **±1 的单位盒** ⇒ 实例缩放 = **半幅**，所以这里
/// 按 `DECAL_SIZE_M * 0.5` 缩放基向量，画出来正好是 8cm 见方的一块
/// （障碍 marker 走的是另一条推导，见 `geom::Shape::template_half_extent`）。
/// 再小：10m 外不足一个像素（等于没画）；再大：读起来像贴纸而不是弹孔。
const DECAL_SIZE_M: f32 = 0.08;
/// 弹孔厚度（米）。方片**埋进墙里一半、露出约 1cm**：共面贴片会与墙面打 z-fighting，
/// 而整个浮在表面又会被看出是一块"贴上去的板"（侧壁在掠射角下可见）。
const DECAL_THICK_M: f32 = 0.016;
/// 方片中心沿法线的外移量（米）：正值 = 更凸出。取厚度的 1/4 ⇒ 露出 1.2cm、埋进 0.4cm。
const DECAL_LIFT_M: f32 = DECAL_THICK_M * 0.25;


// ============================================================
// 导入 GLB 枪模的顶点色烘焙（flat=3 直出通道）—— 2026-09-01 修"纯黑剪影"
// ============================================================
/// 参考反照率（线性空间）。0.24 的取值依据：本项目的 swapchain 首选
/// `B8G8R8A8_SRGB`（renderer.rs pick_format），而枪模走 build.rs 片元着色器的
/// `flat_flag > 2.5` 分支——顶点色**直出、不经曝光/色调映射/雾**，只被硬件做一次
/// linear→sRGB 编码。0.24 × 满光照 1.45 ≈ 0.35 线性 → 屏显 sRGB ≈ 0.62（受光面，
/// 读作亮钢）；0.24 × 环境下限 0.20 ≈ 0.048 线性 → 屏显 sRGB ≈ 0.24（背光面，
/// 读作暗钢但不纯黑）。明暗比 7:1 是"能看出是金属"的最低要求，纯黑剪影时是 1:0.06。
const GUN_REF_ALBEDO: f32 = 0.24;
/// 亮度阈值（Rec.709 luma）：低于此值判定"资产没有可用基色"。
/// 现存的 assets/guns/ak12.glb（所有武器 key 的公共回退）两个材质的
/// baseColorFactor 实测为 0.0573 / 0.0768，且**无 COLOR_0 属性、无 baseColorTexture**
/// ——Sketchfab 抠件的典型产物：颜色本在贴图里，导出时贴图被丢弃、只留下近乎纯黑的
/// 调色因子。而 engine/assets.rs 的 GLB 解析器不读贴图 → 顶点色 = 0.057 →
/// 0.057×(0.85..1.15) = 0.049..0.066，光照梯度被基色乘掉后只剩 ±0.017，
/// 屏显即"无明暗的纯黑卡片"。0.18 定在"正常深灰武器漆(0.25+)与坏资产(0.08)"之间。
const GUN_DARK_LUMA: f32 = 0.18;
/// 环境光下限（朝下的面）：0.20 = 地面反弹，保留暗部形状而不落到 0
const GUN_AMB_MIN: f32 = 0.20;
/// 环境光上限（朝上的面）：0.42 = 天穹漫反射。上下比 2.1:1 提供"哪面朝上"的读感
const GUN_AMB_MAX: f32 = 0.42;
/// 主光漫反射增益：与环境项相加后总区间 [0.20, 1.47]（旧实现 [0.85,1.15]，
/// 只有 1.35:1 的压缩动态范围，是"看不出明暗"的第二个原因）
const GUN_DIFF_GAIN: f32 = 1.05;
/// 高光 Phong 指数：26 → 亮带半角约 12°。金属件（机匣/枪管）需要一条窄而亮的
/// 高光才能读出曲率；聚合物/木质件拿不到高光仍保持哑光
const GUN_SPEC_POWER: f32 = 26.0;
/// 高光增益（金属 F0 近似）：0.55 而非 1.0，避免直出通道下高光过曝成白块
const GUN_SPEC_GAIN: f32 = 0.55;
/// 主光方向（**烘焙局部系**：枪口 +Z、枪顶 +Y，见 load_gun_glb 的 align）。
/// 与 `guns::assemble()` 用同一条光线，保证程序化枪模与导入枪模明暗方向一致。
/// viewmodel 在局部系里相对屏幕固定，所以在局部系烘光 = 屏幕上固定方向来光。
const GUN_KEY_DIR: glam::Vec3 = glam::Vec3::new(-0.45, 0.80, -0.30);


/// 游戏应用主管理结构
struct GameApp {
    /// `RV3D_SHOT_AT` 还没到点的时刻（升序）。空 = 不自动截图（默认）。
    /// 用「逐帧取走已到点的」而不是「判等」—— 理由见 `take_due_shots` 的文档。
    shot_pending: Vec<f32>,
    /// 自动截图的计时起点（第一次渲染时锚定，见 `about_to_wait`）。
    shot_t0: Option<Instant>,
    /// 致命错误的原因。**非 None 表示这次不是正常退出** ——
    /// 由 `event_loop.exit()` 只能走"正常"通道（`run_app` 返回 `Ok`），
    /// 所以致命路径必须额外把原因记在这里，循环结束后据此非零退出。
    /// 🔴 2026-10-03：`创建窗口失败` 与 `渲染器初始化失败` 原来只 `log::error!` + `exit()`，
    /// 于是引擎打出「程序正常退出」并以 **0** 退出 —— 用户解包后跑错目录（缺 assets/mesh.spv）
    /// 看到的就是这个。这是最容易被真人撞上的一类假绿灯（教训 46）。
    fatal: Option<String>,
    /// 🔴 **`renderer` 必须排在 `window` 前面**（2026-10-16 真机实测踩到，见下）。
    ///
    /// **Rust 按【声明顺序】析构结构体字段。** 原来是 `window` 在前 ⇒ 退出时
    /// **窗口（`wl_surface`/`wl_display` 的 wayland 代理）先销毁，交换链后销毁**
    /// ⇒ 驱动程序在销毁 Vulkan 对象时回头去碰**已经死掉的 wayland 代理**
    /// ⇒ **`SIGSEGV`（`SEGV_MAPERR`）**。真机核心转储的栈：
    /// ```text
    /// #0..#3  libwayland-client.so.0   wl_proxy_marshal_flags
    /// #4..#7  libnvidia-glcore.so.615.78.08
    /// #8      <Renderer as Drop>::drop          ← 崩在这里
    /// #9      drop_in_place<Renderer>
    /// #10     drop_in_place<GameApp>
    /// #11     run_steel_front
    /// ```
    /// ⚠️ **而引擎日志最后三行是「窗口关闭请求 / 性能日志已保存 / 程序正常退出」**
    /// —— **主循环是干净退出的**，崩在 `main` 返回之后的析构里。
    ///
    /// 顺序修正：**先销毁 `renderer`（它持有所有 Vulkan 对象，包括交换链），
    /// 再销毁 `window`。**
    ///
    /// 📌 判据 `renderer_field_drops_before_the_window`（源码级：只看声明顺序，
    /// 因为这件事**只能在真机退出时**才暴露，而退出路径恰恰是冒烟最容易漏掉的）。
    renderer: Option<Renderer>,
    /// winit 窗口
    ///
    /// ⚠️ **不要把它挪到 `renderer` 前面**（理由见上）。
    window: Option<Window>,
    /// FPS 相机
    camera: Camera,
    /// 键盘按键状态
    key_state: KeyState,
    /// Android 触控：左半屏方向杆（id + 起点）
    #[cfg(target_os = "android")]
    touch_move: Option<(u64, f32, f32)>,
    /// Android 触控：右半屏拖动转视角（id + 上一次位置）
    #[cfg(target_os = "android")]
    touch_look: Option<(u64, f32, f32)>,
    /// Android 触控：开火按钮（按住连发）
    #[cfg(target_os = "android")]
    touch_fire: Option<u64>,
    /// 最近一次窗口尺寸（触控分区用）
    screen_size: (f32, f32),
    /// 🔴 2026-09-13 诊断：`CursorMoved` 到达/被吞/被判跳变 的计数，
    /// 用于区分"鼠标事件没到"与"到了但被守卫丢掉"（见 `cam:` 日志行）。
    cursor_evt_count: u64,
    cursor_evt_eaten: u64,
    cursor_evt_teleport: u64,
    cursor_evt_last: (f64, f64),
    /// 1x1 全透明光标（见 `resumed` 里的注释）。捕获时用它：屏幕上不显示箭头，
    /// 同时让 winit 认为光标"未隐藏"，从而把指针限制在窗口内而不是钉成 1x1。
    blank_cursor: Option<winit::window::CustomCursor>,
    /// 鼠标左键是否按住（拖拽轨道旋转）
    dragging: bool,
    /// 鼠标右键是否按住（飞行模式拖拽转视角）
    right_dragging: bool,
    /// 开镜瞄准（右键按住；第一人称 FPS：准星收窄 + 枪模居中 + FOV 缩小）
    ads_active: bool,
    /// 开镜混合度 0..1（腰射→开镜 0.2s 指数平滑；驱动枪模锚点插值）
    ads_blend: f32,
    /// 最近一次开火时刻（anim_clock）。2026-09-01 起枪模后坐改由 `gun_sway.kick`
    /// 的连续指数包络驱动（旧写法直接按此值算 `(1-t)²` 抛物线 + 0.30 s 硬截止，
    /// 连发时每个周期都有位置阶跃）。本字段现在只被写入，留作击发时刻的观测点
    // 只写不读，留给后坐/射速诊断挂点（2026-09-26 编译器复查：两种 profile 都不再为此告警
    // —— `Debug` 派生算一次读取 ⇒ 压制已删；**字段本身保留**，三处写入的语义不动）
    last_shot_at: f32,
    /// 第一人称枪摆动状态（步态相位 + 低通速度 + 后坐包络），每帧在 `update()`
    /// 里按 delta_time 积分，`fp_gun_matrix()` 只读 → 摆动与帧率无关且逐帧连续
    gun_sway: GunSway,
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
    /// 后台（窗口失焦）帧率上限（0 = 不额外限制；`RV3D_BG_FPS`，玩家路径由启动器设为 20）
    /// —— 失焦后仍全速渲染会把 GPU 占满、把桌面合成挤到没余量（见 `effective_frame_cap`）
    bg_cap_fps: f32,
    /// 诊断用：强制按「未聚焦」处理（`RV3D_FORCE_UNFOCUSED=1`，仅用于在
    /// 无人值守的探针里复现"后台跑游戏"的负载，正常路径恒为 false）
    force_unfocused: bool,
    /// 上一次打印过的有效帧率上限（变化时才打日志，避免每帧刷屏）
    logged_cap_fps: f32,
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
    /// 上一次**打过日志**的抓取模式（0=未打过 / 1=confined / 2=locked）。
    ///
    /// 🔴 为什么需要它（2026-10-11，Android 实测）：`sync_cursor` 的守卫是
    /// `want && !self.cursor_captured`，而 `cursor_captured = grabbed || locked` ——
    /// **抓取失败时它是 false，下一帧守卫又成立 ⇒ 每帧重试 + 每帧打日志**。
    /// Android 上没有光标，`set_cursor_grab` 必然失败 ⇒ 实测 **962 行日志里 737 行
    /// 是这一条**（77%），把真正的性能日志挤掉。重试本身是对的（窗口可能变得可抓），
    /// **每帧打日志才是错的** ⇒ 只在模式**变化**时打。
    cursor_grab_logged: u8,
    /// 捕获模式是否为系统级 Locked（raw 相对增量驱动视角）；
    /// false = 回退 Confined/无 grab，走绝对位置路径（WSLg/Xwayland 实测：
    /// 真实物理鼠标只产生 CursorMoved 绝对位置，不产生 XI_RawMotion raw 事件）
    cursor_locked: bool,
    /// 锁定态观察窗：`(进入锁定态的时刻, 当时的 cursor_evt_count)`。
    /// `None` = 不在观察中（非锁定态，或已有结论）。判定见 `LockObservation`。
    ///
    /// 为什么需要它：`set_cursor_grab(Locked)` 的 `Ok` **不等于锁生效**（Wayland 下
    /// 指针尚未 enter 时它什么都没做也返回 `Ok`，且 winit 忽略合成器的确认事件），
    /// 而失败的症状与"鼠标坏了"完全一样（零 raw 事件 ⇒ 视角不动）。
    /// 唯一可信的判据是**行为证据**，所以必须真的去数一数增量有没有来。
    lock_observe: Option<(Instant, u64)>,
    /// 假成功时是否已补抓过一次（只补一次，见 `sync_cursor` 的 NoMotion 分支）
    lock_retried: bool,
    /// 假成功告警是否已发过（一次性，不刷屏）
    lock_fake_warned: bool,
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
    /// Android 生命周期：true = 已 Suspended（surface 被系统收回），暂停渲染。
    /// 见 docs/HANDOFF-mobile.md 4.2；桌面恒为 false（不改变桌面行为）。
    suspended: bool,
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
    /// 上一帧的**真实帧间隔**（微秒，未夹取）。
    /// 为什么要存成字段：算 dt 的地方与 `render()` 是**两个方法**，而帧间隔只有前者知道。
    /// 用途仅限性能日志的 `dt_us` 列（口径见 `perf_log.rs`：帧率本身按窗口算，不靠这个值）。
    frame_dt_us: u64,
    /// 上次打印 `cull-diag:` 的时刻（`RV3D_CULL_DIAG=1`，默认关掉时这个字段只被读一次/秒）
    last_cull_diag: std::time::Instant,
    /// 上次打印 `gundiag:` 的时刻（`RV3D_GUN_DIAG=1`，同上）
    last_gun_diag: std::time::Instant,
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
    /// `RV3D_CAM` 的调试机位：`(位置, yaw, pitch, yaw 角速度 rad/s)`。
    /// 第 4 项非 0 时相机绕 yaw **匀速旋转** —— 这是运动向量的**解析验证**专用模式，
    /// 理由见 `build.rs` 里 MV 那段的注释（旋转的屏幕位移与深度无关 ⇒ 全屏均匀、可算）。
    cam_override: Option<(glam::Vec3, f32, f32, f32)>,
    /// 旋转机位的累计转角（rad）。只在 `cam_override` 带角速度时使用。
    cam_spin: f32,
    /// **当前已上传到 GPU 的枪模对应的武器 key**（`None` = 未上传 / 已被清空）。
    ///
    /// 存在的理由（2026-10-13）：`set_first_person_gun_mesh` 原本**每帧**都被调用，
    /// 而它会把整把枪（本仓 AK-12M = **63283 顶点 × 32B = 2.0MB** + 70479 索引 × 4B = 282KB）
    /// 拷进映射缓冲；上游 `first_person_gun_mesh()` 还先返回一份**带所有权**的副本
    /// ⇒ 每帧约 **4.6MB 的无效搬运**。
    /// 而枪模是**按武器 key 缓存**的、姿态走 `fp_gun_pre` 矩阵 ⇒ 同一把枪下逐帧完全相同。
    /// ⇒ 只在 key 变化时重建+上传。切到"不显示枪"时清空它，保证切回来会重传。
    gun_mesh_uploaded_key: Option<String>,
    /// **静态障碍 marker 的缓存** + 它对应的 `Game::map_generation()`。
    ///
    /// 存在的理由（2026-10-13）：`[prep-diag]` 实测准备段里 marker 一项
    /// （`render_geometry()` 逐件滤 `.filter(shape != None)` + `WorldMarker::for_obstacle()`
    /// 构造矩阵 + collect）要 **~600µs**，而**城市场景障碍是静态的** ——
    /// 只有 `apply_level()` 才会换一批。
    /// `map_generation()` 正是那个"关卡换没换"的判据（`session.rs` 里 `apply_level` 自增）。
    ///
    /// ⚠️ 缓存的是**已构造好的 `WorldMarker`**（贵的是逐件的矩阵构造，不是那次拷贝）；
    /// 使用时 `clone()` 一份再 extend 据点/弹孔 —— 拷贝是 memcpy，比重新构造便宜得多。
    obstacle_markers_cache: Vec<engine::renderer::WorldMarker>,
    /// 缓存对应的关卡代号；`None` = 还没建过（首帧必然建一次）。
    obstacle_markers_gen: Option<u64>,
    /// 命令输入缓冲（当前只接受数字，回车切换武器）
    command_buf: String,
    /// 当前武器枪模缓存（构建含光照烘焙，切枪时才重建；帧内只做视空间变换）
    gun_mesh_cache: Option<(String, crate::engine::guns::GunMesh)>,
    /// 导入的 GLB 枪模缓存（按武器 key：assets/guns/{key}.glb → 顶点；无则该武器回退程序化枪模）
    gun_glbs: std::collections::HashMap<String, Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)>>,
    /// GLB 道具网格套件（懒加载一次；None = 还没尝试加载）。摆放列表在 LevelMap::props 上，
    /// 但那份列表只存下标，网格本体归这里——重载地图不必重新解析 24 个 GLB。
    prop_set: Option<engine::props::PropSet>,
    /// 上次上传道具几何时的地图代号；哨兵值保证首帧一定上传一次。
    prop_map_gen: u64,
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
    fn settings_click(&mut self, mx: f32, my: f32) {
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

    /// 把 WASD 按键状态转发给游戏（FPS 玩家移动）
    /// ESC 的语义（2026-10-11 从 `KeyboardInput` 分支里抽出）：
    /// 设置面板打开 → 关面板；ESC 菜单打开 → 关菜单；否则 → 打开 ESC 菜单并**立即**释放鼠标捕获。
    ///
    /// 抽出来是为了让 **Android 返回键**复用同一条路径 —— 手机上返回键必须等价于 ESC，
    /// 否则玩家退不出游戏（见下面 `NativeKeyCode::Android(4)` 那条 arm）。
    /// ⚠️ 「立即释放捕获」那段不能省：等下一帧 `sync_cursor` 的话，用户立刻移动点击时
    /// `last_cursor` 仍是捕获中心 ⇒ 菜单选项命中错位。
    fn toggle_escape_menu(&mut self) {
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
    fn release_cursor_immediately(&mut self, reason: &str) {
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
    fn reset_session_transients(&mut self) {
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

    fn sync_game_movement(&mut self) {
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
    fn begin_exit(&mut self, event_loop: &ActiveEventLoop) {
        self.game.send_leave();
        if let Some(pl) = self.perf_log.as_mut() {
            pl.finish();
            self.perf_log = None;
        }
        self.running = false;
        event_loop.exit();
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

}

/// Android 触控输入（2026-10-09）。
///
/// 为什么完全绕开鼠标那套：铁律 C 的光标捕获协议在手机上**前提不成立**
/// （没有光标、没有悬停、没有右键）。这里直接驱动引擎内部的三个状态：
/// `key_state`（移动）/ `camera.look`（视角）/ `fire_requested`（开火）。
///

impl ApplicationHandler for GameApp {
    /// 应用恢复/启动时创建窗口和渲染器
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.handle_resumed(event_loop);
    }

    /// 设备级事件：系统相对鼠标增量（XInput2 raw motion，与光标位置无关）驱动视角。
    /// 捕获态唯一视角输入源：raw 增量是设备原始计数，与指针位置/grab 状态无关，
    /// 不依赖窗口内指针位置，也不产生"每帧回中 warp → 回声"反馈环。
    /// 用户事件（菜单点击退出用代理发送）：收到即退出事件循环
    fn user_event(&mut self, event_loop: &ActiveEventLoop, _event: ()) {
        log::info!("input: 收到退出事件，退出游戏");
        // 🔴 走统一收尾（原来这里只 `pl.finish()`，**漏了 `send_leave`**）
        self.begin_exit(event_loop);
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
    /// Android 生命周期：surface 被系统收回（切后台/锁屏）⇒ 停止渲染。
    /// 见 docs/HANDOFF-mobile.md 4.2。
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("生命周期：Suspended（surface 被系统收回），暂停渲染");
        self.suspended = true;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        self.handle_window_event(event_loop, _window_id, event);
    }

    /// 事件队列空闲时调用（主循环体）
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Suspended（Android 切后台）时不渲染：surface 已不可用。
        if !self.running || self.window.is_none() || self.suspended {
            return;
        }

        // 🔬 **定时正常退出**（`RV3D_EXIT_AFTER=<秒>`，2026-10-18）：
        // 存在理由：验证「独显退出崩溃」必须走**正常退出路径**（`begin_exit` →
        // `event_loop.exit()` → 正常 drop 掉 Vulkan 对象）。用 `timeout` / `kill` 是 SIGTERM，
        // **根本不走销毁路径** ⇒ 验不到那个 bug。而点窗口 X 需要注入输入（违反鼠标安全协议）。
        // 与 `RV3D_SHOT_AT` 同类，仅诊断用；计时起点锚定在**第一帧之后**（加载期不算）。
        if let Some(secs) = crate::syscfg::parse::<f32>("RV3D_EXIT_AFTER") {
            static EXIT_T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let t0 = *EXIT_T0.get_or_init(Instant::now);
            if t0.elapsed().as_secs_f32() >= secs {
                log::info!("RV3D_EXIT_AFTER={secs:.1}s 已到 ⇒ 走正常退出路径（销毁 Vulkan 对象）");
                self.begin_exit(event_loop);
                return;
            }
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

        // 自动截图（RV3D_SHOT_AT）：**渲染之后**取，保证截到的是本帧真实画面。
        // 计时起点在第一次渲染时锚定 —— 不是在启动时：引擎起来到第一帧之间要加载
        // 着色器/城市/GLB，那段时间截图只会得到一张黑图或半成品，对取证没有意义。
        if !self.shot_pending.is_empty() {
            let t0 = *self.shot_t0.get_or_insert_with(Instant::now);
            let due = take_due_shots(&mut self.shot_pending, t0.elapsed().as_secs_f32());
            for _ in 0..due {
                self.capture_screenshot();
            }
            if self.shot_pending.is_empty() {
                log::info!("自动截图：全部时刻已到，停止检查");
            }
        }
        self.last_update_us = update_us;
        self.last_cycle_us = cycle_start.elapsed().as_micros() as u64;
        // 采集模式帧率上限（RV3D_LLM=1 时 90FPS 封顶）：大幅降低 GPU 负载，
        // 避免与 llama-server 长时间同卡共存导致 VK_ERROR_DEVICE_LOST（2026-08-23）
        //
        // 2026-09-27：这里改成"有效上限" —— 窗口失焦时再夹上后台上限（RV3D_BG_FPS）。
        // 实测失焦后仍全速渲染会把 GPU 占满；桌面合成（dwm）拿不到时间片，用户看到的是
        // "游戏一开整机就卡"。引擎默认 bg=0（不改变任何既有测量口径），玩家路径由
        // SteelFront.bat 设 20。判据：pure fn effective_frame_cap 的四条测试 + 探针实测。
        let focused_now = self.focused && !self.force_unfocused;
        let cap_fps = effective_frame_cap(self.llm_cap_fps, self.bg_cap_fps, focused_now);
        if (cap_fps - self.logged_cap_fps).abs() > 0.5 {
            log::info!(
                "frame-cap: {:.0} fps (focused={} fg={:.0} bg={:.0})",
                cap_fps, focused_now, self.llm_cap_fps, self.bg_cap_fps
            );
            self.logged_cap_fps = cap_fps;
        }
        if cap_fps > 0.0 {
            let used = cycle_start.elapsed().as_secs_f32();
            let target = 1.0 / cap_fps;
            if used < target {
                std::thread::sleep(std::time::Duration::from_secs_f32(target - used));
            }
        }
    }
}

/// 程序入口点
#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

/// 非 Android 平台上的占位类型，仅为让 `run_steel_front` 的签名跨平台统一。
#[cfg(not(target_os = "android"))]
type AndroidApp = ();

#[cfg(target_os = "android")]
mod android_log;

/// 引擎主循环（桌面与 Android 共用）。
///
/// Android 上由 `android_main` 调用并传入 `AndroidApp`（事件循环所有权反转，
/// 见 docs/HANDOFF-mobile.md 4.2）；桌面上由 `main` 以 `None` 调用。
fn run_steel_front(android_app: Option<AndroidApp>) {
    // 初始化日志系统
    // Android：env_logger 写 stderr，系统不看 ⇒ 改用 logcat（见 android_log.rs）。
    #[cfg(target_os = "android")]
    crate::android_log::init();
    #[cfg(not(target_os = "android"))]
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // RV3D_ASSETS_FROM_MEMORY=1：把 assets/ 预载进内存并安装为全局资产来源，
    // 在桌面上验证"资产不来自文件系统"这条路径（为 Android AAssetManager 铺路，
    // 见 docs/HANDOFF-mobile.md 4.3）。
    // Android：把资产读取接到 AAssetManager（APK 内的 assets/）。
    #[cfg(target_os = "android")]
    if let Some(app) = &android_app {
        crate::engine::asset_source::install(Box::new(
            crate::engine::asset_source::AndroidAssetSource::new(app.asset_manager()),
        ));
        log::info!("asset_source: 已安装 AAssetManager 来源");
    }

    if env_truthy("RV3D_ASSETS_FROM_MEMORY") {
        let src = crate::engine::asset_source::MemSource::preload_tree("assets");
        log::info!(
            "RV3D_ASSETS_FROM_MEMORY=1：已预载 assets/ 到内存（{} 项）",
            src.len()
        );
        crate::engine::asset_source::install(Box::new(src));
    }

    // 默认大战场：红 128 vs 蓝 127+玩家（=128v128，2026-08-22 要求海量 NPC 模拟真人压力）；
    // RV3D_STRESS_AI=N 自定义，=0 恢复传统波次模式
    if !crate::syscfg::is_set("RV3D_STRESS_AI") {
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

    // ---- 后端选择：X11 / Wayland ----
    //
    // 为什么这个开关是**必需**的（2026-09-28，Linux 适配）：
    // WSLg（WSL2 + Wayland/Weston）的指针约束/相对指针协议支持不完整 —— 捕获后光标不隐藏、
    // 视角不动，且右键拖动会在原生层静默崩溃（无 panic 日志）。所以旧代码在 WSL 下强制 X11。
    // 但那条判据（`/proc/version` 含 "microsoft"）**只覆盖 WSL**，原生 Linux 会话走不到，
    // 于是用户**没有任何手段退回 XWayland** —— 而 winit 0.30 已经删掉了
    // `WINIT_UNIX_BACKEND` 环境变量（v0.29 changelog），不重建事件循环就没有第二条路。
    //
    // 为什么原生 Linux 更需要这个退路（不是"以防万一"，是**已知的功能差异**）：
    // - **X11/XWayland**：`set_cursor_grab(Locked)` **恒返回 `Err(NotSupported)`**
    //   （winit `x11/window.rs`），于是 `cursor_grab_plan` 退到 `Confined` + 绝对位置路径；
    //   而 X11 的 `set_cursor_position` 走 `XWarpPointer` **真的生效**，
    //   所以「回中 + 用 `CursorMoved` 增量算视角」这条路是**自洽可用的**。
    // - **Wayland**：`Locked` 返回 `Ok`，但 winit 的 `apply_on_pointer` **只对已 enter 的指针
    //   生效** —— 指针还没进窗口时它**什么都没做也返回 Ok**；而且 winit 完全忽略合成器的
    //   确认事件（`ZwpLockedPointerV1` 的 Dispatch 函数体是空的）⇒ 应用层**无法感知**锁
    //   到底有没有生效。绝对位置那条路在 Wayland 上也不通：`set_cursor_position` 只在
    //   已 `Locked` 时才成功 ⇒ **Confined 下指针撞到窗口边就再也转不动**。
    //   ⇒ Wayland 下唯一可用的是 Locked + 相对指针，而它的失败**是静默的**。
    //
    // ⇒ 语义：`RV3D_BACKEND` = `x11` / `wayland` / 未设（自动：WSL 下选 x11，其余不强制）。
    // 判据 = `backend_choice_x11_is_reachable_on_native_linux`（它钉住"非 WSL 的原生 Linux
    // 也必须能选到 x11"，而那正是旧逻辑唯一做不到的事）。
    #[cfg(target_os = "linux")]
    let backend_choice = {
        let is_wsl = std::fs::read_to_string("/proc/version")
            .map(|v| v.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false);
        // WSLg 的自动判定保留旧条件（WSL **且** 有 Wayland 会话）：纯 X11 的 WSL 本来就走 X11
        let auto_x11 = is_wsl && std::env::var_os("WAYLAND_DISPLAY").is_some();
        let explicit = crate::syscfg::cfg("RV3D_BACKEND");
        let choice = backend_choice(explicit.as_deref(), auto_x11);
        match choice {
            BackendChoice::X11 => log::info!(
                "input: 强制 X11 后端（Xwayland + XInput2 raw motion）—— 来源：{}",
                if explicit.is_some() {
                    "RV3D_BACKEND"
                } else {
                    "WSLg 自动判定"
                }
            ),
            BackendChoice::Auto => {
                log::info!("input: 后端不强制（RV3D_BACKEND 未设或 =wayland），交给 winit 自动选择")
            }
        }
        choice
    };
    // 创建事件循环（BackendChoice::X11 时经 with_x11() 强制 Xwayland）
    let event_loop = {
        let mut builder = EventLoop::builder();
        // Android：把系统传入的 AndroidApp 交给 winit（事件循环所有权反转，
        // 见 docs/HANDOFF-mobile.md 4.2）。
        #[cfg(target_os = "android")]
        if let Some(app) = android_app.clone() {
            use winit::platform::android::EventLoopBuilderExtAndroid;
            builder.with_android_app(app);
        }
        #[cfg(not(target_os = "android"))]
        let _ = &android_app;
        #[cfg(target_os = "linux")]
        if backend_choice == BackendChoice::X11 {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }
        match builder.build() {
            Ok(el) => el,
            Err(e) => {
                log::error!("创建事件循环失败: {:?}", e);
                // 🔴 致命启动错误**必须非零退出**。原来这里是 `return`，而 `fn main`
                // 正常返回就是退出码 0 ⇒ 调用方（perf_run / 冒烟 / CI）看到的是"跑完了"，
                // 实际上一帧都没渲染。2026-10-03 实测踩到：在没有图形会话环境变量的 shell 里
                // （XDG_SESSION_TYPE=tty，缺 WAYLAND_DISPLAY/DISPLAY）报的就是这一条，
                // 而 perf_run.sh 只看到"游戏提前退出（code 0）"、当成一次正常结束。
                // 这就是教训 46 的形态：工具必须能说"我没跑成"。
                std::process::exit(1);
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
    // 远端实体插值渲染（本文件 2579 起的 `entity_state_at`）与断线自动重连（game.rs）**已接线**；
    // 未做：NAT 双进程真机验证、输入预测/回滚（见 net.rs 头部「已接线 / 未做」两行）。
    let net_role = crate::syscfg::cfg("RV3D_NET").unwrap_or_default();
    let net_addr =
        crate::syscfg::cfg("RV3D_NET_ADDR").unwrap_or_else(|| "127.0.0.1:27015".to_string());
    // NAT 中继（RV3D_NET_RDV=<host:port> + RV3D_NET_NAME=房间名）：
    // 服务器向中继注册；客户端查询房间名→公网地址直连（NAT 打洞第一步）
    let net_rdv = crate::syscfg::cfg("RV3D_NET_RDV");
    let net_name = crate::syscfg::cfg("RV3D_NET_NAME").unwrap_or_else(|| "steel".to_string());
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
                    let res = server.rdv_register(rdv, &net_name, port);
                    let (ok, msg) = rdv_register_report(rdv, &net_name, port, &res);
                    if ok {
                        log::info!("{msg}");
                    } else {
                        log::error!("{msg}");
                    }
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
        // 🔴 同上，而且这里还多一层误导：原来出错后**继续往下走**，
        // 打出"程序正常退出"并以 0 退出 —— 日志说"正常"、退出码说"成功"，
        // 而实际是异常终止。出错就既不许说正常，也不许退 0。
        std::process::exit(1);
    }

    // 🔴 致命错误可能是在事件循环**内部**记下的（那时只能 `event_loop.exit()`，
    // 而它会让 `run_app` 返回 `Ok`）⇒ 这里补上那次判断，否则同样是"日志报错、退出码说成功"。
    if let Some(why) = app.fatal.take() {
        log::error!("以错误退出：{why}");
        std::process::exit(1);
    }

    log::info!("程序正常退出");
}

/// 中继注册的结论文案：**成功与否由 `res` 决定**，返回 `(是否成功, 文案)`。
///
/// 🔴 存在理由（2026-09-26 复查）：调用点以前是 `let _ = server.rdv_register(...)` 紧接着
/// **无条件**打「已向中继注册房间…（等待玩家查询）」。注册失败时这条日志把「没跑成」写成了成功，
/// 而它是 NAT 打洞链路上唯一的现场证据：中继侧没有 REG 记录，客户端 `net::rdv_resolve`
/// 就只会等到 5s 超时，排查的人会一路去怀疑打洞逻辑与防火墙。
/// 判据 = 测试 `relay_registration_report_never_claims_success_on_failure`。
fn rdv_register_report(
    rdv: &str,
    name: &str,
    port: u16,
    res: &std::io::Result<()>,
) -> (bool, String) {
    match res {
        Ok(()) => (
            true,
            format!("net: 已向中继 {rdv} 注册房间 {name}（端口 {port}，等待玩家查询）"),
        ),
        Err(e) => (
            false,
            format!(
                "net: 向中继 {rdv} 注册房间 {name} 失败: {e} —— 中继侧没有这条记录，客户端按房间名解析只会超时（先确认中继地址可达与出站 UDP 未被拦）"
            ),
        ),
    }
}

#[cfg(test)]
mod tests {

    /// 按**字符**数截取窗口（源码里有中文，按字节切会踩非 char 边界 panic）。
    fn src_window(src: &str, start: usize, max_chars: usize) -> &str {
        let s = &src[start..];
        let end = s.char_indices().nth(max_chars).map(|(i, _)| i).unwrap_or(s.len());
        &s[..end]
    }

    /// 🔴 **释放光标必须恢复系统箭头**（2026-10-20 第 14 批复核第 3 条）：
    /// Windows 捕获路径用的是 1×1 全透明自定义光标，释放只 `set_cursor_visible(true)`
    /// ⇒ 鼠标能动但**完全隐形**。所有释放点必须共用同一个 helper。
    #[test]
    fn cursor_release_restores_default_icon_and_is_shared() {
        // 🔴 Windows 检出是 CRLF（本机 `core.autocrlf=true`）⇒ 先归一化行尾，
        // 否则下面搜 `"\n    }\n"` 永远搜不到（实测 2026-10-26）。
        // 🔴 2026-11-10：`sync_cursor` 已搬到 `src/app_window.rs`（lib.rs 拆分），
        // 而这条判据原本只扫 `lib.rs` ⇒ 搬家后它会把"代码动过"误报成"释放点丢了"。
        // 判据要跟着代码走：把两处拼起来扫，并且**两条都必须非空**（防止 include 失效
        // 导致"扫到 0 个"却仍然通过 —— 那是恒真断言）。
        // 🔴 2026-11-13：`window_event` 的实现体搬到 `app_events.rs` 后，释放光标的调用点也有
        // 一部分跟着过去了 ⇒ 扫描面加上它（三份都必须非空，防 include 失效时变成恒真）。
        let raw = format!(
            "{}\n{}\n{}",
            include_str!("lib.rs"),
            include_str!("app_window.rs"),
            include_str!("app_events.rs")
        );
        let src = raw.replace("\r\n", "\n");
        let src = src.as_str();
        let start = src.find("fn release_cursor_immediately").expect("必须有统一释放实现");
        let end = start + src[start..].find("\n    }\n").expect("helper 必须有函数结尾");
        let body = &src[start..end];
        assert!(
            body.contains("CursorIcon::Default"),
            "释放光标时必须恢复 CursorIcon::Default（否则 Windows 上指针隐形）"
        );
        let calls = src.matches("self.release_cursor_immediately(").count();
        assert!(
            calls >= 4,
            "释放点必须共用该 helper（定义 + sync_cursor + ESC 菜单 + 菜单键 = 至少 4 处），实际 {calls}"
        );
    }

    /// 🔴 **ESC 菜单用其它键关闭后必须 return**（2026-10-20 第 14 批复核第 7 条）：
    /// 否则关闭菜单后 `in_gameplay` 立刻变 true，同一按键继续穿透：
    /// 按 R 关菜单会重开一局、按 X 会吃掉一个医疗包、按 G 会当场扔雷。
    #[test]
    fn esc_menu_consumes_the_key_that_closes_it() {
        let src = include_str!("lib.rs");
        // 从 ESC 菜单那一块起找（文件里还有一处 StartMenu 的 "非 Escape" 判断，别抓错）
        let base = src
            .find("// ESC 菜单导航：Tab 切换选项")
            .expect("ESC 菜单导航块必须存在");
        let i = base
            + src[base..]
                .find("if pressed && key_code != KeyCode::Escape {")
                .expect("ESC 菜单兜底分支必须存在");
        let seg = src_window(&src, i, 1200);
        assert!(
            seg.contains("return;"),
            "关闭 ESC 菜单后必须 return，不能把同一按键继续传给玩法层"
        );
    }

    /// 🔴 **重开/切关必须复位 GameApp 层瞬时状态**（2026-10-20 第 14 批复核第 2 条）：
    /// `Game::start_run` 管不到按键锁存/开火意图/尸体快照/伤害飘字/枪摆动。
    #[test]
    fn restart_paths_reset_gameapp_transients() {
        // 🔴 2026-11-13：`window_event` 的实现体已搬到 `src/app_events.rs`，其中也有复位调用点
        // ⇒ 只扫 lib.rs 会把"搬家"误报成"少了一个入口"。两个文件都扫。
        let joined = format!("{}\n{}", include_str!("lib.rs"), include_str!("app_events.rs"));
        let src = joined.as_str();
        let calls = src.matches(".reset_session_transients()").count();
        assert!(calls >= 3, "重开与切关路径都必须调用复位（方法定义 + 3 个入口），实际 {calls}");
        let i = src.find("fn reset_session_transients").expect("必须有复位方法");
        let body = src_window(&src, i, 1500);
        for needle in [
            "key_state.reset()",
            "fire_requested = false",
            "command_open = false",
            "corpses.clear()",
            "last_npc_snapshot.clear()",
            "hit_damage_popups.clear()",
            "gun_sway.reset_for_new_run()",
        ] {
            assert!(body.contains(needle), "reset_session_transients 缺少 {needle}");
        }
    }
    use super::*;

    /// 🔴 **`GameApp` 里 `renderer` 必须声明在 `window` 前面。**
    ///
    /// 存在理由（2026-10-16 真机实测，一次 300 秒跑法留下核心转储）：
    /// **Rust 按【声明顺序】析构结构体字段。** 原来是 `window` 在前 ⇒ 退出时
    /// **winit 窗口（wayland 的 `wl_surface`/`wl_display` 代理）先销毁，
    /// `Renderer` 后销毁** ⇒ 驱动销毁 Vulkan 对象（交换链）时回头去碰**已死的代理**
    /// ⇒ **`SIGSEGV`（`SEGV_MAPERR`）**。核心转储的栈：
    /// ```text
    /// #0..#3  libwayland-client.so.0   wl_proxy_marshal_flags
    /// #4..#7  libnvidia-glcore.so.615.78.08
    /// #8      <Renderer as Drop>::drop
    /// #9      drop_in_place<Renderer>
    /// #10     drop_in_place<GameApp>
    /// #11     run_steel_front
    /// ```
    ///
    /// ⚠️ **它难办的地方在于日志是干净的**：引擎最后三行是
    /// 「窗口关闭请求 / 性能日志已保存 / **程序正常退出**」——
    /// 主循环干净退出，崩在 `main` 返回**之后** ⇒
    /// **只读日志的门禁对它看不见**（当时 `smoke_linux.sh` 就把这次判成了 `ALL-OK`，
    /// 见同批修的 `--exit-status`）。
    ///
    /// ⇒ 这条判据只能**看声明顺序**：这件事没有便宜的运行时观测点，
    ///   而真机退出恰恰是冒烟最容易漏掉的一段。
    #[test]
    fn renderer_field_drops_before_the_window() {
        let src = include_str!("lib.rs");
        let start = src
            .find("struct GameApp {")
            .expect("找不到 struct GameApp —— 判据的前提不成立");
        let body = &src[start..];
        let end = body.find('}').expect("struct GameApp 没有闭合");
        let body = &body[..end];

        let renderer_at = body
            .find("renderer: Option<Renderer>")
            .expect("GameApp 里没有 renderer 字段 —— 判据的前提不成立");
        let window_at = body
            .find("window: Option<Window>")
            .expect("GameApp 里没有 window 字段 —— 判据的前提不成立");
        assert!(
            renderer_at < window_at,
            "`window` 声明在 `renderer` 前面 ⇒ 退出时**窗口先销毁、交换链后销毁**\n\
             ⇒ 驱动销毁 Vulkan 对象时去碰已死的 wayland 代理 ⇒ SIGSEGV。\n\
             而那次崩溃**日志是干净的**（「程序正常退出」之后就没了），只读日志的门禁抓不到它。"
        );
    }

    /// 🔴 **判据：`main()` 里的致命错误路径不许以退出码 0 结束。**
    ///
    /// 2026-10-03 实测踩到：在一个**没有图形会话环境变量**的 shell 里
    /// （`XDG_SESSION_TYPE=tty`，缺 `WAYLAND_DISPLAY`/`DISPLAY`），引擎报
    /// "创建事件循环失败" 然后 `return` ⇒ **退出码 0**。
    /// `perf_run.sh` 只看到"游戏提前退出（code 0）"，无法把「一帧都没渲染」
    /// 和「正常结束」区分开 —— 教训 46 的形态：工具必须能说"我没跑成"。
    /// 同一晚还发现 `run_app` 出错后会继续往下打出"程序正常退出"（日志说正常、
    /// 退出码说成功，而实际是异常终止），一并钉住。
    ///
    /// ⚠️ 这是**源码扫描**型判据，两个坑都要防：
    /// 1. **必须先证明它真的扫到了东西** —— 文件被搬走/改名/那段被重写时，
    ///    `find` 返回 `None` 会让断言静默恒真；
    /// 2. **必须先去注释** —— 修这个 bug 时写的注释里就引用了「程序正常退出」，
    ///    不去注释的话扫描会先撞上注释、让判据自行满足（本仓 `no_unbounded_wait_on_vulkan_calls`
    ///    同样先做 `is_comment` 过滤）。
    #[test]
    fn fatal_startup_paths_never_exit_zero() {
        // 🔴 2026-11-12：`resumed` 的实现体已搬到 `src/app_events.rs`（"创建窗口失败"/"渲染器
        // 初始化失败"两个锚点跟着搬走了），而这条判据原本只扫 `lib.rs` ⇒ 会报"检查失效"。
        // 判据要跟着代码走：把两个文件拼起来扫。
        //
        // ⚠️ **必须各自先切掉测试模块再拼**：`split("#[cfg(test)]")` 在拼接后的整串上做时，
        // 会在 **lib.rs 的测试模块**处截断 ⇒ app_events.rs 整份被丢掉、锚点又找不到了
        // （我第一次就是这么写的，判据立刻红）。这也是"拼接式扫描"的一个通用坑。
        let lib_prod = include_str!("lib.rs").split("#[cfg(test)]").next().unwrap_or("");
        let events = include_str!("app_events.rs");
        let joined = format!("{lib_prod}\n{events}");
        let prod = joined.as_str();
        assert!(
            prod.contains("fn handle_resumed") && prod.contains("impl ApplicationHandler for GameApp"),
            "扫描面不完整：lib.rs 生产段与 app_events.rs 都必须在内"
        );
        // 去注释：行首是 // 的一律丢掉
        let code: String = prod
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        let anchors = ["创建事件循环失败", "run_app(&mut app)"];
        for a in anchors {
            assert!(
                code.contains(a),
                "检查失效：去注释后扫不到锚点 {a:?}（源码被改名/搬走了？）"
            );
        }

        // 🔴 **必须把两半切开**：第一版写成 `code[i..].contains("process::exit")`，
        // 扫的是"锚点之后的全部内容" ⇒ **第二处修复把第一处的断言喂饱了**，
        // 把事件循环那条改回 `return` 时测试照样绿（反证实测发现）。
        // 这正是本仓教训 14 的形态：恒真的断言等于没写。
        let split = code.find("run_app(&mut app)").unwrap();
        let (startup, running) = code.split_at(split);

        // 1) 事件循环创建失败之后必须出现 process::exit（只在**这一半**里找）
        let i = startup.find("创建事件循环失败").unwrap();
        assert!(
            startup[i..].contains("process::exit"),
            "事件循环创建失败是致命错误，不能只 return（`fn main` 正常返回 = 退出码 0）"
        );

        // 2) run_app 出错分支也必须有 process::exit，且**必须早于**「程序正常退出」那句
        let exit_at = running.find("process::exit");
        let normal_at = running.find("程序正常退出");
        assert!(
            normal_at.is_some(),
            "检查失效：去注释后扫不到「程序正常退出」那句，说明扫描窗口不对"
        );
        assert!(
            exit_at.is_some(),
            "run_app 出错是致命错误，不能退 0"
        );
        assert!(
            exit_at.unwrap() < normal_at.unwrap(),
            "run_app 出错后必须先退出，不能掉到「程序正常退出」那句（日志说正常、退出码说成功，\
             而实际是异常终止）"
        );

        // 3) **循环内部**的致命错误也必须被记进 `fatal`。
        //    这两处只能 `event_loop.exit()`，而它让 `run_app` 返回 `Ok` ⇒ 上面那条
        //    `run_app` 检查抓不到它们。2026-10-03 实测：解包后跑错目录（缺 assets/mesh.spv）
        //    报的就是「渲染器初始化失败」+「程序正常退出」+ 退出码 0。
        for anchor in ["创建窗口失败", "渲染器初始化失败"] {
            let i = code.find(anchor).unwrap_or_else(|| {
                panic!("检查失效：去注释后扫不到锚点 {anchor:?}（源码被改名/搬走了？）")
            });
            let tail = &code[i..(i + 300).min(code.len())];
            assert!(
                tail.contains("self.fatal = Some"),
                "{anchor} 是致命错误，必须记进 `self.fatal` —— 否则 event_loop.exit() 会让 \
                 run_app 返回 Ok，引擎打出「程序正常退出」并以 0 退出"
            );
        }

        // 4) `fatal` 必须在循环结束后被消费掉（只记不查等于没记）
        assert!(
            running.contains("app.fatal"),
            "`fatal` 记了却没人查 —— 循环结束后必须据此非零退出"
        );
    }

    /// 🔴 判据：`RV3D_SHOT_AT` 的解析与"取走到点"两条（2026-10-03）。
    ///
    /// 加这个开关的理由是**平台不对称**：Windows 侧的 `run_resize_probe.ps1` 能用
    /// `PostMessage` 发 F12 截图，而 Linux 上唯一的截图触发就是 F12 按键，
    /// Wayland 下注入按键要抢焦点，违反铁律 C 的鼠标安全协议
    /// ⇒ Linux 的探针只能停在"没有画面取证"。给一个不需要输入的触发把这条补平。
    ///
    /// 两条各钉一个方向，第 3 条是真正要命的那个：
    /// 1. 解析要**去重升序**并丢弃非法项（取证开关不该因为写错一个数就让整局起不来）；
    /// 2. 非正数（0 / 负数）必须丢 —— `0` 会让 `retain(t > elapsed)` 在 elapsed=0 时
    ///    把它留下、之后又永远取不走，变成每帧都截一张；
    /// 3. 🔴 **一帧跨过多个时刻时不能漏**：低帧率或长卡顿下，某一帧的 elapsed 可能
    ///    同时越过 5s 和 15s。用"判等"实现会永久漏掉被跨过的那个 ——
    ///    而截图是取证用的，漏一张就少一份证据，**且不会报错**。
    #[test]
    fn shot_at_parsing_and_due_taking_are_falsifiable() {
        // 1) 去重升序 + 丢弃非法项
        assert_eq!(parse_shot_at("15,5,5,abc,-2,0"), vec![5.0, 15.0]);
        assert_eq!(parse_shot_at("  3.5 , 1 "), vec![1.0, 3.5]);
        assert!(parse_shot_at("").is_empty());
        assert!(parse_shot_at("nope").is_empty());
        assert!(parse_shot_at("NaN,inf,-1,0").is_empty(), "NaN/inf/非正数都要丢");

        // 2) 只取走"已到点"的，其余原样保留
        let mut p = parse_shot_at("5,10,15");
        assert_eq!(take_due_shots(&mut p, 4.9), 0);
        assert_eq!(p, vec![5.0, 10.0, 15.0]);
        assert_eq!(take_due_shots(&mut p, 5.0), 1, "到点即取（含等于）");
        assert_eq!(p, vec![10.0, 15.0]);

        // 3) **一帧跨过两个时刻**：必须一次取走两个，不能只取一个
        assert_eq!(take_due_shots(&mut p, 12.0), 1);
        assert_eq!(p, vec![15.0]);
        let mut q = parse_shot_at("1,2,3");
        assert_eq!(take_due_shots(&mut q, 100.0), 3, "跨过全部时一次全取走");
        assert!(q.is_empty());
        assert_eq!(take_due_shots(&mut q, 100.0), 0, "取空之后不再重复计数");
    }

    /// 🔴 判据：后台帧率上限（2026-09-27「游戏一开整机就卡」）。
    ///
    /// 四条各钉一个方向，缺一条就会出现"限了但限错"：
    /// 1. 默认（两侧都 0）必须是**不限制** —— 压测/采集路径逐字节不变；
    /// 2. 未聚焦且设了 bg 时必须被夹住（这是修的那个 bug）；
    /// 3. 聚焦时 bg **不许**生效（否则玩家自己玩的时候也被限）；
    /// 4. bg 只能**收紧**不能放宽（fg=30、bg=120 时仍取 30）。
    #[test]
    fn frame_cap_is_disabled_by_default_and_only_tightens() {
        // 1) 默认不限制
        assert_eq!(effective_frame_cap(0.0, 0.0, true), 0.0);
        assert_eq!(effective_frame_cap(0.0, 0.0, false), 0.0);
        // 2) 未聚焦：bg 夹住前台上限（含 fg=0 即"前台不限"的两侧组合）
        assert_eq!(effective_frame_cap(300.0, 20.0, false), 20.0);
        assert_eq!(effective_frame_cap(0.0, 20.0, false), 20.0);
        // 3) 聚焦时 bg 不生效
        assert_eq!(effective_frame_cap(300.0, 20.0, true), 300.0);
        assert_eq!(effective_frame_cap(0.0, 20.0, true), 0.0);
        // 4) bg 只能收紧：更宽的 bg 不改变更严的前台上限
        assert_eq!(effective_frame_cap(30.0, 120.0, false), 30.0);
        // 非法的负值按"不设限"处理，绝不允许变成负 fps
        assert_eq!(effective_frame_cap(-5.0, -1.0, false), 0.0);
    }

    /// 🔴 **Linux 适配判据**：窗口请求尺寸必须是**物理像素**，且超屏时等比缩。
    ///
    /// 修的是 `LogicalSize::new(w / 1.5, h / 1.5)` —— 那个 1.5 是 Windows 那台机器
    /// `scale_factor = 1.5` 的硬编码补偿，在 scale=1.0 的 Linux 上把 2560x1600
    /// 变成 1706x1066，而 Wayland 下窗口尺寸直接决定交换链尺寸 ⇒ 整条渲染尺寸链都错。
    ///
    /// 三条断言各自对应一个会被写错的方向（都不是恒真断言）：
    /// 1. **不缩放**：请求值原样返回 —— 任何乘除常数都会让这条红；
    /// 2. **超屏等比缩**：且比例保持（缩完仍宽高比一致），不是只夹一边；
    /// 3. **下限**：0 或极小值不许原样透出去（合成器会拒绝 0 尺寸窗口）。
    #[test]
    fn window_request_is_physical_and_clamped() {
        // 1) 显式分辨率（物理）原样透传 —— 显示器足够大时一个像素都不许改
        assert_eq!(window_physical_request(2560, 1600, Some((2560, 1600))), (2560, 1600));
        assert_eq!(window_physical_request(1920, 1080, Some((3840, 2160))), (1920, 1080));
        // 2) 超屏等比缩：3840x2160 请求放进 1920x1080 => 恰好一半，宽高比不变
        let (w, h) = window_physical_request(3840, 2160, Some((1920, 1080)));
        assert_eq!((w, h), (1920, 1080));
        // 只超高（不超宽）时也要缩，且按**较小的**那个比例缩（否则会溢出另一边）
        let (w, h) = window_physical_request(1920, 4000, Some((1920, 1080)));
        assert!(w <= 1920 && h <= 1080, "缩完不许仍溢出：{w}x{h}");
        assert!(w > 0 && h > 0);
        // 3) 拿不到显示器（Wayland 的 primary_monitor() 恒 None）=> 如实请求，不猜
        assert_eq!(window_physical_request(2560, 1600, None), (2560, 1600));
        // 4) 下限：0 与极小值不许透传
        assert_eq!(window_physical_request(0, 0, None), (320, 200));
        assert_eq!(window_physical_request(100, 50, None), (320, 200));
    }

    /// 🔴 **Linux 适配判据**：`RV3D_BACKEND=x11` 在**原生 Linux** 上必须能生效。
    ///
    /// 旧代码的判据是 `is_wsl && WAYLAND_DISPLAY.is_some()` ⇒ 原生 Arch/KDE Wayland 上
    /// **恒为 false**，而 winit 0.30 已删除 `WINIT_UNIX_BACKEND` ⇒ 用户**没有**任何
    /// 退回 XWayland 的手段。这不是"以防万一"的开关：Wayland 下 `Locked` 抓取的失败
    /// 是**静默**的（winit 忽略合成器的确认事件），XWayland 是唯一可验证的退路。
    ///
    /// 每条断言对应一个**会被写错的方向**：
    /// 1. **非 WSL 也必须能选到 x11**（这一条就是整个修复）；
    /// 2. 显式变量**压过** WSL 自动判定（两个方向都测：wsl 上选 wayland、非 wsl 上选 x11）；
    /// 3. 大小写/空白不敏感（`X11`、` xwayland ` 都要认）；
    /// 4. 未设时不改变旧行为（WSL 自动选 x11；原生 Linux 保持 Auto）。
    #[cfg(target_os = "linux")]
    #[test]
    fn backend_choice_x11_is_reachable_on_native_linux() {
        // 1) 就是这个修复：非 WSL 的原生 Linux + 显式 x11
        assert_eq!(backend_choice(Some("x11"), false), BackendChoice::X11);
        // 2) 显式变量压过自动判定（两个方向）
        assert_eq!(backend_choice(Some("wayland"), true), BackendChoice::Auto);
        assert_eq!(backend_choice(Some("x11"), true), BackendChoice::X11);
        // 3) 大小写与空白
        assert_eq!(backend_choice(Some("X11"), false), BackendChoice::X11);
        assert_eq!(backend_choice(Some(" xwayland "), false), BackendChoice::X11);
        assert_eq!(backend_choice(Some("Wayland"), true), BackendChoice::Auto);
        // 4) 未设 / 不认识的取值 ⇒ 旧行为不变
        assert_eq!(backend_choice(None, true), BackendChoice::X11, "WSL 自动判定保持不变");
        assert_eq!(backend_choice(None, false), BackendChoice::Auto, "原生 Linux 默认不强制");
        assert_eq!(backend_choice(Some("nonsense"), false), BackendChoice::Auto);
    }

    /// 🔴 **Linux 适配判据**：锁定态的「假成功」必须能被**行为证据**判出来。
    ///
    /// `set_cursor_grab(Locked)` 返回 `Ok` 不代表锁生效（Wayland 下指针尚未 enter 时
    /// winit 什么都没做也返回 `Ok`，且它忽略合成器的确认事件）。失败症状与"鼠标坏了"
    /// 一模一样（零 raw 事件 ⇒ 视角不动），所以**唯一**可信的判据是"锁定之后真的
    /// 收到过相对增量"。
    ///
    /// 每条断言对应一个**会被写错的方向**：
    /// 1. 观察窗内没增量 ⇒ `Pending`（**不许**提前下结论 —— 用户可能只是还没动鼠标，
    ///    这一条挂了就会把正常玩家误判成假成功并去补抓，反而干扰他）；
    /// 2. 增量**优先于时间**：就算第 1ms 就来，也立刻 `Confirmed`（最强的正面证据，
    ///    没有理由再等满观察窗）；
    /// 3. 到点仍为零 ⇒ `NoMotion`（这才是要报警的那条）；
    /// 4. 边界：恰好等于窗口长度算到点（`>=` 而不是 `>` —— 写成 `>` 会让判定
    ///    永远晚一帧，在 165fps 下无感，但语义上"满窗"就该有结论）。
    #[test]
    fn lock_observation_needs_evidence_not_just_ok() {
        const W: u128 = 1500;
        // 1) 窗口内、无增量 ⇒ 不许下结论
        assert_eq!(lock_observation(0, 0, W), LockObservation::Pending);
        assert_eq!(lock_observation(W - 1, 0, W), LockObservation::Pending);
        // 2) 增量优先于时间：第 1ms 就来也立刻确认
        assert_eq!(lock_observation(1, 1, W), LockObservation::Confirmed);
        assert_eq!(lock_observation(0, 7, W), LockObservation::Confirmed);
        // 3) 到点仍为零 ⇒ 假成功
        assert_eq!(lock_observation(W, 0, W), LockObservation::NoMotion);
        // 4) 边界：恰好满窗算到点；超时很久也仍是 NoMotion（不是 Confirmed）
        assert_eq!(lock_observation(u128::MAX, 0, W), LockObservation::NoMotion);
        // 反向保险：确认过之后再多的零增量也不该翻回 NoMotion（delta 是单调的）
        assert_eq!(lock_observation(u128::MAX, 3, W), LockObservation::Confirmed);
    }

    /// 观察窗长度不许是 0 —— 那会让判定在第一帧就报假成功，把"还没动鼠标"当成故障。
    #[test]
    fn lock_observe_window_is_not_degenerate() {
        assert!(LOCK_OBSERVE_MS >= 200, "太短会把正常玩家误判成假成功");
        assert!(LOCK_OBSERVE_MS <= 5000, "太长则用户看着不动的视角干等");
    }

    /// 🔴 **Linux 适配判据**：`RV3D_NO_CAPTURE=1` 必须能**无条件**关掉光标抓取。
    ///
    /// 为什么这条在 Linux 上是必需品：Windows 侧的鼠标安全协议靠 `PostMessage`
    /// 投键 + 不抢前台（用户 2026-09-03 的要求），于是自动化不必抓光标；Linux 没有
    /// 这条路，而引擎在 `Playing` 态一定会抓（`Locked` 或 `Confined`）⇒ 跑一局冒烟
    /// 就等于把用户的指针锁进游戏窗口。把"关掉捕获"做成一条判据，自动化不碰用户
    /// 输入才是**代码保证**，而不是靠调用方自觉。
    ///
    /// 每条断言对应一个**会被写错的方向**：
    /// 1. `allowed=false` ⇒ 恒假（**这一条就是那个开关**；漏掉它 = 开关无效）；
    /// 2. 四个条件的**每一个**单独为假都要能关掉捕获 —— 特别是 `esc_menu_open`：
    ///    漏掉它就是"菜单里鼠标被锁死，只能 Alt+F4"（本仓 2026-08-15 修过一次）；
    /// 3. 全真才为真（防止把 `||` 写成 `&&` 之类的反向错误）。
    #[test]
    fn capture_wanted_obeys_the_no_capture_switch_and_the_menu_gates() {
        // 1) 开关优先：其它条件全真也必须是假
        assert!(!capture_wanted(true, true, false, false, false), "RV3D_NO_CAPTURE=1 必须关掉捕获");
        // 2) 四个条件逐个单独为假
        assert!(!capture_wanted(false, true, false, false, true), "失焦不许抓");
        assert!(!capture_wanted(true, false, false, false, true), "非 Playing 不许抓");
        assert!(!capture_wanted(true, true, true, false, true), "设置面板打开不许抓");
        assert!(!capture_wanted(true, true, false, true, true), "ESC 菜单打开不许抓");
        // 3) 全真才抓
        assert!(capture_wanted(true, true, false, false, true));
    }

    /// 🔴 判据：中继注册失败时**不许**报「已注册」（教训 46：日志不许把「没跑成」写成成功）。
    ///
    /// 两个方向都要：成功时文案里必须有房间名与端口（现场要用它核对），
    /// 失败时文案里必须**没有**成功字样，且必须带出原始错误。
    #[test]
    fn relay_registration_report_never_claims_success_on_failure() {
        let ok: std::io::Result<()> = Ok(());
        let (okv, msg) = rdv_register_report("10.0.0.1:9000", "steel", 27015, &ok);
        assert!(okv, "成功必须报成功: {msg}");
        assert!(msg.contains("已向中继") && msg.contains("steel"), "{msg}");
        assert!(msg.contains("27015"), "端口要出现在现场日志里: {msg}");

        let err: std::io::Result<()> = Err(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "no route",
        ));
        let (okv2, msg2) = rdv_register_report("10.0.0.1:9000", "steel", 27015, &err);
        assert!(!okv2, "失败不许报成功: {msg2}");
        assert!(msg2.contains("失败"), "{msg2}");
        assert!(
            !msg2.contains("已向中继"),
            "失败文案里不许留下成功字样: {msg2}"
        );
        assert!(msg2.contains("no route"), "要把原始错误带出来: {msg2}");
    }

    /// 🔴 判据：`scripts/*.ps1` 的**每一行都不许以非 ASCII 字节结尾**。
    ///
    /// 存在理由（2026-09-26 实测，5 个文件全中）：Windows PowerShell 5.1 把**无 BOM** 的
    /// `.ps1` 按系统 ANSI（本机 GBK）解码。某行若以中文/全角字符结尾，其最后一个字节落在
    /// GBK 首字节区间，解码器就把**行尾**当成它的第二个字节吃掉 ⇒ 下一行被并进这一行；
    /// 若这一行恰好是注释，**下一行代码就被静默注释掉**，脚本照跑但少了一条语句：
    ///   * `compile_pt.ps1` 吃掉 `$ErrorActionPreference = 'Stop'`（编译失败不再中止）；
    ///   * `pt_power_ab.ps1` 吃掉 `function Run-Case(...)`（整个脚本不可用）；
    ///   * `ask_qianwen.ps1` 吃掉 `$ix = ...`（点击落在 x=0）；
    ///   * `release_input.ps1` 吃掉 `$alive = 0`；
    ///   * `run_gameplay_smoke.ps1` 吃掉四处代码（含启动游戏与跑冒烟的调用）。
    /// 判据只看**行尾字节**：文件里出现中文没关系（`send_work.ps1` 的消息体就是中文，
    /// 但它后面跟着 ASCII 的 `}`），被吃掉的一定是"行尾非 ASCII"的那些行。
    #[test]
    fn powershell_scripts_never_end_a_line_with_a_non_ascii_byte() {
        // 🔴 2026-09-26：扫描面以前只有 `scripts/`，而 `tools/shot_diff.ps1`（**已入库**）
        // 当时就有 6 行以中文结尾 ⇒ 它下一行（`$ErrorActionPreference` / `$src = ...` /
        // `$step = 7`）全被注释掉了 —— `tools/` 不在扫描面里，所以这条判据一直是绿的。
        // **判据的扫描面必须覆盖"所有入库的 .ps1"**，否则它只保护它恰好记得的那一个目录。
        let dirs = ["scripts", "tools"];
        let mut checked = 0usize;
        let mut bad: Vec<String> = Vec::new();
        for dir in dirs {
            for entry in std::fs::read_dir(dir).expect("目录必须存在") {
                let path = entry.expect("读取目录项失败").path();
                if path.extension().and_then(|s| s.to_str()) != Some("ps1") {
                    continue;
                }
                checked += 1;
                let bytes = std::fs::read(&path).expect("读 .ps1 失败");
                let name = format!(
                    "{}/{}",
                    dir,
                    path.file_name().and_then(|s| s.to_str()).unwrap_or("?")
                );
                for (i, line) in bytes.split(|b| *b == b'\n').enumerate() {
                    // 结尾的 CR 不算行尾内容（CRLF 文件）
                    let end = if line.last() == Some(&b'\r') { line.len().saturating_sub(1) } else { line.len() };
                    if end == 0 {
                        continue;
                    }
                    if line[end - 1] >= 0x80 {
                        let text = String::from_utf8_lossy(&line[..end]).to_string();
                        bad.push(format!("{name}:{}  …{}", i + 1, &text[text.len().saturating_sub(40)..]));
                    }
                }
            }
        }
        // 目录走空时这条会静默通过 —— 那正是教训 27 的形态（"没测到"与"测到 0"分不清）
        assert!(checked >= 10, "只扫到 {checked} 个 .ps1，路径大概不对");
        assert!(
            bad.is_empty(),
            "有 {} 行以非 ASCII 字节结尾（PS 5.1 按 ANSI 读 ⇒ 会吞掉下一行）：\n{}",
            bad.len(),
            bad.join("\n")
        );
    }

    /// 🔴 判据：**入库的文本文件里不许有 NUL 字节**（2026-09-26 实测命中 1 处）。
    ///
    /// 存在理由：`docs/HANDOFF-soldier.md` 里藏着一个 NUL —— 就在「· 0 警告 ·」那句里，
    /// `0` 被写成了 `0x00`。后果是**文本工具到此为止**：`read` 直接判定 "binary file" 拒绝读取，
    /// 按行处理的脚本也会在那行出怪事 —— 而在终端里它只显示成空白，**完全看不出来**。
    /// 这与教训 36 同族：文件"看着好好的"，工具却读不了。
    ///
    /// 扫描面 = `docs/*.md` + `src/**/*.rs` + `build.rs`/`build_spv_rt.rs`；
    /// 自检：扫到的文件数必须 ≥ 30，否则路径写错也会静默通过（教训 27）。
    #[test]
    fn tracked_text_files_contain_no_nul_byte() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out);
                    } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                        out.push(p);
                    }
                }
            }
        }
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir("docs") {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("md") {
                    files.push(p);
                }
            }
        }
        walk(std::path::Path::new("src"), &mut files);
        for extra in ["build.rs", "build_spv_rt.rs"] {
            let p = std::path::PathBuf::from(extra);
            if p.exists() {
                files.push(p);
            }
        }
        assert!(files.len() >= 30, "只扫到 {} 个文本文件，路径大概不对", files.len());
        let bad: Vec<String> = files
            .iter()
            .filter_map(|p| {
                let b = std::fs::read(p).ok()?;
                let at = b.iter().position(|x| *x == 0)?;
                let line = b[..at].iter().filter(|x| **x == b'\n').count() + 1;
                Some(format!("{} 第 {line} 行附近（字节偏移 {at}）", p.display()))
            })
            .collect();
        assert!(
            bad.is_empty(),
            "文本文件里含 NUL 字节 ⇒ 文本工具会把它当二进制拒绝读取：\n{}",
            bad.join("\n")
        );
    }

    /// 🔴 跨文件重复的着色器常量必须逐值一致（2026-10-01）。
    ///
    /// 存在理由：本会话修掉的**两条真缺陷是同一个模式**——同一约定存在若干份物理副本，
    /// 改了生产者、漏改消费者：
    /// - `pt_set_scene_markers` 的 `* 0.5` 是 2026-09-17 之前"渲染盒 = 2×AABB"的遗留，
    ///   生产者改了、PT 这个消费者没跟上 ⇒ **PT 的每个 marker 盒整体小一半**（§22.14）；
    /// - `main.rs` 占领底盘 `from_scale(10.0,…)` 配注释"半径 5.0 → scale 10.0"，
    ///   同样是旧约定遗留 ⇒ **领地底盘画成真实占领圈的 2 倍**（§23.13）。
    /// 两条都**从画面上看不出来**、只能靠把值对起来算。既然算得出来，就该在测试里算，
    /// 而不是等下一位再花两小时反推。
    ///
    /// 自检（教训 27：判据必须能红）：**每份副本都必须真的被找到**。若某处改了名、
    /// 挪了文件或正则不匹配，"找到 0 份"绝不能算通过——那正是本仓反复踩的恒真断言。
    #[test]
    fn duplicated_shader_constants_stay_in_sync() {
        /// 取某个标识符在**非注释行**上的所有声明值。
        /// 跳过注释是必须的：说明性注释里满是"必须与 X 同值"这类句子，
        /// 不跳过的话注释里提到的数字会被当成一份副本，把测试变成噪音。
        fn decls(src: &str, name: &str) -> Vec<f32> {
            let mut out = Vec::new();
            for line in src.lines() {
                let t = line.trim_start();
                if t.starts_with("//") || !line.contains(name) {
                    continue;
                }
                let Some(eq) = line.find('=') else { continue };
                // 必须先 trim：`const X: f32 = 1.5;` 的等号后紧跟一个空格，
                // 不 trim 则 take_while 首字符即失败、解析出空串 ⇒ 一份都找不到。
                // （本函数第一版就栽在这里，靠下面"必须找到 4 份"的自检才没静默通过。）
                let rest = line[eq + 1..].trim_start();
                let num: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                    .collect();
                if let Ok(v) = num.parse::<f32>() {
                    out.push(v);
                }
            }
            out
        }

        let read = |p: &str| -> String {
            std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("读 {p} 失败：{e}（测试工作目录应为仓库根）"))
        };
        let build = read("build.rs");
        let ray = read("src/engine/ray_tracer.rs");
        let ptshader = read("assets/rt/pt_panorama.glsl");

        // ① 砌块皮肤尺寸门：光栅 WGSL 两份（顶点着色器 + mesh 着色器各一份）
        //    + PT 的 Rust 常量 + PT 的 GLSL 常量 = 4 份。
        let mut gate: Vec<(&str, f32)> = Vec::new();
        for (i, v) in decls(&build, "MASONRY_MIN_SPAN").into_iter().enumerate() {
            gate.push((if i == 0 { "build.rs(VS)" } else { "build.rs(mesh)" }, v));
        }
        for v in decls(&ray, "MASONRY_MIN_SPAN") {
            gate.push(("ray_tracer.rs", v));
        }
        for v in decls(&ptshader, "MASONRY_MIN_SPAN") {
            gate.push(("pt_panorama.glsl", v));
        }
        assert_eq!(
            gate.len(),
            4,
            "MASONRY_MIN_SPAN 应有 4 份声明，实际找到 {} 份（{:?}）\
             ⇒ 有副本被改名/挪走/删掉了，这个测试本身需要跟着更新，不要直接放宽断言",
            gate.len(),
            gate
        );
        let want = crate::engine::ray_tracer::MASONRY_MIN_SPAN;
        let off: Vec<String> = gate
            .iter()
            .filter(|(_, v)| (*v - want).abs() > 1e-6)
            .map(|(w, v)| format!("{w} = {v}"))
            .collect();
        assert!(
            off.is_empty(),
            "MASONRY_MIN_SPAN 各副本与 ray_tracer 的 {want} 不一致：{}\
             ⇒ 尺寸门会在光栅/PT 两侧给出不同判定（§22.14 同族错法）",
            off.join("；")
        );

        // ② 皮肤 tile 尺寸：光栅用字面量、PT 用常量，两处必须同为 1.6 × 0.8。
        //    这是 §22.4b 那套"砖块尺度"的唯一真值来源，任一侧改动都会让两侧砖大小不同。
        assert!(
            build.contains("vec2<f32>(1.6, 0.8)"),
            "build.rs 里找不到皮肤 tile 字面量 `vec2<f32>(1.6, 0.8)`\
             ⇒ 若改了写法（例如换成常量），请同步更新本测试与 PT 侧，不要删断言"
        );
        assert!(
            ptshader.contains("SKIN_TILE_M = vec2(1.6, 0.8)"),
            "pt_panorama.glsl 里找不到 `SKIN_TILE_M = vec2(1.6, 0.8)`\
             ⇒ 两侧皮肤 tile 已脱钩，砖块尺度会光栅/PT 不一致"
        );
    }

    /// 🔴 尺寸门用的 **1.05 必须"除门本身以外与 1.0 不可区分"**（§22.7）。
    ///
    /// `flat_flag` 是片元着色器的材质分派值：0=地面、1.0=marker、1.25=外部建模、
    /// 2.0=NPC、3.0=枪。2026-09-30 起多了一个 **1.05 = "太小、不发砌块皮肤"的 marker**
    /// （`MASONRY_MIN_SPAN` 尺寸门，护柱/消防栓这类小件走它）。
    ///
    /// 风险不在门本身，而在**将来新加的那一条分支**：只要有人写出
    /// `flat_flag > 1.0` 或 `flat_flag <= 1.03` 这种**在 1.0 与 1.05 上取值不同**的判据，
    /// 被门挡下的 1.05 就会**悄悄走进与 1.0 不同的分支** —— 症状是"护柱忽然少了一层
    /// 效果"，而没人会去查一个 0.05 的差。⇒ 把它变成硬约束：
    /// **片元里每条 flat_flag 判据在 1.0 与 1.05 上取值必须相同，
    /// 唯一例外是阈值恰为 1.02 的那条（尺寸门自己）。**
    ///
    /// ⚠️ 判据是"**取值不同**"，不是"阈值落在 1.0~1.05 之间"：`flat_flag < 1.1`
    /// 两条都成立 ⇒ **无害**，本测试不该报它（第一版把例子写成 `< 1.1`，
    /// 被 `target/guardlogic.py` 的合成用例当场否掉）。
    ///
    /// 自检（教训 27：判据必须能红）：解析到的判据数必须 **>= 8**，且必须**至少找到
    /// 一条阈值 1.02 的门** —— 否则"没有例外"会因为"什么都没解析到"而恒真通过。
    #[test]
    fn gated_marker_flag_is_indistinguishable_except_at_the_gate() {
        /// 把非注释行里的 `flat_flag <op> <num>` 解析成 (算子, 阈值, 行号)。
        fn atoms(src: &str) -> Vec<(String, f32, usize)> {
            let mut out = Vec::new();
            for (ln, line) in src.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                let mut base = 0usize;
                while let Some(k) = line[base..].find("flat_flag") {
                    let i = base + k + "flat_flag".len();
                    let rest = line[i..].trim_start();
                    let op = ["<=", ">=", "==", "!=", "<", ">"]
                        .iter()
                        .find(|o| rest.starts_with(**o))
                        .copied();
                    let Some(op) = op else {
                        base = i;
                        continue;
                    };
                    let num: String = rest[op.len()..]
                        .trim_start()
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '.')
                        .collect();
                    if let Ok(v) = num.parse::<f32>() {
                        out.push((op.to_string(), v, ln + 1));
                    }
                    base = i;
                }
            }
            out
        }

        fn holds(op: &str, flag: f32, rhs: f32) -> bool {
            match op {
                "<=" => flag <= rhs,
                ">=" => flag >= rhs,
                "==" => flag == rhs,
                "!=" => flag != rhs,
                "<" => flag < rhs,
                ">" => flag > rhs,
                other => panic!("未知算子 {other}"),
            }
        }

        let build = std::fs::read_to_string("build.rs")
            .expect("读 build.rs 失败（测试工作目录应为仓库根）");
        let lines: Vec<&str> = build.lines().collect();
        let from = lines
            .iter()
            .position(|l| l.contains("const FRAGMENT_SHADER_WGSL"))
            .expect("找不到 FRAGMENT_SHADER_WGSL 起点");
        let to = lines[from + 1..]
            .iter()
            .position(|l| l.contains("const MESH_SHADER_WGSL"))
            .map(|p| p + from + 1)
            .expect("找不到 FRAGMENT_SHADER_WGSL 终点（MESH_SHADER_WGSL 之后）");

        let a = atoms(&lines[from..to].join("\n"));
        assert!(
            a.len() >= 8,
            "片元里只解析到 {} 条 flat_flag 判据（应 >= 8）⇒ 着色器改了形态或解析器坏了，\
             本测试会因'没解析到'而假通过，必须先修解析器再下结论",
            a.len()
        );
        assert!(
            a.iter().any(|(_, v, _)| (*v - 1.02).abs() < 1e-6),
            "一条阈值 1.02 的尺寸门都没找到 ⇒ 门被删了或本测试期望已过时（§22.7）"
        );

        let diff: Vec<String> = a
            .iter()
            .filter(|(op, v, _)| {
                (v - 1.02).abs() > 1e-6 && holds(op, 1.0, *v) != holds(op, 1.05, *v)
            })
            .map(|(op, v, ln)| format!("build.rs:{ln}  flat_flag {op} {v}"))
            .collect();
        assert!(
            diff.is_empty(),
            "这些 flat_flag 判据会让 1.05（被尺寸门挡下的小件）与 1.0 走不同分支：{}\
             ⇒ 要么把阈值挪出 (1.0, 1.05] 区间，要么显式写成对 1.05 也成立",
            diff.join("；")
        );
    }

    /// 🔴 每把 GLB 枪模的索引必须落在顶点数以内，且顶点不得含 NaN/Inf。
    ///
    /// 存在理由（2026-09-15）：**按 2 切枪（AK-104）会直接把设备打掉**
    /// （`vkQueueSubmit` → `VK_ERROR_DEVICE_LOST`）。越界索引在 GPU 上是**顶点抓取越界**：
    /// 不报 VUID、不 panic，只是整台设备消失 —— 而索引是 GLB 解析器算出来的，
    /// 只要合并多 primitive 时漏加基址偏移就会整段偏出去。
    /// 这条测试把"能不能安全上传"变成上传**之前**就能判的纯数字判据。
    #[test]
    fn gun_glb_indices_all_in_range() {
        let mut checked = 0;
        for entry in std::fs::read_dir("assets/guns").expect("assets/guns 目录必须存在") {
            let path = entry.expect("读取 assets/guns 项失败").path();
            if path.extension().and_then(|s| s.to_str()) != Some("glb") {
                continue;
            }
            let key = path
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("枪模文件名必须是 UTF-8")
                .to_string();
            let Some((verts, indices)) = GameApp::load_gun_glb(&key) else {
                continue;
            };
            checked += 1;
            assert!(!verts.is_empty(), "{key}: 顶点不得为空");
            let max_i = indices.iter().copied().max().unwrap_or(0);
            assert!(
                (max_i as usize) < verts.len(),
                "{key}: 索引越界 —— 最大索引 {max_i} ≥ 顶点数 {}（GPU 顶点抓取越界 = device lost）",
                verts.len()
            );
            for (i, v) in verts.iter().enumerate() {
                assert!(
                    v.pos.iter().all(|c| c.is_finite()) && v.color.iter().all(|c| c.is_finite()),
                    "{key}: 顶点 #{i} 含 NaN/Inf（pos={:?} color={:?}）",
                    v.pos,
                    v.color
                );
            }
        }
        // 2026-10-30：许可清理后仓库只剩 **8 件** 第三方枪模（其中 ak12 是 GLB 自带
        // CC BY 元数据的那件）；另外 8 件追不到原始出处，已从仓库移出（见
        // `docs/WEAPON-LICENCE-AUDIT.md` §3）。这条断言的本意是"确实扫到了一批文件"，
        // 不是钉死某个数字，所以跟着资产清单走 8；**不要**为了让数字好看而把模型加回来。
        assert!(checked >= 8, "至少应校验到 8 把 GLB 枪模，实际 {checked}");
    }

    /// 键与资产文件名必须对得上（2026-10-31 实测踩过）：`ak12m` 的 GLB 叫 `ak12.glb`，
    /// 少了映射 ⇒ `exists()` 恒 false ⇒ 主力枪静默掉到程序化模型。
    #[test]
    fn gun_key_maps_to_the_shipped_glb_name() {
        assert_eq!(GameApp::gun_glb_stem("ak12m"), "ak12");
        assert_eq!(GameApp::gun_glb_stem("pkm"), "pkm");
        // 凡是仓库里真有 GLB 的键，映射后必须能找到文件（反向：把映射改坏 ⇒ 这里红）
        for entry in std::fs::read_dir("assets/guns").expect("assets/guns 必须存在") {
            let name = entry.expect("读目录项").file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".glb") else { continue };
            let key = if stem == "ak12" { "ak12m" } else { stem };
            assert_eq!(
                GameApp::gun_glb_stem(key),
                stem,
                "键 {key} 映射到 {}，但库里是 {stem}.glb",
                GameApp::gun_glb_stem(key)
            );
        }
        assert!(
            GameApp::load_gun_glb("ak12m").is_some(),
            "ak12m 有 assets/guns/ak12.glb，却返回 None（映射又丢了？）"
        );
    }

    /// **每把武器都必须有自己的模型来源**（2026-10-31）：要么 `assets/guns/{key}.glb` 在，
    /// 要么 `engine::guns` 里有它的程序化模型。两者都没有的键只能借用 `ak12.glb`，
    /// 换枪就等于没换外观 —— 这正是许可清理后暴露出来的问题。
    #[test]
    fn every_weapon_has_a_model_source() {
        let mut procedural_only = Vec::new();
        for spec in crate::engine::weapon_data::ALL_WEAPONS.iter() {
            let glb = format!("assets/guns/{}.glb", spec.key);
            let has_glb = std::path::Path::new(&glb).exists();
            let has_proc = crate::engine::guns::has_procedural_mesh(spec.key);
            assert!(
                has_glb || has_proc,
                "武器 '{}' 既没有 {} 也没有程序化模型 ⇒ 会静默借用 ak12 的外形",
                spec.key,
                glb
            );
            if !has_glb {
                procedural_only.push(spec.key);
            }
        }
        // 至少要有 8 把走程序化（2026-10-30 许可清理移出的那批），否则说明这个判据
        // 在"GLB 全在"的机器上恒真、测不到东西。
        assert!(
            procedural_only.len() >= 8,
            "只有 {} 把武器走程序化模型，预期 >=8（被移出仓库的枪应当靠自己的程序化模型）",
            procedural_only.len()
        );
    }

    /// 被移出仓库的 8 件枪模，**必须回落到自己的程序化模型**，而不是 `ak12.glb`。
    /// 反向验证：把 `load_gun_glb` 里 `has_procedural_mesh` 那条分支删掉 ⇒ 这里拿到 `Some`
    /// （= ak12 的网格）⇒ 红。
    #[test]
    fn removed_gun_glbs_use_their_own_procedural_mesh() {
        for key in [
            "ak104", "ash12", "pkm", "pkp", "rpk16", "saiga12", "sv98", "svd12",
        ] {
            assert!(
                !std::path::Path::new(&format!("assets/guns/{key}.glb")).exists(),
                "{key}.glb 又回到仓库了？许可清理的结论见 docs/WEAPON-LICENCE-AUDIT.md"
            );
            assert!(
                GameApp::load_gun_glb(key).is_none(),
                "{key} 应当返回 None（走自己的程序化模型），却拿到了 GLB 网格"
            );
            assert!(
                crate::engine::guns::has_procedural_mesh(key),
                "{key} 没有程序化模型，回退链会掉到 ak12"
            );
        }
        // 有 GLB 的键照旧走 GLB：这条防止"把回退顺序改反了"。
        // ⚠ 这一条在测试进程里**不能**断言 "ak12m 返回 Some"：`load_gun_glb` 走
        // `asset_source::global()`，而 10 号那把枪的 GLB 有 2.3 MB、解析要走完整 glTF 路径，
        // 单元测试只该钉住**回退分支**。反过来钉一条不容易被误改的事实：ak12m 的 GLB 在库里。
        assert!(
            std::path::Path::new("assets/guns/ak12.glb").exists(),
            "ak12m 的 GLB 不在库里 ⇒ 回退链的最后一级也没了"
        );
    }

    /// raw 不可用的平台（本机 Windows）**连试都不许试** `Locked`。
    /// 闭包写成 panic 而不是返回 false，是为了把"没被调用"也钉住 ——
    /// 若哪天有人把 `raw_motion &&` 去掉，这条测试会立刻炸，而不是静默退化成
    /// "试了 Locked 拿到 Ok，于是 cursor_locked = true，视角从此无输入"。
    #[test]
    fn grab_plan_never_tries_locked_without_raw_motion() {
        let (locked, grabbed) = cursor_grab_plan(
            false,
            || panic!("raw 不可用的平台不得尝试 Locked"),
            || true,
        );
        assert!(!locked, "raw 不可用时 cursor_locked 必须为 false");
        assert!(grabbed, "应退到 Confined 并报告已抓住");

        let (locked, grabbed) = cursor_grab_plan(
            false,
            || panic!("raw 不可用的平台不得尝试 Locked"),
            || false,
        );
        assert!(!locked);
        assert!(!grabbed, "Confined 也失败时应如实报告未抓住");
    }

    /// raw 可用（X11/Wayland/macOS）时保持既有优先级：Locked → Confined。
    #[test]
    fn grab_plan_prefers_locked_where_raw_motion_exists() {
        assert_eq!(cursor_grab_plan(true, || true, || false), (true, true));
        assert_eq!(cursor_grab_plan(true, || false, || true), (false, true));
        assert_eq!(cursor_grab_plan(true, || false, || false), (false, false));
    }

    /// Windows 上 RAW_MOUSE_MOTION 必须是 false：winit 的 Windows 后端不构造
    /// DeviceEvent::MouseMotion，为真会让 capture 走进无输入的死路。
    #[test]
    #[cfg(target_os = "windows")]
    fn windows_backend_has_no_raw_mouse_motion() {
        assert!(
            !RAW_MOUSE_MOTION,
            "winit 的 Windows 后端只发 Added/Removed；这里若为真，锁定即等于视角失效"
        );
    }

    /// 以恒定速度直行 `secs` 秒，返回积分后的摆动状态。
    /// `dt` 故意可传入不同帧率，用于断言"同一行程得到同一状态"。
    /// 进循环前先 tick 一次播种 `prev_pos`，否则两种帧率会各少吃一步的行程。
    ///
    /// 帧数必须**四舍五入**：`(0.5f32 / (1.0f32 / 30.0)) as usize` 截断成 14 而不是 15
    /// （f32 里 1/30 的倒数乘回来是 14.999999），于是两种帧率模拟的根本不是同一段时长，
    /// 这条测试就从"验证帧率无关"变成了"验证两个不同时长相等"，必然红。
    fn walk(dt: f32, secs: f32, speed: f32) -> GunSway {
        let mut s = GunSway::new();
        let right = glam::Vec3::X;
        let fwd = glam::Vec3::NEG_Z;
        let mut pos = glam::Vec3::ZERO;
        s.tick(dt, pos, right, fwd, false, false);
        let step = speed * dt;
        for _ in 0..(secs / dt).round() as usize {
            pos += fwd * step;
            s.tick(dt, pos, right, fwd, false, false);
        }
        s
    }

    /// 帧率无关性：以 6 m/s 直行 0.5 秒，165 fps 与 30 fps 必须得到同样的相位和包络
    /// （旧实现用 `anim_clock * 7.5` 累积时间相位 + 逐帧原始速度，两者都与帧率耦合）。
    /// 0.5 s 让总相位落在 π 附近——刻意避开 2π 回绕点，否则比较的是回绕后的余数。
    #[test]
    fn gun_sway_is_framerate_independent() {
        let fast = walk(1.0 / 165.0, 0.5, 6.0);
        let slow = walk(1.0 / 30.0, 0.5, 6.0);
        assert!(
            (fast.stride - slow.stride).abs() < 0.05,
            "同样行程后相位应一致：{} vs {}",
            fast.stride,
            slow.stride
        );
        assert!(
            (fast.speed - slow.speed).abs() < 0.25,
            "低通后的速度应基本与帧率无关：{} vs {}",
            fast.speed,
            slow.speed
        );
    }

    /// 判据：静止呼吸微摆**有界、两轴都真的在动、且 y 走 2× 频率**（画"∞"而不是来回直线）。
    /// 并钉住"长时间运行不漂"（纯函数、无累积量）。
    #[test]
    fn idle_sway_is_bounded_and_figure_eight() {
        let mut max_x = 0.0f32;
        let mut max_y = 0.0f32;
        let mut x_changed = false;
        let mut y_changed = false;
        let mut prev = idle_sway(0.0);
        for i in 0..=2000 {
            let t = i as f32 * 0.01;
            let (x, y) = idle_sway(t);
            assert!(
                (-1.0..=1.0).contains(&x) && (-1.0..=1.0).contains(&y),
                "单位轨迹必须有界：({x}, {y})"
            );
            max_x = max_x.max(x.abs());
            max_y = max_y.max(y.abs());
            if (x - prev.0).abs() > 1e-3 {
                x_changed = true;
            }
            if (y - prev.1).abs() > 1e-3 {
                y_changed = true;
            }
            prev = (x, y);
        }
        assert!(x_changed && y_changed, "两轴都必须真的在动");
        assert!(max_x > 0.99, "x 轴应达到满幅，实际 {max_x}");
        assert!((max_y - 0.6).abs() < 0.01, "y 轴应是 0.6 倍幅值，实际 {max_y}");
        // 2× 频率：x 走半个周期时 y 回到同号（一圈"∞"的两个环在 y 上同相）
        let period_x = 1.0 / GUN_IDLE_HZ;
        let (x0, y0) = idle_sway(0.1);
        let (x1, y1) = idle_sway(0.1 + period_x * 0.5);
        assert!((x1 + x0).abs() < 0.02, "x 半周期后应反相：{x0} vs {x1}");
        assert!((y1 - y0).abs() < 0.02, "y 是 2× 频率 ⇒ 半周期后同相：{y0} vs {y1}");
    }

    /// 判据：冲刺姿态包络**必须帧率无关地收敛**，且松开 Shift 后回到精确 0
    /// （0 是 A/B 与"姿态项彻底不参与"的判据）。
    #[test]
    fn gun_sprint_pose_converges_and_is_framerate_independent() {
        let run = |dt: f32, secs: f32, sprinting: bool| {
            let mut s = GunSway::new();
            for _ in 0..(secs / dt).round() as usize {
                s.tick(dt, glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::NEG_Z, false, sprinting);
            }
            s
        };
        let fast = run(1.0 / 165.0, 0.5, true);
        let slow = run(1.0 / 30.0, 0.5, true);
        assert!(fast.sprint > 0.99, "0.5 s 后应基本收敛到满姿态：{}", fast.sprint);
        assert!(
            (fast.sprint - slow.sprint).abs() < 0.02,
            "冲刺姿态低通应与帧率无关：{} vs {}",
            fast.sprint,
            slow.sprint
        );
        // 松开 Shift：回到精确 0（不是"接近 0"）
        let mut s = fast;
        for _ in 0..200 {
            s.tick(1.0 / 165.0, glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::NEG_Z, false, false);
        }
        assert_eq!(s.sprint, 0.0, "松开冲刺后包络应收敛到精确 0");
        // 传送帧不该抹掉姿态目标（冲刺状态与位置无关）
        let mut t = run(1.0 / 165.0, 0.3, true);
        t.tick(
            1.0 / 165.0,
            glam::Vec3::new(80.0, 0.0, 80.0),
            glam::Vec3::X,
            glam::Vec3::NEG_Z,
            false,
            true,
        );
        assert!(t.sprint > 0.9, "传送帧后冲刺姿态应保持：{}", t.sprint);
    }

    /// 有界性 + 相位回绕：长时间运行后相位仍在 [0, 2π)、速度不发散、包络 ≤1。
    ///
    /// 这里**不**用紧容差查"不过冲"：600 s × 6 m/s 把坐标累加到 3.6e3 m，
    /// `now_pos - prev_pos` 在该量级下每帧带约一个 ulp（≈2.4e-4）的舍入误差，
    /// 再除以 dt=1/165 s 放大成约 7e-3 m/s 的**输入噪声**。低通本身单调逼近、不会过冲，
    /// 超的是这个噪声。紧的那条断言在下面的 `gun_sway_low_pass_does_not_overshoot`。
    #[test]
    fn gun_sway_stays_bounded_and_wrapped() {
        let s = walk(1.0 / 165.0, 600.0, 6.0);
        assert!(s.stride >= 0.0 && s.stride < std::f32::consts::TAU);
        assert!(
            s.speed <= 6.05,
            "长时间运行后速度发散，说明低通或相位累加失去了有界性：{}",
            s.speed
        );
        assert!(s.kick <= 1.0);
    }

    /// 不过冲（紧容差）：短时运行下坐标只有十几米，浮点噪声比容差小三个数量级，
    /// 因此这条能真正守住"指数低通单调逼近、绝不越过输入值"。
    #[test]
    fn gun_sway_low_pass_does_not_overshoot() {
        let s = walk(1.0 / 165.0, 3.0, 6.0);
        assert!(
            s.speed <= 6.0 + 1e-4,
            "指数低通是单调逼近，不应过冲：{}",
            s.speed
        );
        assert!(s.speed > 5.99, "3 秒后应已收敛到满幅，实际 {}", s.speed);
    }

    /// 瞬移/重生保护：单帧几十米的位移不得被当成巨型速度（旧实现没有这层保护，
    /// 而且 `player_speed()` 恒为 0，两种错法都会让摆动不可信）
    #[test]
    fn gun_sway_ignores_teleport() {
        let mut s = walk(1.0 / 165.0, 0.5, 6.0);
        assert!(s.speed > 5.0, "前置条件：应先积分出满幅速度");
        s.tick(
            1.0 / 165.0,
            glam::Vec3::new(0.0, 0.0, -50.0),
            glam::Vec3::X,
            glam::Vec3::NEG_Z,
            false,
            false,
        );
        assert!(s.speed < 1e-3, "传送帧速度应归零，实际 {}", s.speed);
    }

    /// 后坐包络连续：击发后逐帧单调下降，单帧变化量 ≤8%
    /// （旧实现在 0.25 s 整点把阻尼从 0.15 阶跃到 1.0，单帧变化 0.85 = 位置跳变）
    #[test]
    fn gun_recoil_kick_decays_continuously() {
        let mut s = GunSway::new();
        let dt = 1.0 / 165.0;
        s.tick(dt, glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::NEG_Z, true, false);
        assert!(s.kick > 0.9, "击发帧应接近满幅后坐");
        let mut prev = s.kick;
        for _ in 0..60 {
            s.tick(dt, glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::NEG_Z, false, false);
            assert!(s.kick <= prev, "包络不得回升：{} > {}", s.kick, prev);
            assert!(
                prev - s.kick < 0.08,
                "单帧后坐变化量应连续，实际 {}",
                prev - s.kick
            );
            prev = s.kick;
        }
        assert!(s.kick < 0.01, "0.36 s 后应基本归零，实际 {}", s.kick);
    }

    /// 枪模顶点色：坏资产（baseColorFactor 0.057）经反照率补偿后必须给出
    /// 可用的明暗区间，而不是旧公式的 0.049..0.066（梯度 ±0.017 = 纯黑剪影）
    #[test]
    fn gun_bake_color_keeps_a_readable_gradient() {
        let boost = GUN_REF_ALBEDO / 0.0768; // ak12.glb 实测最亮材质
        let raw = [0.0573, 0.0573, 0.0573];
        let dirs = [
            glam::Vec3::new(-0.45, 0.80, -0.30), // 迎光面（= 主光方向）
            glam::Vec3::Y,
            glam::Vec3::NEG_Y,
            glam::Vec3::X,
            glam::Vec3::NEG_X,
            glam::Vec3::NEG_Z, // 朝向镜头的侧面
            glam::Vec3::Z,     // 枪口方向
        ];
        let mut lo = f32::MAX;
        let mut hi: f32 = 0.0;
        for d in dirs {
            let n = d.normalize();
            let c = fp_gun_bake_color(n, raw, boost);
            for ch in c {
                assert!(ch.is_finite() && (0.0..=1.0).contains(&ch), "{:?} → {:?}", n, c);
                lo = lo.min(ch);
                hi = hi.max(ch as f32);
            }
        }
        assert!(lo > 0.03, "最暗面不应是纯黑，实际 {}", lo);
        assert!(hi < 0.99, "最亮面不得削顶，实际 {}", hi);
        assert!(
            hi / lo >= 4.0,
            "明暗比过小说明仍是平面剪影：{:.4} / {:.4}",
            hi,
            lo
        );
    }
}
 

/// 桌面入口：供薄 bin（`src/main.rs`）调用。
pub fn run_steel_front_desktop() {
    run_steel_front(None);
}

/// Android 入口：系统在 `.so` 加载后回调它（事件循环所有权反转）。
#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(app: AndroidApp) {
    run_steel_front(Some(app));
}
