use crate::engine::meshgen::{beveled_box, cylinder, frustum, sphere, torus_arc};
use crate::engine::guns::{assemble, GunMesh, rz};
use glam::Mat4;

/// MP-443 乌鸦：9×19 双动手枪，厚实套筒 + 塑料粗握把
pub fn mp443() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let tz = |x: f32, y: f32, z: f32| t(x, y, z) * rz();
    let rx = |x: f32, y: f32, z: f32, a: f32| t(x, y, z) * Mat4::from_rotation_x(a);

    let steel = [0.62, 0.65, 0.70];
    let dsteel = [0.30, 0.33, 0.37];
    let poly = [0.16, 0.17, 0.19];
    let black = [0.10, 0.10, 0.12];

    let parts = vec![
        (t(0.0, 0.052, 0.02), beveled_box(0.034, 0.04, 0.2, 0.008, 2), steel),
        (tz(0.0, 0.025, 0.06), cylinder(0.013, 0.13, 12), dsteel),
        (tz(0.0, 0.025, 0.115), cylinder(0.0145, 0.016, 16), steel),
        (t(0.0, 0.017, 0.0), beveled_box(0.032, 0.028, 0.14, 0.008, 2), dsteel),
        (rx(0.0, -0.043, -0.05, 0.12), beveled_box(0.03, 0.115, 0.035, 0.008, 2), poly),
        (t(0.0, -0.085, -0.05), beveled_box(0.024, 0.062, 0.027, 0.004, 2), black),
        (t(0.0, -0.115, -0.05), beveled_box(0.03, 0.014, 0.033, 0.003, 2), black),
        (tz(0.0, -0.004, -0.048), torus_arc(0.024, 0.007, 3.30, 6.13, 8, 6), black),
        (rx(0.0, -0.02, -0.044, 0.15), beveled_box(0.009, 0.012, 0.007, 0.002, 2), dsteel),
        (t(0.0, 0.073, 0.085), beveled_box(0.009, 0.012, 0.005, 0.002, 2), dsteel),
        (t(0.0, 0.0725, -0.05), beveled_box(0.016, 0.009, 0.008, 0.002, 2), dsteel),
        (rx(0.0, 0.038, -0.083, 0.18), beveled_box(0.016, 0.02, 0.009, 0.002, 2), dsteel),
        (t(0.0, 0.073, -0.06), beveled_box(0.024, 0.005, 0.005, 0.002, 2), black),
        (t(0.0, 0.073, -0.068), beveled_box(0.024, 0.005, 0.005, 0.002, 2), black),
        (t(0.0175, 0.05, -0.01), beveled_box(0.004, 0.02, 0.042, 0.002, 2), black),
        (t(-0.0175, 0.03, -0.02), beveled_box(0.004, 0.016, 0.01, 0.002, 2), black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "MP-443 乌鸦", length: 0.21 }
}

/// RSh-12 撞锤：12.7×55 左轮，大转轮 + 制退器 + 木握把片
pub fn rsh12() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let tz = |x: f32, y: f32, z: f32| t(x, y, z) * rz();
    let rx = |x: f32, y: f32, z: f32, a: f32| t(x, y, z) * Mat4::from_rotation_x(a);

    let steel = [0.62, 0.65, 0.70];
    let dsteel = [0.30, 0.33, 0.37];
    let poly = [0.16, 0.17, 0.19];
    let black = [0.10, 0.10, 0.12];
    let wood = [0.50, 0.34, 0.17];

    let parts = vec![
        (t(0.0, 0.03, -0.03), beveled_box(0.042, 0.055, 0.2, 0.01, 2), dsteel),
        (tz(0.0, 0.032, 0.13), cylinder(0.016, 0.17, 12), steel),
        (tz(0.0, 0.032, 0.2415), frustum(0.021, 0.026, 0.055, 16, true), black),
        (t(0.0, 0.063, -0.045), beveled_box(0.04, 0.008, 0.09, 0.003, 2), dsteel),
        (t(0.0, 0.014, 0.09), beveled_box(0.03, 0.022, 0.14, 0.006, 2), steel),
        (tz(0.0, 0.045, -0.075), cylinder(0.035, 0.05, 12), steel),
        (t(0.0, 0.045, -0.046) * Mat4::from_scale(glam::vec3(0.004, 0.004, 0.004)), sphere(8, 6), steel),
        (rx(0.0, -0.03, -0.105, 0.28), beveled_box(0.032, 0.095, 0.038, 0.008, 2), poly),
        (rx(-0.019, -0.03, -0.103, 0.28), beveled_box(0.005, 0.078, 0.033, 0.002, 2), wood),
        (rx(0.019, -0.03, -0.103, 0.28), beveled_box(0.005, 0.078, 0.033, 0.002, 2), wood),
        (tz(0.0, -0.006, -0.055), torus_arc(0.028, 0.007, 3.30, 6.13, 8, 6), black),
        (rx(0.0, -0.024, -0.051, 0.15), beveled_box(0.01, 0.013, 0.008, 0.002, 2), black),
        (t(0.0, 0.052, 0.15), beveled_box(0.011, 0.016, 0.006, 0.002, 2), steel),
        (t(0.0, 0.072, -0.08), beveled_box(0.02, 0.014, 0.01, 0.002, 2), dsteel),
        (rx(0.0, 0.046, -0.128, 0.2), beveled_box(0.024, 0.034, 0.014, 0.003, 2), dsteel),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "RSh-12 撞锤", length: 0.40 }
}

