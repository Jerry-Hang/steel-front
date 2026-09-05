//! 钢铁前线 (Steel Front) - 程序入口
//!
//! 游戏主循环：
//! 1. 初始化窗口（winit）
//! 2. 初始化 Vulkan 渲染器（ash）
//! 3. 事件循环处理输入
//! 4. 每帧更新相机并渲染

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

/// 第一人称枪摆动状态。
///
/// 全部量在 `update()` 内按 delta_time 积分（`fp_gun_matrix()` 只读），原因是
/// 旧实现在矩阵函数里就地用两个逐帧不连续的输入：
/// ① `game.player_speed() > 0.6` 硬开关（原始瞬时速度，无任何平滑）；
/// ② `sin(anim_clock * 7.5)` —— 相位参数随会话时长**无上界累积**，f32 尾数
///    24 bit，anim_clock 到 ~2.4 h 后 `相位 × 7.5` 的 ulp 已与每帧相位增量同量级，
///    sin 输出被量化 → 越玩越抖；且相位按时间而非行程推进，与帧率互相拍频。
/// ③ `fire_damp` 0.15→1.0 阶跃。
/// 新实现：相位按行程累积并恒定回绕到 [0, 2π)（参数有界 → 精度不衰减）；
/// 速度经与帧率无关的指数低通（`x += (t-x)*(1-exp(-dt/τ))`）；幅值用 smoothstep
/// 连续起落；后坐用连续指数包络。三者都满足"任意帧率下平滑且有界"。
struct GunSway {
    /// 上一帧玩家脚底世界坐标（用真实位移求速度，见 `update()` 里的说明）；
    /// None = 尚未播种（启动首帧、瞬移/重生之后），此时不按位移伪造速度
    prev_pos: Option<glam::Vec3>,
    /// 平滑后的水平速度模长（m/s）→ 只驱动幅值包络
    speed: f32,
    /// 平滑后的侧向速度分量（+ = 向右），→ 驱动侧向"惯性滞后"偏移
    strafe: f32,
    /// 平滑后的前向速度分量（+ = 前进），→ 驱动前后拖拽偏移
    fore: f32,
    /// 步态相位（弧度，恒定回绕到 [0, 2π)）
    stride: f32,
    /// 后坐冲量包络 [0,1]：开火帧置 1，其后按 exp(-dt/τ) 连续衰减
    kick: f32,
    /// 摆动总增益（RV3D_GUN_SWAY 覆盖，缺省 1.0；=0 可完全关闭摆动做 A/B 判定）
    gain: f32,
}

impl GunSway {
    fn new() -> Self {
        Self {
            prev_pos: None,
            speed: 0.0,
            strafe: 0.0,
            fore: 0.0,
            stride: 0.0,
            kick: 0.0,
            // 诊断门（与项目其它 RV3D_* 一致）：RV3D_GUN_SWAY=0 关闭全部枪摆动
            gain: env_f32("RV3D_GUN_SWAY").unwrap_or(1.0).clamp(0.0, 3.0),
        }
    }

    /// 下一帧不按位移伪造速度（重生/传送/切模式后用；`tick()` 会自动重新播种）
    #[allow(dead_code)] // 预留：game.rs 侧提供显式传送事件时调用（当前由 2 m 瞬移保护兜底）
    fn reset_motion(&mut self) {
        self.speed = 0.0;
        self.strafe = 0.0;
        self.fore = 0.0;
        self.prev_pos = None;
    }

