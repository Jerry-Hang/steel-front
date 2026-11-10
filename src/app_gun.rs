// 由 src/lib.rs 拆出（2026-11-10，纯移动 + 仅加 pub(crate)）：枪模姿态相关的纯函数。
//
// 为什么单独成模块：`lib.rs` 6320→5993 行仍远超"单文件 ≤1200 行"的纪律，而这几支是
// **纯数学**（呼吸微摆轨迹、诊断开关、把材质基色+局部法线烘成顶点色），不碰渲染器与游戏状态，
// 搬出去不改变任何行为。
//
// 判据：tools/move_verify_normalized.py（只允许 pub(crate) 前缀这一处差异）。
use crate::{GUN_AMB_MAX, GUN_AMB_MIN, GUN_DIFF_GAIN, GUN_IDLE_HZ, GUN_KEY_DIR, GUN_SPEC_GAIN, GUN_SPEC_POWER};
use crate::env_f32;
use crate::GameApp;
use crate::{GUN_DARK_LUMA, GUN_REF_ALBEDO};
use crate::{
    GUN_HIP_DEPTH_M,
    GUN_HIP_HALF_TAN,
    GUN_IDLE_ROLL_RAD,
    GUN_IDLE_SWAY_M,
    GUN_RELOAD_DROP_M,
    GUN_RELOAD_PITCH_RAD,
    GUN_RELOAD_ROLL_RAD,
    GUN_SPRINT_DROP_M,
    GUN_SPRINT_PITCH_RAD,
    GUN_SPRINT_ROLL_RAD,
    GUN_SWAY_ADS_BOB,
    GUN_SWAY_ADS_FORE,
    GUN_SWAY_ADS_SIDE,
    GUN_SWAY_BOB_M,
    GUN_SWAY_FIRE_DAMP,
    GUN_SWAY_FORE_M,
    GUN_SWAY_LEAN_M,
    GUN_SWAY_SIDE_M,
    GUN_SWAY_SPEED_HI,
    GUN_SWAY_SPEED_LO,
    GUN_SWITCH_DROP_M,
    GUN_SWITCH_PITCH_RAD,
    GUN_SWITCH_ROLL_RAD,
};

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

impl GameApp {
    pub(crate) fn gun_glb_stem(key: &str) -> &str {
        match key {
            "ak12m" => "ak12",
            other => other,
        }
    }

    pub(crate) fn gun_walk_env(&self) -> f32 {
        let t = ((self.gun_sway.speed - GUN_SWAY_SPEED_LO)
            / (GUN_SWAY_SPEED_HI - GUN_SWAY_SPEED_LO))
            .clamp(0.0, 1.0);
        let smooth = t * t * (3.0 - 2.0 * t); // Hermite smoothstep：起止斜率为 0
        let fire_damp = 1.0 - (1.0 - GUN_SWAY_FIRE_DAMP) * self.gun_sway.kick;
        smooth * fire_damp * self.gun_sway.gain
    }