/// M18 信标：9×19 紧凑手枪，沙色聚合物握把 + 亮钢套筒
pub fn m18() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let tz = |x: f32, y: f32, z: f32| t(x, y, z) * rz();
    let rx = |x: f32, y: f32, z: f32, a: f32| t(x, y, z) * Mat4::from_rotation_x(a);

    let steel = [0.62, 0.65, 0.70];
    let dsteel = [0.30, 0.33, 0.37];
    let black = [0.10, 0.10, 0.12];
    let sand = [0.48, 0.39, 0.26];

    let parts = vec![
        (t(0.0, 0.048, 0.02), beveled_box(0.03, 0.035, 0.145, 0.008, 2), steel),
        (tz(0.0, 0.026, 0.055), cylinder(0.011, 0.09, 16), steel),
        (t(0.0, 0.015, 0.01), beveled_box(0.029, 0.026, 0.13, 0.008, 2), sand),
        (rx(0.0, -0.035, -0.045, 0.12), beveled_box(0.026, 0.095, 0.032, 0.008, 2), sand),
        (t(0.0, -0.078, -0.045), beveled_box(0.022, 0.055, 0.025, 0.004, 2), black),
        (t(0.0, -0.105, -0.045), beveled_box(0.028, 0.013, 0.03, 0.003, 2), sand),
        (tz(0.0, -0.004, -0.05), torus_arc(0.021, 0.006, 3.30, 6.13, 8, 6), black),
        (rx(0.0, -0.019, -0.046, 0.15), beveled_box(0.008, 0.011, 0.006, 0.002, 2), black),
        (t(0.0, 0.066, 0.075), beveled_box(0.008, 0.011, 0.005, 0.002, 2), dsteel),
        (t(0.0, 0.066, -0.035), beveled_box(0.014, 0.009, 0.007, 0.002, 2), dsteel),
        (t(0.0155, 0.044, -0.005), beveled_box(0.004, 0.018, 0.035, 0.002, 2), black),
        (t(0.0, 0.0665, -0.042), beveled_box(0.02, 0.004, 0.005, 0.002, 2), black),
        (t(0.0, 0.0665, -0.049), beveled_box(0.02, 0.004, 0.005, 0.002, 2), black),
        (t(-0.0155, 0.028, 0.035), beveled_box(0.004, 0.012, 0.008, 0.002, 2), dsteel),
        (t(-0.0155, 0.024, -0.02), beveled_box(0.004, 0.012, 0.009, 0.002, 2), dsteel),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "M18 信标", length: 0.16 }
}

/// Mk23 海豹：.45 重型手枪，带螺纹枪管 + 皮卡汀尼导轨
pub fn mk23() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let tz = |x: f32, y: f32, z: f32| t(x, y, z) * rz();
    let rx = |x: f32, y: f32, z: f32, a: f32| t(x, y, z) * Mat4::from_rotation_x(a);

    let steel = [0.62, 0.65, 0.70];
    let dsteel = [0.30, 0.33, 0.37];
    let poly = [0.16, 0.17, 0.19];
    let black = [0.10, 0.10, 0.12];

    let parts = vec![
        (t(0.0, 0.052, 0.01), beveled_box(0.034, 0.042, 0.19, 0.008, 2), steel),
        (tz(0.0, 0.028, 0.065), cylinder(0.0135, 0.15, 12), steel),
        (tz(0.0, 0.028, 0.15), cylinder(0.011, 0.02, 16), dsteel),
        (t(0.0, 0.016, 0.0), beveled_box(0.031, 0.028, 0.15, 0.008, 2), dsteel),
        (rx(0.0, -0.036, -0.052, 0.12), beveled_box(0.03, 0.105, 0.038, 0.008, 2), poly),
        (t(0.0, -0.082, -0.052), beveled_box(0.027, 0.06, 0.03, 0.004, 2), poly),
        (t(0.0, -0.107, -0.052), beveled_box(0.033, 0.013, 0.034, 0.003, 2), poly),
        (tz(0.0, -0.005, -0.05), torus_arc(0.026, 0.0075, 3.30, 6.13, 8, 6), black),
        (rx(0.0, -0.021, -0.046, 0.15), beveled_box(0.01, 0.013, 0.008, 0.002, 2), black),
        (t(0.0, 0.0735, 0.085), beveled_box(0.009, 0.013, 0.006, 0.002, 2), dsteel),
        (t(0.0, 0.0735, -0.065), beveled_box(0.016, 0.011, 0.008, 0.002, 2), dsteel),
        (t(0.0, 0.0, 0.02), beveled_box(0.024, 0.009, 0.016, 0.002, 2), black),
        (t(0.0, 0.0, 0.05), beveled_box(0.024, 0.009, 0.016, 0.002, 2), black),
        (t(0.0, 0.0, 0.08), beveled_box(0.024, 0.009, 0.016, 0.002, 2), black),
        (t(0.017, 0.03, -0.015), beveled_box(0.004, 0.015, 0.012, 0.002, 2), dsteel),
        (rx(0.0, 0.044, -0.082, 0.15), beveled_box(0.02, 0.026, 0.01, 0.002, 2), dsteel),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "Mk23 海豹", length: 0.25 }
}
