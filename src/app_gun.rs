// 由 src/lib.rs 拆出（2026-11-10，纯移动 + 仅加 pub(crate)）：枪模姿态相关的纯函数。
//
// 为什么单独成模块：`lib.rs` 6320→5993 行仍远超"单文件 ≤1200 行"的纪律，而这几支是
// **纯数学**（呼吸微摆轨迹、诊断开关、把材质基色+局部法线烘成顶点色），不碰渲染器与游戏状态，
// 搬出去不改变任何行为。
//
// 判据：tools/move_verify_normalized.py（只允许 pub(crate) 前缀这一处差异）。
use crate::{GUN_AMB_MAX, GUN_AMB_MIN, GUN_DIFF_GAIN, GUN_IDLE_HZ, GUN_KEY_DIR, GUN_SPEC_GAIN, GUN_SPEC_POWER};
use crate::env_f32;
use crate::{GUN_GAIT_MAX_RATE, GUN_GAIT_PHASE_PER_M, GUN_RECOIL_TAU, GUN_SWAY_SMOOTH_TAU};

/// 静止呼吸微摆的**单位轨迹**（纯函数，可单测）：返回两轴偏移，各自在 `[-1, 1]`。
///
/// x 走 1× 频率、y 走 2× 且错开相位 ⇒ 枪口画一个极扁的"∞"字（而不是一条来回直线）。
/// 有界性由 `sin` 保证，且**不含任何累积量** ⇒ 玩多久都不会漂（与 `GunSway::stride`
/// 回绕是同一类考虑：相位必须有界）。
pub(crate) fn idle_sway(clock: f32) -> (f32, f32) {
    let w = clock * std::f32::consts::TAU * GUN_IDLE_HZ;
    (w.sin(), (w * 2.0).sin() * 0.6)
}

/// `RV3D_GUN_DIAG=1`：每秒打一行枪姿态诊断（见 `fp_gun_matrix` 调用点的注释）。
/// 关掉时零成本（只多一次已缓存的 bool 比较）。
pub(crate) fn gun_diag_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        crate::syscfg::flag("RV3D_GUN_DIAG")
    })
}

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
pub(crate) fn fp_gun_bake_color(n: glam::Vec3, raw: [f32; 3], albedo_boost: f32) -> [f32; 3] {
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
pub(crate) struct GunSway {
    /// 上一帧玩家脚底世界坐标（用真实位移求速度，见 `update()` 里的说明）；
    /// None = 尚未播种（启动首帧、瞬移/重生之后），此时不按位移伪造速度
    pub(crate) prev_pos: Option<glam::Vec3>,
    /// 平滑后的水平速度模长（m/s）→ 只驱动幅值包络
    pub(crate) speed: f32,
    /// 平滑后的侧向速度分量（+ = 向右），→ 驱动侧向"惯性滞后"偏移
    pub(crate) strafe: f32,
    /// 平滑后的前向速度分量（+ = 前进），→ 驱动前后拖拽偏移
    pub(crate) fore: f32,
    /// 步态相位（弧度，恒定回绕到 [0, 2π)）
    pub(crate) stride: f32,
    /// 后坐冲量包络 [0,1]：开火帧置 1，其后按 exp(-dt/τ) 连续衰减
    pub(crate) kick: f32,
    /// 冲刺姿态混合 [0,1]：按住 Shift 且真的在跑时收敛到 1，否则回 0。
    /// 与速度同一套帧率无关低通 ⇒ 起止无阶跃（冲刺姿态不许"啪"地跳出来）。
    pub(crate) sprint: f32,
    /// 摆动总增益（RV3D_GUN_SWAY 覆盖，缺省 1.0；=0 可完全关闭摆动做 A/B 判定）
    pub(crate) gain: f32,
}

impl GunSway {
    /// 重开一局时清掉摆动/后坐的动态量（`gain` 是环境变量配置，保留）。
    pub(crate) fn reset_for_new_run(&mut self) {
        self.prev_pos = None;
        self.speed = 0.0;
        self.strafe = 0.0;
        self.fore = 0.0;
        self.stride = 0.0;
        self.kick = 0.0;
        self.sprint = 0.0;
    }

    pub(crate) fn new() -> Self {
        Self {
            prev_pos: None,
            speed: 0.0,
            strafe: 0.0,
            fore: 0.0,
            stride: 0.0,
            kick: 0.0,
            sprint: 0.0,
            // 诊断门（与项目其它 RV3D_* 一致）：RV3D_GUN_SWAY=0 关闭全部枪摆动
            gain: env_f32("RV3D_GUN_SWAY").unwrap_or(1.0).clamp(0.0, 3.0),
        }
    }

    /// 下一帧不按位移伪造速度（重生/传送/切模式后用；`tick()` 会自动重新播种）
    #[allow(dead_code)] // 预留：game.rs 侧提供显式传送事件时调用（当前由 2 m 瞬移保护兜底）
    pub(crate) fn reset_motion(&mut self) {
        self.speed = 0.0;
        self.strafe = 0.0;
        self.fore = 0.0;
        self.prev_pos = None;
    }

    /// 每帧积分：`now_pos` 为玩家本帧脚底世界坐标，`right`/`fwd` 为相机基向量，
    /// `fired` 为本帧是否击发，`sprinting` 为本帧是否处于冲刺（姿态包络目标），
    /// `dt` 为本次 update 的帧时间（秒）。
    pub(crate) fn tick(
        &mut self,
        dt: f32,
        now_pos: glam::Vec3,
        right: glam::Vec3,
        fwd: glam::Vec3,
        fired: bool,
        sprinting: bool,
    ) {
        // 冲刺姿态低通：与速度用同一个 τ（帧率无关）。**放在瞬移保护之外** ——
        // 重生/传送不该抹掉"我正按着 Shift 跑"这个姿态目标。
        let sprint_a = 1.0 - (-dt / GUN_SWAY_SMOOTH_TAU).exp();
        self.sprint += ((sprinting as u32 as f32) - self.sprint) * sprint_a;
        if self.sprint < 1e-4 {
            self.sprint = 0.0; // 收敛到精确 0：姿态项彻底不参与（也保证 A/B 可判定）
        }
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
