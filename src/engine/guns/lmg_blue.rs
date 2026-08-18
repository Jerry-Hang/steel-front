use crate::engine::meshgen::{beveled_box, cylinder, frustum, sphere};
use crate::engine::guns::{assemble, GunMesh, rz};
use glam::Mat4;

pub fn m249() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let steel_dark = [0.22f32, 0.24, 0.27];
    let steel = [0.45f32, 0.48, 0.52];
    let black_poly = [0.13f32, 0.14, 0.16];
    let black = [0.08f32, 0.08, 0.10];
    let olive = [0.25f32, 0.32, 0.20];
    let parts = vec![
        // 机匣
        (t(0.0, 0.02, 0.18), beveled_box(0.09, 0.10, 0.36, 0.02, 2), steel_dark),
        // 上机匣盖
        (t(0.0, 0.075, 0.16), beveled_box(0.07, 0.02, 0.30, 0.005, 2), steel),
        // 枪管
        (t(0.0, 0.02, 0.565) * rz(), cylinder(0.016, 0.45, 20), steel),
        // 枪口消焰器
        (t(0.0, 0.02, 0.815) * rz(), frustum(0.019, 0.024, 0.05, 20, true), steel_dark),
        // 导气块
        (t(0.0, 0.02, 0.56), beveled_box(0.035, 0.035, 0.05, 0.005, 2), steel_dark),
        // 导气管
        (t(0.0, -0.012, 0.33) * rz(), cylinder(0.007, 0.46, 12), steel),
        // 方形护木
        (t(0.0, 0.01, 0.36), beveled_box(0.055, 0.065, 0.30, 0.012, 2), olive),
        // 提把横梁
        (t(0.0, 0.115, 0.15), beveled_box(0.035, 0.028, 0.22, 0.008, 2), black),
        // 提把前立柱
        (t(0.0, 0.082, 0.075), beveled_box(0.02, 0.05, 0.02, 0.004, 2), black),
        // 提把后立柱
        (t(0.0, 0.082, 0.225), beveled_box(0.02, 0.05, 0.02, 0.004, 2), black),
        // 照门
        (t(0.0, 0.10, 0.30), beveled_box(0.028, 0.04, 0.03, 0.005, 2), black),
        // 准星座
        (t(0.0, 0.035, 0.74), beveled_box(0.02, 0.025, 0.03, 0.004, 2), black),
        // 准星柱(垂直,无需rz())
        (t(0.0, 0.062, 0.74), cylinder(0.006, 0.03, 8), black),
        // 弹箱(机匣下扁盒)
        (t(0.0, -0.0625, 0.28), beveled_box(0.12, 0.075, 0.17, 0.015, 2), black_poly),
        // 握把(后倾)
        (t(0.0, -0.055, 0.055) * Mat4::from_rotation_x(0.35), beveled_box(0.045, 0.13, 0.05, 0.01, 2), black_poly),
        // 扳机护圈
        (t(0.0, -0.055, 0.13), beveled_box(0.035, 0.045, 0.10, 0.008, 2), black),
        // 固定托
        (t(0.0, 0.005, -0.12), beveled_box(0.06, 0.09, 0.24, 0.015, 2), olive),
        // 托底板
        (t(0.0, 0.005, -0.25), beveled_box(0.065, 0.10, 0.02, 0.005, 2), black),
        // 两脚架枢轴
        (t(0.0, -0.035, 0.47), beveled_box(0.04, 0.03, 0.06, 0.005, 2), steel_dark),
        // 两脚架左腿
        (t(0.0, -0.04, 0.44) * Mat4::from_rotation_z(0.45) * Mat4::from_rotation_x(0.6) * rz(), cylinder(0.008, 0.34, 10), steel_dark),
        // 两脚架右腿
        (t(0.0, -0.04, 0.44) * Mat4::from_rotation_z(-0.45) * Mat4::from_rotation_x(0.6) * rz(), cylinder(0.008, 0.34, 10), steel_dark),
        // 左脚掌
        (t(0.028, -0.099, 0.535) * Mat4::from_scale(glam::vec3(0.016, 0.016, 0.016)), sphere(8, 6), black),
        // 右脚掌
        (t(-0.028, -0.099, 0.535) * Mat4::from_scale(glam::vec3(0.016, 0.016, 0.016)), sphere(8, 6), black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "M249 SAW 蜂群", length: 1.10 }
}

pub fn m240l() -> crate::engine::guns::GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let steel_dark = [0.22f32, 0.24, 0.27];
    let steel = [0.45f32, 0.48, 0.52];
    let black_poly = [0.13f32, 0.14, 0.16];
    let black = [0.08f32, 0.08, 0.10];
    let parts = vec![
        // 细长机匣
        (t(0.0, 0.015, 0.16), beveled_box(0.075, 0.095, 0.36, 0.02, 2), steel_dark),
        // 上机匣盖
        (t(0.0, 0.07, 0.16), beveled_box(0.06, 0.018, 0.30, 0.005, 2), steel_dark),
        // 供弹机盖
        (t(0.0, 0.095, 0.14), beveled_box(0.07, 0.025, 0.12, 0.005, 2), steel_dark),
        // 细长枪管
        (t(0.0, 0.02, 0.56) * rz(), cylinder(0.017, 0.50, 20), steel),
        // 枪口消焰器
        (t(0.0, 0.02, 0.84) * rz(), frustum(0.021, 0.026, 0.06, 20, true), steel_dark),
        // 导气块
        (t(0.0, 0.02, 0.58), beveled_box(0.04, 0.04, 0.05, 0.005, 2), steel_dark),
        // 导气管
        (t(0.0, -0.01, 0.445) * rz(), cylinder(0.007, 0.27, 12), steel),
        // 准星座
        (t(0.0, 0.045, 0.75), beveled_box(0.02, 0.035, 0.03, 0.004, 2), black),
        // 准星柱(垂直,无需rz())
        (t(0.0, 0.07, 0.75), cylinder(0.005, 0.02, 8), black),
        // 照门
        (t(0.0, 0.095, 0.30), beveled_box(0.028, 0.035, 0.03, 0.005, 2), black),
        // 弹链盒
        (t(0.0, -0.06, 0.26), beveled_box(0.10, 0.07, 0.16, 0.012, 2), black_poly),
        // 弹链供弹槽
        (t(0.0, -0.02, 0.15), beveled_box(0.05, 0.03, 0.06, 0.005, 2), steel_dark),
        // 握把(后倾)
        (t(0.0, -0.05, 0.055) * Mat4::from_rotation_x(0.35), beveled_box(0.042, 0.12, 0.045, 0.008, 2), black_poly),
        // 扳机护圈
        (t(0.0, -0.058, 0.13), beveled_box(0.03, 0.04, 0.09, 0.007, 2), black),
        // 固定托
        (t(0.0, 0.01, -0.10), beveled_box(0.055, 0.085, 0.20, 0.012, 2), black_poly),
        // 缓冲管
        (t(0.0, 0.02, -0.25) * rz(), cylinder(0.017, 0.10, 12), steel_dark),
        // 托底板
        (t(0.0, 0.015, -0.31), beveled_box(0.062, 0.095, 0.022, 0.005, 2), black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "M240L 铁砧", length: 1.19 }
}
