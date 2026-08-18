use crate::engine::meshgen::{beveled_box, cylinder, frustum, sphere, torus_arc};
use crate::engine::guns::{assemble, GunMesh, rz};
use glam::Mat4;

// Saiga-12：AK 系半自动霰弹枪，管式弹仓（枪管下方），圆木托/护木
pub fn saiga12() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let steel = [0.45, 0.48, 0.52];
    let dark_steel = [0.22, 0.24, 0.27];
    let black = [0.08, 0.08, 0.10];
    let wood = [0.45, 0.30, 0.16];
    let parts = vec![
        // 粗枪管（半径 0.019，长 0.47，z 0.46..0.93）
        (t(0.0, 0.0, 0.695) * rz(), cylinder(0.019, 0.47, 20), steel),
        // 枪口环
        (t(0.0, 0.0, 0.945) * rz(), cylinder(0.023, 0.03, 20), dark_steel),
        // 管式弹仓（枪管正下方）
        (t(0.0, -0.048, 0.68) * rz(), cylinder(0.0135, 0.40, 16), dark_steel),
        // 弹仓前端帽
        (t(0.0, -0.048, 0.895) * rz(), cylinder(0.016, 0.03, 16), dark_steel),
        // 准星
        (t(0.0, 0.045, 0.86), beveled_box(0.016, 0.045, 0.014, 0.004, 6), black),
        // 木质下护木
        (t(0.0, -0.014, 0.61), beveled_box(0.062, 0.048, 0.20, 0.01, 8), wood),
        // 导气管（枪管上方）
        (t(0.0, 0.038, 0.62) * rz(), cylinder(0.011, 0.20, 14), dark_steel),
        // 木质导气管罩
        (t(0.0, 0.048, 0.61), beveled_box(0.040, 0.026, 0.20, 0.008, 8), wood),
        // AK 式机匣
        (t(0.0, 0.0, 0.40), beveled_box(0.078, 0.095, 0.24, 0.015, 8), dark_steel),
        // 机匣顶盖
        (t(0.0, 0.055, 0.40), beveled_box(0.070, 0.012, 0.22, 0.005, 6), black),
        // 照门
        (t(0.0, 0.066, 0.455), beveled_box(0.018, 0.028, 0.018, 0.004, 6), black),
        // 拉机柄（右侧伸出）
        (t(0.058, 0.012, 0.335) * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2) * rz(), cylinder(0.007, 0.040, 12), dark_steel),
        // 拉机柄圆头
        (t(0.078, 0.012, 0.335) * Mat4::from_scale(glam::vec3(0.009, 0.009, 0.009)), sphere(10, 8), dark_steel),
        // 扳机护圈
        (t(0.0, -0.012, 0.42), torus_arc(0.028, 0.0045, 1.92, 7.50, 18, 8), dark_steel),
        // 扳机
        (t(0.0, -0.042, 0.425), beveled_box(0.014, 0.030, 0.010, 0.003, 6), black),
        // 木质手枪握把
        (t(0.0, -0.080, 0.435) * Mat4::from_rotation_x(0.10), beveled_box(0.034, 0.120, 0.052, 0.012, 8), wood),
        // 圆木托（AK 式，后倾下沉）
        (t(0.0, 0.012, 0.135) * Mat4::from_rotation_x(-0.16), beveled_box(0.046, 0.078, 0.27, 0.014, 8), wood),
        // 托底板
        (t(0.0, -0.012, 0.005), beveled_box(0.048, 0.080, 0.02, 0.006, 6), black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "Saiga-12 半自动霰弹枪", length: 0.96 }
}