    /// 第一人称枪的世界空间矩阵（程序化/导入枪模共用）：view_inv × anchor × scale。
    /// 开火后坐 + 行走摆动 + ADS 插值 + FOV 缩放（2026-08-27 抽离共享；
    /// 2026-09-01 摆动/后坐全部改由 `gun_sway` 的连续状态量驱动，见该结构注释）
    pub(crate) fn fp_gun_matrix(&self) -> glam::Mat4 {
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
        let env = self.gun_walk_env();
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
        // ④ 切枪动作（2026-09-15）：包络 sin(π·t) 两端为 0 ⇒ 起止速度连续。
        //    只在真的在切枪时非零：switch_time == 0 时 progress 直接是 1.0 ⇒ sin(π)=0。
        let (switch_drop, switch_pitch, switch_roll) = {
            let t = self.game.weapon_switch_progress();
            let swing = (std::f32::consts::PI * t).sin();
            (
                GUN_SWITCH_DROP_M * swing * screen_gain,
                GUN_SWITCH_PITCH_RAD * swing,
                GUN_SWITCH_ROLL_RAD * swing,
            )
        };
        anchor.y -= switch_drop;
        // ⑤ 冲刺姿态（2026-09-26 补）：平滑混合到"压低 + 前倾 + 内收"。
        //    混合量来自 `GunSway::sprint`（帧率无关低通），所以按住/松开 Shift 都是渐变。
        let sprint = self.gun_sway.sprint;
        anchor.y -= GUN_SPRINT_DROP_M * sprint * screen_gain;
        // ⑥ 换弹动作（2026-09-26 补）：包络中点最大、两端为 0（见 `reload_envelope` 的测试）
        let reload = crate::engine::game::reload_envelope(
            1.0 - self.game.hud.reload_progress,
        );
        anchor.y -= GUN_RELOAD_DROP_M * reload * screen_gain;
        // ⑦ 静止呼吸微摆（2026-09-26 补）：只在几乎不动时出现，开镜时再压到 1/4。
        //    用 `anim_clock` 当相位（已有、单调）⇒ 不需另加计时器；幅值与姿态无关，
        //    走 `screen_gain` 保持与其它通道的屏幕等幅。
        let idle_w = (1.0 - env).clamp(0.0, 1.0) * (1.0 - 0.75 * self.ads_blend);
        let (idle_x, idle_y) = idle_sway(self.anim_clock);
        anchor.x += idle_x * GUN_IDLE_SWAY_M * screen_gain * idle_w;
        anchor.y += idle_y * GUN_IDLE_SWAY_M * screen_gain * idle_w;
        let view_inv = cam.view_matrix().inverse();
        view_inv
            * glam::Mat4::from_translation(anchor)
            * glam::Mat4::from_rotation_z(
                switch_roll + GUN_SPRINT_ROLL_RAD * sprint + GUN_RELOAD_ROLL_RAD * reload
                    + GUN_IDLE_ROLL_RAD * idle_x * idle_w,
            )
            * glam::Mat4::from_scale(glam::Vec3::splat(gun_scale))
            * glam::Mat4::from_rotation_x(
                -0.045
                    + switch_pitch
                    + GUN_SPRINT_PITCH_RAD * sprint
                    + GUN_RELOAD_PITCH_RAD * reload,
            )
            * glam::Mat4::from_rotation_y(std::f32::consts::PI)
    }
}

