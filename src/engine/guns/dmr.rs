use crate::engine::meshgen::{beveled_box, cylinder, frustum, sphere};
use crate::engine::guns::{assemble, GunMesh, rz};
use glam::Mat4;

pub fn svd12() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let rx = |a: f32| Mat4::from_rotation_x(a);
    let _ztox = || Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_2);
    let bright: [f32; 3] = [0.62, 0.65, 0.70];
    let dark: [f32; 3] = [0.30, 0.33, 0.37];
    let black: [f32; 3] = [0.16, 0.17, 0.19];
    let deep: [f32; 3] = [0.10, 0.10, 0.12];
    let wood: [f32; 3] = [0.50, 0.34, 0.17];
    let parts = vec![
        // 1 粗枪管
        (t(0.0, 0.045, 0.325) * rz(), cylinder(0.018, 0.45, 12), bright),
        // 2 枪口制退器
        (t(0.0, 0.045, 0.575) * rz(), frustum(0.022, 0.028, 0.05, 12, true), dark),
        // 3 导气管
        (t(0.0, 0.100, 0.28) * rz(), cylinder(0.011, 0.36, 16), dark),
        // 4 护木
        (t(0.0, 0.062, 0.20), beveled_box(0.056, 0.075, 0.28, 0.012, 3), black),
        // 5 准星
        (t(0.0, 0.085, 0.48), beveled_box(0.022, 0.055, 0.022, 0.008, 2), dark),
        // 6 表尺
        (t(0.0, 0.150, -0.14), beveled_box(0.032, 0.035, 0.060, 0.008, 2), dark),
        // 7 机匣
        (t(0.0, 0.085, -0.05), beveled_box(0.065, 0.085, 0.30, 0.012, 3), dark),
        // 8 镜桥
        (t(0.0, 0.150, -0.02), beveled_box(0.040, 0.022, 0.18, 0.008, 2), black),
        // 9 大瞄准镜身
        (t(0.0, 0.190, 0.05) * rz(), cylinder(0.022, 0.30, 12), deep),
        // 10 前端镜头
        (t(0.0, 0.190, 0.205) * rz(), frustum(0.027, 0.027, 0.02, 12, true), deep),
        // 11 后端镜头
        (t(0.0, 0.190, -0.105) * rz(), frustum(0.027, 0.027, 0.02, 12, true), deep),
        // 12 镜架前环
        (t(0.0, 0.168, 0.08), beveled_box(0.032, 0.028, 0.024, 0.008, 2), black),
        // 13 镜架后环
        (t(0.0, 0.168, -0.06), beveled_box(0.032, 0.028, 0.024, 0.008, 2), black),
        // 14 拉机柄杆
        (t(0.0485, 0.120, -0.15) * _ztox(), cylinder(0.008, 0.06, 16), bright),
        // 15 拉机柄球
        (t(0.079, 0.120, -0.15), sphere(10, 6), deep),
        // 16 10发弹匣（略前倾）
        (t(0.0, -0.02, 0.02) * rx(-0.22), beveled_box(0.042, 0.16, 0.10, 0.010, 3), black),
        // 17 握把
        (t(0.0, -0.03, -0.11) * rx(0.28), beveled_box(0.046, 0.14, 0.058, 0.012, 3), black),
        // 18 扳机护圈
        (t(0.0, 0.012, -0.05), beveled_box(0.056, 0.024, 0.115, 0.010, 2), deep),
        // 19 厚木托
        (t(0.0, 0.075, -0.37), beveled_box(0.056, 0.10, 0.34, 0.015, 3), wood),
        // 20 托腮凸起
        (t(-0.012, 0.140, -0.37), beveled_box(0.048, 0.05, 0.17, 0.010, 3), wood),
        // 21 斜切托底板
        (t(0.0, 0.080, -0.545) * rx(0.30), beveled_box(0.060, 0.12, 0.02, 0.008, 2), deep),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "SVD-12M 支点", length: 1.16 }
}