    /// 每帧积分：`now_pos` 为玩家本帧脚底世界坐标，`right`/`fwd` 为相机基向量，
    /// `fired` 为本帧是否击发，`dt` 为本次 update 的帧时间（秒）。
    fn tick(
        &mut self,
        dt: f32,
        now_pos: glam::Vec3,
        right: glam::Vec3,
        fwd: glam::Vec3,
        fired: bool,
    ) {
        // 位移按水平面处理（y 是地形跟随，不属于步态）
        let moved = self
            .prev_pos
            .map(|p| now_pos - p)
            .unwrap_or(glam::Vec3::ZERO);
        self.prev_pos = Some(now_pos);
        let dxz = glam::Vec3::new(moved.x, 0.0, moved.z);
        let dist = dxz.length();
        // 瞬移保护：单帧 >2 m 只可能来自重生/传送/热重载（玩家最快 ~5 m/s，
        // 6 ms 帧内 ≤3 cm）。当作不连续事件：速度归零，不吃这一帧的假速度。
        if dist > 2.0 {
            self.speed = 0.0;
            self.strafe = 0.0;
            self.fore = 0.0;
        } else {
            // 与帧率无关的指数低通：a = 1 - exp(-dt/τ)，30 fps 与 165 fps 下
            // 同一段行程得到同一条速度曲线（旧实现直接取原始逐帧速度）
            let a = 1.0 - (-dt / GUN_SWAY_SMOOTH_TAU).exp();
            let inv = 1.0 / dt.max(1e-4);
            self.speed += (dist * inv - self.speed) * a;
            // 投影到相机水平基向量（分量式，避免构造 Vec3 的额外开销）
            let v_strafe = (dxz.x * right.x + dxz.z * right.z) * inv;
            let v_fore = (dxz.x * fwd.x + dxz.z * fwd.z) * inv;
            self.strafe += (v_strafe - self.strafe) * a;
            self.fore += (v_fore - self.fore) * a;
            // 相位按**行程**推进，随后回绕：sin/cos 的参数恒在 [0, 2π)，
            // 不会因长时间游玩而精度衰减；单帧推进量再受角速度上限约束，
            // 于是摆频在任何速度/帧率组合下都有硬上界
            let dphase = (dist * GUN_GAIT_PHASE_PER_M).min(dt * GUN_GAIT_MAX_RATE);
            self.stride = (self.stride + dphase) % std::f32::consts::TAU;
        }
        // 后坐包络：击发帧置 1（连发不叠加超过 1，保证幅值有上界），随后连续指数衰减
        if fired {
            self.kick = 1.0;
        }
        self.kick *= (-dt / GUN_RECOIL_TAU).exp();
        if self.kick < 1e-4 {
            self.kick = 0.0; // 收敛到精确 0，省掉之后每帧的无穷次微小乘法
        }
    }
}

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

