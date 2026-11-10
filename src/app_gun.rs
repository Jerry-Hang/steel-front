// 由 src/lib.rs 拆出（2026-11-10，纯移动 + 仅加 pub(crate)）：枪模姿态相关的纯函数。
//
// 为什么单独成模块：`lib.rs` 6320→5993 行仍远超"单文件 ≤1200 行"的纪律，而这几支是
// **纯数学**（呼吸微摆轨迹、诊断开关、把材质基色+局部法线烘成顶点色），不碰渲染器与游戏状态，
// 搬出去不改变任何行为。
//
// 判据：tools/move_verify_normalized.py（只允许 pub(crate) 前缀这一处差异）。
use crate::{GUN_AMB_MAX, GUN_AMB_MIN, GUN_DIFF_GAIN, GUN_IDLE_HZ, GUN_KEY_DIR, GUN_SPEC_GAIN, GUN_SPEC_POWER};

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