pub fn m110a1() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let rx = |a: f32| Mat4::from_rotation_x(a);
    let _ztox = || Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_2);
    let bright: [f32; 3] = [0.62, 0.65, 0.70];
    let dark: [f32; 3] = [0.30, 0.33, 0.37];
    let black: [f32; 3] = [0.16, 0.17, 0.19];
    let deep: [f32; 3] = [0.10, 0.10, 0.12];
    let sand: [f32; 3] = [0.48, 0.39, 0.26];
    let parts = vec![
        // 1 长粗枪管
        (t(0.0, 0.05, 0.31) * rz(), cylinder(0.017, 0.42, 12), bright),
        // 2 枪口制退器
        (t(0.0, 0.05, 0.545) * rz(), frustum(0.022, 0.026, 0.05, 12, true), bright),
        // 3 全长护木
        (t(0.0, 0.085, 0.25), beveled_box(0.058, 0.085, 0.30, 0.012, 3), sand),
        // 4 全长顶部导轨
        (t(0.0, 0.141, 0.165), beveled_box(0.050, 0.026, 0.71, 0.010, 3), black),
        // 5 上机匣
        (t(0.0, 0.09, -0.08), beveled_box(0.066, 0.080, 0.26, 0.012, 3), dark),
        // 6 下机匣
        (t(0.0, 0.033, -0.08), beveled_box(0.060, 0.062, 0.22, 0.012, 3), dark),
        // 7 拉机柄
        (t(0.0, 0.165, -0.235), beveled_box(0.052, 0.016, 0.028, 0.008, 2), deep),
        // 8 镜座
        (t(0.0, 0.169, 0.03), beveled_box(0.050, 0.030, 0.11, 0.010, 3), black),
        // 9 大瞄准镜身
        (t(0.0, 0.205, 0.03) * rz(), cylinder(0.022, 0.26, 12), deep),
        // 10 前端镜头
        (t(0.0, 0.205, 0.161) * rz(), frustum(0.025, 0.025, 0.018, 12, true), deep),
        // 11 后端镜头
        (t(0.0, 0.205, -0.101) * rz(), frustum(0.025, 0.025, 0.018, 12, true), deep),
        // 12 高低调节钮
        (t(0.0, 0.245, 0.03), cylinder(0.009, 0.035, 16), deep),
        // 13 前翻准星
        (t(0.0, 0.185, 0.48), beveled_box(0.022, 0.045, 0.02, 0.008, 2), deep),
        // 14 后翻照门
        (t(0.0, 0.183, -0.16), beveled_box(0.022, 0.040, 0.02, 0.008, 2), deep),
        // 15 导气块
        (t(0.0, 0.062, 0.42), beveled_box(0.034, 0.040, 0.05, 0.010, 2), dark),
        // 16 20发弹匣（略后倾）
        (t(0.0, -0.035, 0.005) * rx(0.15), beveled_box(0.046, 0.175, 0.085, 0.010, 3), black),
        // 17 握把
        (t(0.0, -0.01, -0.115) * rx(0.35), beveled_box(0.046, 0.135, 0.058, 0.012, 3), black),
        // 18 扳机护圈
        (t(0.0, -0.004, -0.055), beveled_box(0.052, 0.022, 0.10, 0.010, 2), deep),
        // 19 缓冲管
        (t(0.0, 0.055, -0.30) * rz(), cylinder(0.014, 0.22, 16), dark),
        // 20 可调托主体
        (t(0.0, 0.065, -0.47), beveled_box(0.056, 0.10, 0.24, 0.012, 3), sand),
        // 21 可调托腮板
        (t(0.0, 0.145, -0.47), beveled_box(0.050, 0.035, 0.15, 0.010, 2), sand),
        // 22 橡胶托垫
        (t(0.0, 0.065, -0.603), beveled_box(0.060, 0.115, 0.025, 0.010, 2), deep),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "M110A1 信使", length: 1.19 }
}