impl GameApp {
    pub(crate) fn first_person_gun_mesh(&mut self) -> (Vec<crate::engine::meshgen::GVertex>, Vec<u32>) {
        // 导入枪模优先（按武器 key 缓存；检视与第一人称共用）
        //
        // 🔴 **检视模式下必须按"要检视的那把枪"取 key，不能取当前槽位**
        // （2026-10-15，外部复核指出第 5 条，已对着代码确认）：
        // 原来这里无条件 `self.game.active_weapon_key()`。而检视模式**从不**
        // `switch_weapon`（`update()` 只设 `camera.mode = Orbit` 并按
        // `spec_by_number(n)` 的包围盒算取景距离）⇒ `active_weapon_key()` 恒为 0 号槽
        // ⇒ **`--inspect=2` 和 `--inspect=35` 画出来的都是 1 号枪**，而相机距离却按
        // 第 N 号枪算 ⇒ 枪在画面里大小也不对。
        //
        // 更要紧的是：下面那个真正读 `self.inspect_weapon` 的分支
        // （"检视模式：枪模放世界原点上方"）**只有 GLB 加载失败时才可达** ——
        // 而 `load_gun_glb` 有 `ak12.glb` 兜底、几乎不会失败
        // ⇒ 那段代码实际上**从来没被执行过**（死分支）。
        let gkey = match self.inspect_weapon {
            Some(n) => crate::engine::weapon_data::spec_by_number(n)
                .map(|s| s.key.to_string())
                .unwrap_or_else(|| self.game.active_weapon_key().to_string()),
            None => self.game.active_weapon_key().to_string(),
        };
        let load = |k: &String| Self::load_gun_glb(k);
        // 只在某把枪**首次**入缓存时打一次几何探针，避免每帧刷屏。
        // 用途：把"枪到底朝屏幕深处还是朝右"从肉眼判断变成一个数——把枪身局部包围盒
        // 的顶点经真实 fp_gun_matrix 变到世界后，分别投影到相机前向与相机右向，
        // 跨度大的那个才是枪身实际躺着的方向。肉眼已三次给出互相矛盾的结论。
        let first_load = !self.gun_glbs.contains_key(&gkey);
        // 先 clone 出来结束对 self.gun_glbs 的可变借用，否则下面没法再 &self 取矩阵/相机
        let loaded = self
            .gun_glbs
            .entry(gkey.clone())
            .or_insert_with(|| load(&gkey))
            .clone();
        if first_load {
            if let Some((verts, _)) = &loaded {
                if verts.len() > 8 {
                    let m = self.fp_gun_matrix();
                    // 输出**世界空间 AABB 各轴跨度**而不是"沿相机轴的投影长度"：
                    // 后者要正确解释相机基向量的约定，我上一轮就是被这种间接量误导过。
                    // 世界 AABB 无法误读——枪指入屏幕则 z 跨度大，横躺则 x 跨度大。
                    let mut lo = [f32::MAX; 3];
                    let mut hi = [f32::MIN; 3];
                    for v in verts.iter() {
                        let w = m.transform_point3(glam::Vec3::from_slice(&v.pos));
                        for k in 0..3 {
                            lo[k] = lo[k].min(w[k]);
                            hi[k] = hi[k].max(w[k]);
                        }
                    }
                    let f = self.camera.forward();
                    let r = self.camera.right();
                    log::info!(
                        "gun-orient: {gkey} 世界AABB跨度 x={:.3} y={:.3} z={:.3} \
                         | 相机forward=({:.2},{:.2},{:.2}) right=({:.2},{:.2},{:.2})",
                        hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2],
                        f.x, f.y, f.z, r.x, r.y, r.z
                    );
                }
            }
        }
        // 下面统一用已取出的 `loaded`，不再持有对 self.gun_glbs 的借用
        // （函数体后面还要读 self.inspect_weapon / self.ads_blend 等字段）。
        if let Some((verts, indices)) = loaded {
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

    pub(crate) fn load_gun_glb(key: &str) -> Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)> {
        // 🔬 A/B 开关（2026-10-18）：`RV3D_NO_GUN_GLB=1` 强制走**程序化枪模**。
        // 存在理由：第一人称一直只验过 GLB 枪模（AK-12M 等 16 把），而**无 GLB 的 19 把**
        // 走的是 `meshgen` 程序化图元 + `gun_pipeline`（`cull_mode = BACK` + 无深度）——
        // 这条路**从未有过实机画面证据**。有了这个开关就能在不切枪的前提下拍到它。
        // 与 `RV3D_NO_SHADOW` / `RV3D_NO_MESH` / `RV3D_NO_DECALS` 同类，仅诊断用。
        if crate::syscfg::parse::<u8>("RV3D_NO_GUN_GLB") == Some(1) {
            return None;
        }
        // 🎯 缺 GLB 时的回退顺序（2026-10-31 改）：**自己的程序化模型优先，其次才是 ak12**。
        // 旧写法一律借用 `ak12.glb`，于是许可清理后被移出仓库的 8 把枪（ak104 / ash12 / pkm /
        // pkp / rpk16 / saiga12 / sv98 / svd12）**全变成 AK-12 的外形**——玩家会以为换枪没生效。
        // 而这 35 把枪在 `engine::guns` 里**本来就有各自的程序化模型**（`has_procedural_mesh`
        // 不建网格即可判定），所以这里返回 `None`，让调用方走那条路。
        let stem = Self::gun_glb_stem(key);
        let path = if crate::engine::asset_source::global()
            .exists(&format!("assets/guns/{stem}.glb"))
        {
            format!("assets/guns/{stem}.glb")
        } else if crate::engine::guns::has_procedural_mesh(key) {
            log::info!("assets: {key} 无 GLB ⇒ 使用本键程序化枪模（不再借用 ak12）");
            return None;
        } else {
            log::info!("assets: {key} 既无 GLB 也无程序化模型 ⇒ 借用 ak12.glb");
            "assets/guns/ak12.glb".to_string()
        };
        let path: &str = &path;
        // 🔴 **读取也必须走资产抽象层**（2026-10-15，外部复核指出第 5 条，已确认）：
        // 上面那句"存不存在"用的是 `asset_source::global().exists(..)`（Android 走
        // `AAssetManager`、`RV3D_ASSETS_FROM_MEMORY=1` 走 `MemSource`），
        // **而这里原来调的是 `std::fs::read`** ⇒ 在 Android 上
        // `exists()` 在 APK 里找得到、`std::fs::read` 却必然失败
        // ⇒ **所有 GLB 枪模静默退回程序化枪模**（玩家看到"枪变样了"，日志里只有一句
        // 误导性的"未发现"）。
        //
        // 仓库里另外两处是对的（`map.rs::load_map` 走 `read_to_string`、
        // `renderer/helpers.rs::load_spirv` 走 `read`）—— **只有这一处只改了一半**。
        let bytes = match crate::engine::asset_source::global().read(path) {
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
}