/// 把「材质基色 + 局部法线」烘成 flat=3 直出用的顶点色。
///
/// 三段式（全部只用 n·常量，无逐帧量 → 结果对同一资产确定不变）：
/// ① 半兰伯特平方漫反射：`(0.5·N·L+0.5)²`。直接 `max(N·L,0)` 会把背光面全压成 0，
///    而枪身有一半的面法线背离主光；平方后的半兰伯特在 N·L=0 处仍有 0.25 且斜率
///    连续，明暗过渡不带"腰线"。
/// ② 天穹环境：按 `n.y` 线性插值 [GUN_AMB_MIN, GUN_AMB_MAX]，让朝上的面亮、
///    朝下的面暗（旧实现的 0.85 常数底噪正是把梯度抹平的东西）。
/// ③ 金属高光：`(N·H)^26`，H 为"主光 + 镜头方向"的半角向量。镜头方向在烘焙局部系
///    里是常量 -Z（fp_gun_matrix 的 rotY(π) 把局部 +Z 转到屏幕深处），所以可以烘。
fn fp_gun_bake_color(n: glam::Vec3, raw: [f32; 3], albedo_boost: f32) -> [f32; 3] {
    let key = GUN_KEY_DIR.normalize();
    // 局部系里指向镜头的方向恒定（viewmodel 钉在屏幕上），故高光可烘焙
    let half = (key + glam::Vec3::new(0.0, 0.0, -1.0)).normalize();
    let ndl = n.dot(key);
    let wrap = 0.5 * ndl + 0.5;
    let diff = wrap * wrap;
    let sky = (0.5 + 0.5 * n.y).clamp(0.0, 1.0);
    let amb = GUN_AMB_MIN + (GUN_AMB_MAX - GUN_AMB_MIN) * sky;
    let spec = n.dot(half).max(0.0).powf(GUN_SPEC_POWER) * GUN_SPEC_GAIN;
    let shade = amb + GUN_DIFF_GAIN * diff + spec;
    let a = [
        raw[0] * albedo_boost,
        raw[1] * albedo_boost,
        raw[2] * albedo_boost,
    ];
    [
        (a[0] * shade).clamp(0.0, 1.0),
        (a[1] * shade).clamp(0.0, 1.0),
        (a[2] * shade).clamp(0.0, 1.0),
    ]
}

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
    /// 最近一次开火时刻（anim_clock）。2026-09-01 起枪模后坐改由 `gun_sway.kick`
    /// 的连续指数包络驱动（旧写法直接按此值算 `(1-t)²` 抛物线 + 0.30 s 硬截止，
    /// 连发时每个周期都有位置阶跃）。本字段现在只被写入，留作击发时刻的观测点
    #[allow(dead_code)] // 只写不读：留给后坐/射速诊断挂点，删掉会连带三处写入语义丢失
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
            gun_sway: GunSway::new(),
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
                // 2026-08-28 实测：128v128 无上限 311fps @99% GPU；默认 300（LLM 120）
                let cap = env_f32("RV3D_FPS").unwrap_or(if llm_on { 120.0 } else { 300.0 });
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
            gun_glbs: std::collections::HashMap::new(),
            prop_set: None,
            // 哨兵：保证第一帧就上传一次道具几何
            prop_map_gen: u64::MAX,
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

        // 第一人称枪摆动状态积分（2026-09-01）。
        // 速度取自**玩家脚底的实际位移 / dt**，不用 `game.player_speed()`：后者读
        // PlayerBody.vel，而玩家移动走的是 `PlayerBody::try_move()`（直接改 pos，
        // 从不写 vel）→ vel 恒为 0 → 旧摆动分支实际上一次都没执行过。这里改成
        // 自带估计后，摆动不再依赖那个通道（game.rs/physics.rs 的 vel 修复另行提出）。
        {
            let p = self.game.player_pos();
            // dt 下限 1e-4 s：卡帧后 delta_time 被 clamp 到 0.1，正常帧约 6 ms；
            // 只有 0（同一时刻重复调用）会越过硬下限，此时按静止处理
            let dt = delta_time.max(1e-4);
            // 前向取**水平投影后归一化**：与 game.rs move_first_person 计算位移用的
            // 同一个基向量一致（俯仰时 forward 含 y 分量，直接点乘水平位移会低估前向
            // 速度，抬头/低头时前向摆动会莫名变小）。pitch 被 clamp 在 ±89°，
            // 水平分量最小 cos(89°)=0.0175，归一化不会退化。
            let f = self.camera.forward();
            let fwd = glam::Vec3::new(f.x, 0.0, f.z).normalize_or_zero();
            let right = self.camera.right();
            self.gun_sway.tick(dt, p, right, fwd, fired > 0);
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
    /// 导入枪模（按武器 key 自动寻找 assets/guns/{key}.glb；不存在回退 ak12.glb）
    /// 2026-08-28 终局：使用原始模型材质本色（baseColorFactor 直出 × 忠实现光）
    fn load_gun_glb(key: &str) -> Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)> {
        let path = if std::path::Path::new(&format!("assets/guns/{key}.glb")).exists() {
            format!("assets/guns/{key}.glb")
        } else {
            "assets/guns/ak12.glb".to_string()
        };
        let path: &str = &path;
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
                let (align, align_name) = if ext[1] >= ext[0] && ext[1] >= ext[2] {
                    // 长轴=Y（Sketchfab Z-up 竖立枪）：-90°X 立正 + 180°Z 滚转（枪顶朝上、弹匣朝下）
                    (
                        glam::Mat4::from_rotation_z(std::f32::consts::PI)
                            * glam::Mat4::from_rotation_y(std::f32::consts::PI)
                            * glam::Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                        "Y-long",
                    )
                } else if ext[0] >= ext[1] && ext[0] >= ext[2] {
                    (glam::Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2), "X-long")
                } else {
                    // 长轴已在 Z = 资产已是规范化朝向（枪口 +Z）。由 fp_gun_matrix 的
                    // rotY(π) 把局部 +Z 转到视空间 -Z，再经 view_inv 即相机前方 —— 与
                    // `gun_mesh_by_key` 文档约定的「局部枪口朝 +Z」一致，故不需任何旋转。
                    (glam::Mat4::IDENTITY, "IDENTITY")
                };
                let center = [
                    (mn[0] + mx[0]) * 0.5,
                    (mn[1] + mx[1]) * 0.5,
                    (mn[2] + mx[2]) * 0.5,
                ];
                // 注意：不在此处做任何相机空间变换——FP 帧内与程序化枪共用 fp_gun_matrix
                // （view_inv × anchor × scale；世界空间 + 每帧跟随相机）
                // 基色可用性判定：先看整份网格最亮的顶点有多亮。GLB 解析器
                // （engine/assets.rs）不读 baseColorTexture，只把 baseColorFactor 摊到
                // 顶点色上，所以"贴图丢了只剩暗调色因子"的资产会给出近乎纯黑的基色
                // （ak12.glb 实测 0.057/0.077）。此时按**全局最大值**归一到参考反照率：
                // 保留各材质之间的相对明暗差（0.057 与 0.077 差 34%，归一后仍差 34%），
                // 只是把它们整体抬到可分辨的亮度；正常资产（luma ≥ 0.18）不改变倍率。
                let mut luma_max = 0.0f32;
                for v in &mesh.verts {
                    let l = 0.2126 * v[8] + 0.7152 * v[9] + 0.0722 * v[10];
                    luma_max = luma_max.max(l);
                }
                let albedo_boost = if luma_max > 1e-5 && luma_max < GUN_DARK_LUMA {
                    GUN_REF_ALBEDO / luma_max
                } else {
                    1.0
                };
                // 每个武器 key 只加载一次（结果按 key 缓存），所以这行日志不会刷屏。
                // 它把两个"错了只会表现为枪看起来怪、不会报错"的决定摊开给人看：
                // ① align 走了哪条分支——assets/guns 里由 tools/install_guns.py 安装的
                //    资产是**规范化过的**（枪口 +Z、上 +Y、最长边 1.0），必然走 IDENTITY；
                //    若哪天它走了别的分支，说明有资产没经过预处理就进来了，朝向是猜的。
                // ② albedo_boost 是否为 1.0——不为 1 说明该资产基色过黑、走了亮度归一。
                log::info!(
                    "gun-glb: {key} ← {path} 顶点={} 索引={} 跨度=({:.2},{:.2},{:.2}) \
                     align={align_name} luma_max={:.3} albedo_boost={:.2}",
                    mesh.verts.len(),
                    mesh.indices.len(),
                    ext[0], ext[1], ext[2],
                    luma_max, albedo_boost
                );
                let verts: Vec<crate::engine::meshgen::GVertex> = mesh
                    .verts
                    .iter()
                    .map(|v| {
                        let mut p = glam::Vec3::new(v[0] - center[0], v[1] - center[1], v[2] - center[2]) * scale;
                        let mut n = glam::Vec3::from_slice(&v[3..6]);
                        p = align.transform_point3(p);
                        n = align.transform_vector3(n).normalize_or_zero();
                        let c = fp_gun_bake_color(n, [v[8], v[9], v[10]], albedo_boost);
                        crate::engine::meshgen::GVertex {
                            pos: [p.x, p.y, p.z],
                            normal: [n.x, n.y, n.z],
                            uv: [v[6], v[7]],
                            color: c,
                        }
                    })
                    .collect();
                log::info!(
                    "assets: 导入枪模 {path}（{} 顶点 / {} 索引，基色亮度 {:.3} → 反照率增益 ×{:.2}，首色 {:?}）",
                    verts.len(),
                    mesh.indices.len(),
                    luma_max,
                    albedo_boost,
                    verts.first().map(|v| v.color)
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
        // 导入枪模优先（按当前武器 key 缓存；检视与第一人称共用）
        let gkey = self.game.active_weapon_key().to_string();
        let load = |k: &String| Self::load_gun_glb(k);
        let entry = self.gun_glbs.entry(gkey.clone()).or_insert_with(|| load(&gkey));
        if let Some((verts, indices)) = entry.clone() {
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
            // 第一人称：顶点已在加载时静态化到「视空间基座」，每帧仅由实例矩阵驱动
            // （2026-08-28 残影修复：消除每帧 3MB CPU 重变换）
            return (verts, indices);
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
        // 程序化枪：返回**局部坐标**顶点，与导入 GLB 分支同一约定——矩阵只在
        // `render()` 里作为实例 model（fp_gun_pre）施加一次。
        // 2026-09-01 修复：这里曾先 `gun.transformed(fp_gun_matrix())` 把
        // view_inv×anchor×scale 烘进顶点，而调用方又把同一个矩阵当作实例 model 再乘
        // 一次 → 实际变换是 M·M·p。M 含 view_inv，M·M 里的第二个 view_inv 不会被 view
        // 抵消，枪会被再平移一次相机位置（城区坐标 ±215 m）→ 整支枪飞出画面/
        // 贴脸乱甩。该分支只在 GLB 缺失时命中（例如 release_dist/game/assets/guns
        // 未随包发布），所以平时看不见，一旦命中就是彻底坏掉。
        (gun.verts.clone(), gun.indices.clone())
    }
    /// 第一人称枪的世界空间矩阵（程序化/导入枪模共用）：view_inv × anchor × scale。
    /// 开火后坐 + 行走摆动 + ADS 插值 + FOV 缩放（2026-08-27 抽离共享；
    /// 2026-09-01 摆动/后坐全部改由 `gun_sway` 的连续状态量驱动，见该结构注释）
    fn fp_gun_matrix(&self) -> glam::Mat4 {
        let cam = &self.camera;
        let hip_pos = glam::Vec3::new(0.25, -0.20, -0.60);
        let ads_pos = glam::Vec3::new(0.0, -0.08, -0.42);
        let anchor_base = hip_pos.lerp(ads_pos, self.ads_blend);
        // 屏幕等幅归一化（2026-09-01，修 ADS 摆幅过大）：anchor 是**视空间**平移，
        // 它在屏幕上的位移 = offset / (锚距 × tan(fov/2))。开镜时锚距 0.60→0.42、
        // fov 70°→55°，两个因素叠起来把同样的 offset 视觉放大 1.92 倍；
        // `gun_scale` 下面已经做了 tan(fov/2) 补偿，平移量必须用同一套基准补偿，
        // 否则"模型不放大、摆动放大"→ 越是精确瞄准枪甩得越凶。
        let depth_gain = (-anchor_base.z) / GUN_HIP_DEPTH_M;
        let fov_gain = (cam.fov * 0.5).tan() / GUN_HIP_HALF_TAN;
        // 横向/垂向偏移的等幅因子；前后偏移只改变成像比例，用 depth_gain
        let screen_gain = depth_gain * fov_gain;
        let mut anchor = anchor_base;
        // ① 后坐：连续指数包络（击发帧置 1 后按 τ=75 ms 衰减）。
        //    旧实现用 (1-t)² 抛物线 + 0.30 s 硬截止：连发（10 发/秒）时每 0.1 s
        //    重新从 0.44 跳到 1.0，回落末端还有一次速度不连续 → 抖 + 残影。
        //    同样要走屏幕等幅补偿：0.07 m 的下蹲在开镜锚距 0.42 m + FOV 55° 下
        //    占半屏高 32%，而腰射只占 17% —— 不补偿时"开镜连发"就是玩家描述的
        //    "开镜射击下左右移动甩得特别大"里幅度最大的那个分量。
        //    视轴方向的前顶只改变成像比例，补偿因子是 depth（不含 tan(fov/2)）。
        let kick = self.gun_sway.kick;
        if kick > 0.0 {
            anchor.y -= 0.07 * kick * screen_gain;
            anchor.z += 0.05 * kick * depth_gain;
        }
        // ② 行走摆动。幅值 = smoothstep(速度) × 开火阻尼 × 诊断增益；三个因子全部
        //    连续，且 speed 已在 update() 里做过与帧率无关的低通，所以不存在
        //    "逐帧通断"的阶跃（那是旧实现高频残影的直接来源）。
        //    ADS 的按轴抑制放在下面各分量里，避免这里再乘一次造成双重衰减。
        let env = {
            let t = ((self.gun_sway.speed - GUN_SWAY_SPEED_LO)
                / (GUN_SWAY_SPEED_HI - GUN_SWAY_SPEED_LO))
                .clamp(0.0, 1.0);
            let smooth = t * t * (3.0 - 2.0 * t); // Hermite smoothstep：起止斜率为 0
            let fire_damp = 1.0 - (1.0 - GUN_SWAY_FIRE_DAMP) * kick;
            smooth * fire_damp * self.gun_sway.gain
        };
        if env > 1e-6 {
            let ads = self.ads_blend;
            let st = self.gun_sway.stride;
            let two = st * 2.0;
            // 侧向：1× 步频（一个完整步态周期回到原位一次）
            let sway = st.sin() * GUN_SWAY_SIDE_M * (1.0 - (1.0 - GUN_SWAY_ADS_SIDE) * ads);
            // 上下：2× 步频（每个落脚一次冲击），用 -cos 让 phase=0（刚落地）为最低点
            let bob = -two.cos() * GUN_SWAY_BOB_M * (1.0 - (1.0 - GUN_SWAY_ADS_BOB) * ads);
            // 前后：2× 步频、与落地错位 1/4 周期（手臂随步伐前后牵动）
            let fore = (two + std::f32::consts::FRAC_PI_2).sin()
                * GUN_SWAY_FORE_M
                * (1.0 - (1.0 - GUN_SWAY_ADS_FORE) * ads);
            // 侧向"惯性滞后"：与侧移速度反号、按饱和速度归一，最大 0.004 m
            let lean = -(self.gun_sway.strafe / GUN_SWAY_SPEED_HI).clamp(-1.0, 1.0)
                * GUN_SWAY_LEAN_M
                * (1.0 - (1.0 - GUN_SWAY_ADS_SIDE) * ads);
            anchor.x += (sway + lean) * screen_gain * env;
            anchor.y += bob * screen_gain * env;
            anchor.z += fore * depth_gain * env;
        }
        let base_scale = 0.50 - 0.03 * self.ads_blend;
        // 模型缩放与摆动偏移共用同一个 fov 补偿量（fov_gain），保证两条通道
        // 在腰射/开镜之间视觉一致（旧实现只有这里补了 fov，摆动没补）
        let gun_scale = fov_gain.clamp(0.5, 1.0) * base_scale;
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
        // 2026-08-28：枪实例矩阵预计算（进入 renderer 借用前——防借用冲突 + 每帧一次）
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
        // 2026-08-28：枪实例矩阵预计算（进入 renderer 借用前）
        let fp_gun_pre = {
            let show = self.inspect_weapon.is_some()
                || (self.game.state() == GameState::Playing
                    && self.camera.mode == CameraMode::FirstPerson);
            // 检视模式：顶点已居中到世界坐标，需用单位矩阵
            if self.inspect_weapon.is_some() { glam::Mat4::IDENTITY }
            else if show { self.fp_gun_matrix() } else { glam::Mat4::IDENTITY }
        };

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
            // 世界障碍 marker：关卡地图几何 → 按种类材质着色的实例（复用主 pipeline，
            // 见 renderer.rs MARKER_SLOT_BASE；模型矩阵/材质色统一由 WorldMarker::for_obstacle
            // 构建，与物理刚体 AABB（game.rs apply_level，同 half_w/half_d）严格同尺寸）。
            // 用 render_geometry() 而不是 map_obstacles()：后者只含会挡人的障碍，前者还包含
            // 挑檐/窗带/壁柱/屋顶设备这类纯装饰件（game.rs LevelMap::decor）。
            // 但要跳过 [`engine::geom::Shape::None`]：那是 GLB 道具的碰撞核，只参与物理与
            // 布局不变式检查，画出来会和 GLB 表面共面 z-fighting（正是 city.rs 零共面纪律
            // 禁止的那类穿帮）。过滤放在这里而不是 render_geometry() 内部，因为 city.rs 的
            // 布局测试需要遍历到每一个盒子。
            let markers: Vec<engine::renderer::WorldMarker> = self
                .game
                .render_geometry()
                .filter(|o| o.shape != engine::geom::Shape::None)
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
                    // 底盘配色：三通道等比缩放 → 色相/饱和度不变，归属色语义（蓝/红/灰）保持
                    let base_tint = [tint[0] * 0.8, tint[1] * 0.8, tint[2] * 0.8, 0.6];
                    let _ = id; // 标记 id 暂不绘制文字（HUD 已有 id 标签）
                    [
                        // 立柱（旗杆）
                        engine::renderer::WorldMarker {
                            model: glam::Mat4::from_translation(glam::Vec3::new(x, 2.0, z))
                                * glam::Mat4::from_scale(glam::Vec3::new(0.4, 4.0, 0.4)),
                            tint,
                        },
                        // 地面底盘（占领半径范围，半径 5.0 → scale 10.0）。
                        // D10 根因：旧值 y=0.08 + 厚 0.15 → 实体跨 y∈[0.005,0.155]，而地面实例
                        // 平面在 y=+0.05 正好从中间穿过，顶面只高出 ~10cm；玩家视线 ~1.7m 看
                        // 一层 10cm 的板几乎完全侧向（edge-on）→ 投影不足一像素 → 底盘"消失"，
                        // 据点读起来只剩两根电线杆。改为 0.5m 厚的低台（底面埋进地里 5cm 避免
                        // 与地形之间留缝），顶面离地 ~40cm，任何视角都能读出"这是一块领地"。
                        engine::renderer::WorldMarker {
                            model: glam::Mat4::from_translation(glam::Vec3::new(x, 0.20, z))
                                * glam::Mat4::from_scale(glam::Vec3::new(10.0, 0.5, 10.0)),
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
                let mut out = vec![engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(cx, 1.2, cz))
                        * glam::Mat4::from_scale(glam::Vec3::splat(fb_s)),
                    tint: [
                        1.0,
                        0.85 * (1.0 - fireball_t) + 0.2,
                        0.35 * (1.0 - fireball_t),
                        0.0, // tint.w = 火（build.rs 体积光晕分支选择器）
                    ],
                }];
                // ② 贴地冲击波环：扁球体（球体几何压扁）沿地面水平扩散 + 高度衰减，半透明
                let ring_s = r * (0.4 + 1.6 * t);
                let ring_h = (1.1 * (1.0 - t)).max(0.15);
                out.push(engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(cx, ring_h * 0.5, cz))
                        * glam::Mat4::from_scale(glam::Vec3::new(ring_s, ring_h, ring_s)),
                    tint: [1.0, 0.55 * (1.0 - t) + 0.15, 0.06, 0.0], // 火
                });
                // ③ 火柱：垂直拉长火舌从地面向上（0.5-2 寿命段），半透明
                let col_h = 2.2 + 2.6 * t;
                out.push(engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(cx, 1.1 + col_h * 0.5, cz))
                        * glam::Mat4::from_scale(glam::Vec3::new(r * 0.5, col_h, r * 0.5)),
                    tint: [1.0, 0.45 * (1.0 - t), 0.05, 0.0], // 火
                });
                // ④ 烟柱：暗色膨胀上浮（后段，营造爆炸余烟），半透明
                let smoke_s = r * (0.5 + 1.4 * t);
                let smoke_h = 2.0 + 3.0 * t;
                out.push(engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(cx, 0.6 + smoke_h * 0.5, cz))
                        * glam::Mat4::from_scale(glam::Vec3::new(smoke_s, smoke_h, smoke_s)),
                    tint: [0.16 * (1.0 - t) + 0.05, 0.13 * (1.0 - t) + 0.04, 0.1 * (1.0 - t) + 0.03, 1.0], // 烟
                });
                out
                })
                .collect();
            // 粒子（枪口焰/弹壳）转 emissive marker：枪口焰随 age 缩小淡出，弹壳保持小方块。
            // 自发光槽位只有 64 个（与 build.rs 的 EMISSIVE_INSTANCE_BASE + 64 严格同步）：
            // 128v128 压力下上百个 NPC 同时开火，按插入顺序截断会让「远处/将熄的焰」占坑、
            // 「近处的新焰」被丢弃 —— 玩家面前因此悬浮着几团本不该存在的琥珀色圆盘（D8）。
            // 策略：爆炸特效保底，剩余槽位按相机距离由近及远分配。
            const MAX_EMISSIVE: usize = 64;
            let eye = self.camera.position();
            // 预留：爆炸特效已入列的 + 紧随其后要画的手雷（数量很小），剩下的才给粒子
            let reserved = emissive_markers.len() + self.game.grenade_positions().len();
            let mut cand: Vec<(f32, engine::renderer::WorldMarker)> = self
                .particles
                .iter()
                .map(|p| {
                    let t = (p.age / p.life).clamp(0.0, 1.0);
                    let size = if p.kind == 0 {
                        p.size * (1.0 - t * 0.7) // 焰：快速收缩
                    } else {
                        p.size
                    };
                    let fade = 1.0 - t;
                    let d = (p.pos[0] - eye.x).powi(2)
                        + (p.pos[1] - eye.y).powi(2)
                        + (p.pos[2] - eye.z).powi(2);
                    (
                        d,
                        engine::renderer::WorldMarker {
                            model: glam::Mat4::from_translation(glam::Vec3::from(p.pos))
                                * glam::Mat4::from_scale(glam::Vec3::splat(size)),
                            tint: [
                        p.tint[0] * fade,
                        p.tint[1] * fade,
                        p.tint[2] * fade,
                        if p.kind == 0 { 0.0 } else { 1.0 }, // 焰=火，壳=固体
                    ],
                        },
                    )
                })
                .collect();
            cand.sort_by(|a, b| a.0.total_cmp(&b.0));
            emissive_markers.extend(
                cand.into_iter()
                    .take(MAX_EMISSIVE.saturating_sub(reserved))
                    .map(|(_, m)| m),
            );
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
            // ---- GLB 道具几何上传 ----
            // 套件懒加载一次（重载地图不必重新解析 24 个 GLB）；几何只在**地图代号变化**
            // 时重传：一次合并是百万级顶点的 CPU 拷贝，绝不能进每帧路径。
            // 读不到 assets/props 只意味着城市退回纯程序化外观，不是错误。
            if self.prop_set.is_none() {
                self.prop_set = Some(match engine::props::PropSet::load_dir("assets/props") {
                    Ok(s) => {
                        log::info!("props: 渲染侧载入 {} 件网格", s.len());
                        s
                    }
                    Err(e) => {
                        log::info!("props: 渲染侧未载入（{e}），不绘制道具");
                        Default::default()
                    }
                });
            }
            let map_gen = self.game.map_generation();
            if map_gen != self.prop_map_gen {
                self.prop_map_gen = map_gen;
                if let Some(set) = self.prop_set.as_ref() {
                    if !set.is_empty() {
                        renderer.set_props(set, self.game.prop_placements());
                    }
                }
            }
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
                renderer.set_first_person_gun_model(fp_gun_pre);
            } else {
                renderer.set_first_person_gun_mesh(&[], &[]);
                renderer.set_first_person_gun_model(fp_gun_pre);
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
            // PT 取景：与光栅化同一相机、同一太阳方向（路径追踪要当烘焙参照，参数必须同源）
            let lu = self.game.light_uniform();
            renderer.set_pt_params(crate::engine::ray_tracer::PtParams {
                cam: self.camera.position(),
                fwd: self.camera.forward(),
                tan_half_fov: (self.camera.fov * 0.5).tan(),
                bounces: 6,
                sun_dir: lu.directional.direction.truncate(),
                sun_color: lu.directional.color_intensity.truncate()
                    * lu.directional.color_intensity.w,
                exposure: 0.2,
            });
            // PT 场景 = 光栅化同一批 WorldMarker（盒集合变化时才重建 BLAS，指纹判定在渲染器内）
            if let Err(e) = renderer.pt_set_scene_markers(&markers) {
                log::warn!("PT-SCENE 失败: {e}");
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
            Ok(mut renderer) => {
                log::info!("Vulkan 渲染器初始化成功");
                // ---- RT core 纯求交吞吐基准（RV3D_PT_BENCH=1）----
                if std::env::var("RV3D_PT_BENCH").as_deref() == Ok("1") {
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
                if std::env::var("RV3D_PT_VIEW").as_deref() == Ok("1") {
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
            if crate::config::load().pt_enable {
                // RV3D_PT_SIZE：实时 PT 渲染分辨率（默认 1024；调高可验证 RT 通路真在算，
                // 也可为光照烘焙取更高分辨率参照帧）
                // 原生分辨率：直接对齐窗口物理尺寸（2560×1600！）；RV3D_PT_SIZE 单值覆盖（等比）
                let win_sz = window.inner_size();
                let def_w = win_sz.width.max(64);
                let def_h = win_sz.height.max(64);
                let pt_w = std::env::var("RV3D_PT_SIZE").ok().and_then(|v| v.parse::<u32>().ok())
                    .filter(|v| (128..=4096).contains(v) && v % 8 == 0).unwrap_or(def_w & !7);
                let pt_h = def_h & !7;
                if let Err(e) = renderer.init_pt_resident(pt_w, pt_h) {
                    log::info!("PT-RESIDENT init: {e}");
                }
            }
            let mut pt_on = crate::config::load().pt_enable;
            // RV3D_PT_LIVE: 0=强制关 1=强制开 未设=跟随配置
            if let Ok(v) = std::env::var("RV3D_PT_LIVE") {
                if v == "1" { pt_on = true; }
                else if v == "0" { pt_on = false; }
            }
            renderer.pt_live_enabled = pt_on;
            log::info!("RT: 路径追踪全景 = {}", if renderer.pt_live_enabled { "开启" } else { "关闭" });
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

#[cfg(test)]
mod tests {
    use super::*;

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
        s.tick(dt, pos, right, fwd, false);
        let step = speed * dt;
        for _ in 0..(secs / dt).round() as usize {
            pos += fwd * step;
            s.tick(dt, pos, right, fwd, false);
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
        );
        assert!(s.speed < 1e-3, "传送帧速度应归零，实际 {}", s.speed);
    }

    /// 后坐包络连续：击发后逐帧单调下降，单帧变化量 ≤8%
    /// （旧实现在 0.25 s 整点把阻尼从 0.15 阶跃到 1.0，单帧变化 0.85 = 位置跳变）
    #[test]
    fn gun_recoil_kick_decays_continuously() {
        let mut s = GunSway::new();
        let dt = 1.0 / 165.0;
        s.tick(dt, glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::NEG_Z, true);
        assert!(s.kick > 0.9, "击发帧应接近满幅后坐");
        let mut prev = s.kick;
        for _ in 0..60 {
            s.tick(dt, glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::NEG_Z, false);
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
 