// M1014：半自动破门霰弹枪，粗管 + 管式弹仓 + 伸缩托
pub fn m1014() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let steel = [0.45, 0.48, 0.52];
    let dark_steel = [0.22, 0.24, 0.27];
    let black = [0.08, 0.08, 0.10];
    let poly = [0.13, 0.14, 0.16];
    let parts = vec![
        // 粗枪管（半径 0.020，长 0.44，z 0.48..0.92）
        (t(0.0, 0.0, 0.70) * rz(), cylinder(0.020, 0.44, 20), steel),
        // 枪口环
        (t(0.0, 0.0, 0.935) * rz(), cylinder(0.024, 0.035, 20), dark_steel),
        // 管式弹仓
        (t(0.0, -0.046, 0.69) * rz(), cylinder(0.014, 0.42, 16), dark_steel),
        // 弹仓前端帽
        (t(0.0, -0.046, 0.905) * rz(), cylinder(0.017, 0.028, 16), black),
        // 聚合物护木
        (t(0.0, -0.005, 0.585), beveled_box(0.068, 0.058, 0.22, 0.012, 8), poly),
        // 护木顶部导轨
        (t(0.0, 0.034, 0.585), beveled_box(0.028, 0.010, 0.18, 0.003, 6), black),
        // 准星
        (t(0.0, 0.044, 0.855), beveled_box(0.016, 0.040, 0.014, 0.004, 6), black),
        // 机匣
        (t(0.0, 0.0, 0.345), beveled_box(0.070, 0.090, 0.26, 0.014, 8), dark_steel),
        // 机匣顶部导轨
        (t(0.0, 0.052, 0.345), beveled_box(0.032, 0.012, 0.16, 0.003, 6), black),
        // 觇孔照门（圆环）
        (t(0.0, 0.075, 0.415), torus_arc(0.015, 0.004, 0.0, 6.2832, 14, 6), black),
        // 扳机护圈
        (t(0.0, -0.012, 0.375), torus_arc(0.027, 0.0045, 1.92, 7.50, 18, 8), dark_steel),
        // 扳机
        (t(0.0, -0.040, 0.378), beveled_box(0.013, 0.028, 0.010, 0.003, 6), black),
        // 聚合物手枪握把
        (t(0.0, -0.082, 0.405) * Mat4::from_rotation_x(0.08), beveled_box(0.032, 0.115, 0.048, 0.010, 8), poly),
        // 缓冲管（伸缩托导轨基座）
        (t(0.0, 0.005, 0.15) * rz(), cylinder(0.016, 0.13, 16), dark_steel),
        // 伸缩托上杆
        (t(0.0, 0.035, 0.105), beveled_box(0.030, 0.030, 0.20, 0.008, 6), poly),
        // 伸缩托下杆
        (t(0.0, -0.028, 0.105), beveled_box(0.030, 0.028, 0.20, 0.008, 6), poly),
        // 伸缩托托底板
        (t(0.0, 0.005, -0.0175), beveled_box(0.055, 0.085, 0.035, 0.008, 6), poly),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "M1014 半自动霰弹枪", length: 0.99 }
}

// AA-12：全自动霰弹枪，重型枪身 + 箱式弹匣 + 粗管
pub fn aa12() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let steel = [0.45, 0.48, 0.52];
    let dark_steel = [0.22, 0.24, 0.27];
    let black = [0.08, 0.08, 0.10];
    let poly = [0.13, 0.14, 0.16];
    let parts = vec![
        // 重型枪管护罩（前细后粗，z 0.50..0.88）
        (t(0.0, 0.0, 0.69) * rz(), frustum(0.040, 0.033, 0.38, 20, true), dark_steel),
        // 枪口制退器（z 0.88..0.94）
        (t(0.0, 0.0, 0.91) * rz(), cylinder(0.043, 0.06, 20), dark_steel),
        // 枪口管口（z 0.94..0.985）
        (t(0.0, 0.0, 0.9625) * rz(), cylinder(0.019, 0.045, 20), steel),
        // 护罩顶部导轨
        (t(0.0, 0.047, 0.69), beveled_box(0.022, 0.012, 0.30, 0.004, 6), black),
        // 箱式弹匣（前倾）
        (t(0.0, -0.125, 0.375) * Mat4::from_rotation_x(-0.15), beveled_box(0.050, 0.16, 0.09, 0.010, 8), poly),
        // 弹匣底板
        (t(0.0, -0.200, 0.352) * Mat4::from_rotation_x(-0.15), beveled_box(0.054, 0.020, 0.10, 0.006, 6), black),
        // 厚重机匣
        (t(0.0, 0.0, 0.35), beveled_box(0.082, 0.105, 0.30, 0.016, 8), dark_steel),
        // 机匣顶部
        (t(0.0, 0.058, 0.35), beveled_box(0.040, 0.012, 0.22, 0.004, 6), black),
        // 准星（架在护罩上）
        (t(0.0, 0.062, 0.72), beveled_box(0.020, 0.040, 0.015, 0.004, 6), black),
        // 照门
        (t(0.0, 0.072, 0.48), beveled_box(0.022, 0.038, 0.018, 0.004, 6), black),
        // 拉机柄
        (t(0.0, 0.066, 0.45), beveled_box(0.030, 0.016, 0.05, 0.004, 6), black),
        // 抛壳口盖（右侧）
        (t(0.045, 0.0, 0.42), beveled_box(0.020, 0.040, 0.020, 0.004, 6), black),
        // 扳机护圈
        (t(0.0, -0.015, 0.40), torus_arc(0.030, 0.005, 1.92, 7.50, 18, 8), dark_steel),
        // 扳机
        (t(0.0, -0.045, 0.405), beveled_box(0.014, 0.030, 0.010, 0.003, 6), black),
        // 重型握把
        (t(0.0, -0.085, 0.435) * Mat4::from_rotation_x(0.06), beveled_box(0.038, 0.120, 0.055, 0.012, 8), poly),
        // 固定枪托（直托）
        (t(0.0, -0.005, 0.10), beveled_box(0.050, 0.075, 0.20, 0.012, 8), poly),
        // 枪托缓冲垫
        (t(0.0, -0.005, -0.005), beveled_box(0.055, 0.082, 0.02, 0.008, 6), black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "AA-12 全自动霰弹枪", length: 1.0 }
}