pub fn mk14p() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let rx = |a: f32| Mat4::from_rotation_x(a);
    let _ztox = || Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_2);
    let bright: [f32; 3] = [0.62, 0.65, 0.70];
    let dark: [f32; 3] = [0.30, 0.33, 0.37];
    let black: [f32; 3] = [0.16, 0.17, 0.19];
    let deep: [f32; 3] = [0.10, 0.10, 0.12];
    let wood: [f32; 3] = [0.50, 0.34, 0.17];
    let parts = vec![
        // 1 长粗枪管
        (t(0.0, 0.05, 0.32) * rz(), cylinder(0.017, 0.44, 12), bright),
        // 2 消焰器
        (t(0.0, 0.05, 0.57) * rz(), frustum(0.024, 0.028, 0.06, 12, true), dark),
        // 3 准星
        (t(0.0, 0.088, 0.48), beveled_box(0.030, 0.050, 0.024, 0.008, 2), dark),
        // 4 表尺
        (t(0.0, 0.185, -0.135), beveled_box(0.036, 0.034, 0.062, 0.010, 2), dark),
        // 5 导气筒
        (t(0.0, 0.028, 0.28) * rz(), cylinder(0.013, 0.30, 16), bright),
        // 6 导气筒帽
        (t(0.0, 0.028, 0.435) * rz(), frustum(0.014, 0.017, 0.02, 16, true), bright),
        // 7 木前托
        (t(0.0, 0.063, 0.23), beveled_box(0.058, 0.085, 0.34, 0.014, 3), wood),
        // 8 木托中段（弹匣口上方留出落差）
        (t(0.0, 0.068, -0.04), beveled_box(0.058, 0.062, 0.20, 0.014, 3), wood),
        // 9 厚木枪托
        (t(0.0, 0.075, -0.33), beveled_box(0.058, 0.11, 0.38, 0.015, 3), wood),
        // 10 托底板
        (t(0.0, 0.08, -0.531), beveled_box(0.062, 0.13, 0.022, 0.010, 2), deep),
        // 11 机匣
        (t(0.0, 0.125, -0.02), beveled_box(0.066, 0.08, 0.28, 0.012, 3), dark),
        // 12 拉机柄杆
        (t(0.066, 0.145, -0.02) * _ztox(), cylinder(0.008, 0.06, 16), bright),
        // 13 拉机柄头
        (t(0.099, 0.145, -0.02), sphere(10, 6), dark),
        // 14 20发直弹匣
        (t(0.0, -0.035, 0.04) * rx(0.06), beveled_box(0.052, 0.21, 0.09, 0.010, 3), dark),
        // 15 握把（木）
        (t(0.0, 0.02, -0.20) * rx(0.25), beveled_box(0.046, 0.10, 0.055, 0.012, 3), wood),
        // 16 扳机护圈
        (t(0.0, 0.015, -0.045), beveled_box(0.056, 0.030, 0.09, 0.010, 2), black),
        // 17 镜桥
        (t(0.0, 0.185, 0.02), beveled_box(0.042, 0.024, 0.18, 0.010, 3), black),
        // 18 大瞄准镜身
        (t(0.0, 0.225, 0.05) * rz(), cylinder(0.022, 0.28, 12), deep),
        // 19 前端镜头
        (t(0.0, 0.225, 0.191) * rz(), frustum(0.027, 0.027, 0.018, 12, true), deep),
        // 20 后端镜头
        (t(0.0, 0.225, -0.091) * rz(), frustum(0.027, 0.027, 0.018, 12, true), deep),
        // 21 镜架前环
        (t(0.0, 0.20, 0.08), beveled_box(0.036, 0.030, 0.03, 0.008, 2), black),
        // 22 镜架后环
        (t(0.0, 0.20, 0.0), beveled_box(0.036, 0.030, 0.03, 0.008, 2), black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "MK14P 仲裁者", length: 1.14 }
}
