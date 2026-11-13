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
mod app_init;
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
use engine::renderer::Renderer;
use engine::window;
use net::{Client, Server};

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
#[cfg(test)]
mod app_tests;
 

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
