use crate::engine::meshgen::{beveled_box, cylinder, frustum, sphere, torus_arc};
use crate::engine::guns::{assemble, GunMesh, rz};
use glam::Mat4;

pub fn osv96() -> GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::Vec3::new(x, y, z));
    let rzm = |x: f32, y: f32, z: f32| t(x, y, z) * rz();
    let sc = |x: f32, y: f32, z: f32, k: f32| t(x, y, z) * Mat4::from_scale(glam::Vec3::splat(k));

    let steel_dark = [0.22, 0.24, 0.27];
    let steel = [0.45, 0.48, 0.52];
    let steel_light = [0.6, 0.63, 0.67];
    let poly_black = [0.13, 0.14, 0.16];
    let black = [0.08, 0.08, 0.10];
    let olive = [0.25, 0.32, 0.20];
    let brake_grey = [0.30, 0.31, 0.34];

    let parts = vec![
        // 长粗枪管
        (rzm(0.0, 0.0, 0.27), cylinder(0.028, 0.78, 24), steel_dark),
        // 枪管根部气室
        (rzm(0.0, 0.0, 0.02), cylinder(0.035, 0.10, 20), steel),
        // 大型制退器主体（前扩锥台）
        (rzm(0.0, 0.0, 0.695), frustum(0.044, 0.052, 0.07, 20, true), brake_grey),
        // 制退器后箍
        (rzm(0.0, 0.0, 0.672), cylinder(0.045, 0.012, 20), steel),
        // 制退器排气凹槽（模拟孔）
        (rzm(0.0, 0.0, 0.69), cylinder(0.043, 0.02, 20), black),
        // 制退器中环
        (t(0.0, 0.0, 0.71), torus_arc(0.047, 0.006, 0.0, std::f32::consts::TAU, 20, 6), steel_light),
        // 制退器前环
        (t(0.0, 0.0, 0.725), torus_arc(0.052, 0.009, 0.0, std::f32::consts::TAU, 24, 8), steel_light),
        // 护木
        (t(0.0, 0.0, 0.30), beveled_box(0.075, 0.085, 0.40, 0.008, 2), olive),
        // 护木箍 x2
        (t(0.0, 0.0, 0.18), torus_arc(0.043, 0.006, 0.0, std::f32::consts::TAU, 20, 6), steel),
        (t(0.0, 0.0, 0.44), torus_arc(0.043, 0.006, 0.0, std::f32::consts::TAU, 20, 6), steel),
        // 粗壮机匣
        (t(0.0, 0.0, -0.12), beveled_box(0.085, 0.105, 0.42, 0.012, 2), steel_dark),
        // 顶部导轨
        (t(0.0, 0.063, -0.10), beveled_box(0.03, 0.02, 0.34, 0.004, 2), black),
        // 拉机柄球头
        (sc(0.062, 0.04, -0.05, 0.02), sphere(10, 8), steel_light),
        // 握把（弹匣前方）
        (t(0.0, -0.055, -0.22) * Mat4::from_rotation_x(-0.30), beveled_box(0.045, 0.16, 0.07, 0.01, 2), poly_black),
        // 扳机护圈
        (t(0.0, -0.05, -0.155) * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2), torus_arc(0.042, 0.007, 0.0, std::f32::consts::TAU, 20, 6), black),
        // 弹匣（无托：在握把后方）
        (t(0.0, -0.06, -0.44), beveled_box(0.06, 0.15, 0.19, 0.012, 2), steel_dark),
        // 弹匣底板
        (t(0.0, -0.145, -0.44), beveled_box(0.062, 0.02, 0.19, 0.006, 2), black),
        // 枪托主体
        (t(0.0, 0.005, -0.62), beveled_box(0.075, 0.105, 0.20, 0.012, 2), olive),
        // 贴腮板
        (t(0.0, 0.052, -0.60), beveled_box(0.05, 0.035, 0.14, 0.008, 2), black),
        // 托底板
        (t(0.0, 0.005, -0.72), beveled_box(0.085, 0.12, 0.03, 0.008, 2), black),
        // 两脚架座
        (t(0.0, -0.052, 0.28), beveled_box(0.06, 0.02, 0.10, 0.006, 2), steel),
        // 两脚架腿 x2
        (t(-0.032, -0.11, 0.30) * Mat4::from_rotation_z(-0.14) * Mat4::from_rotation_x(0.55) * rz(), cylinder(0.006, 0.22, 8), steel_dark),
        (t(0.032, -0.11, 0.30) * Mat4::from_rotation_z(0.14) * Mat4::from_rotation_x(0.55) * rz(), cylinder(0.006, 0.22, 8), steel_dark),
        // 准星座
        (rzm(0.0, 0.035, 0.545), cylinder(0.005, 0.06, 8), black),
        // 照门
        (t(0.0, 0.078, -0.25), beveled_box(0.016, 0.012, 0.03, 0.003, 1), black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "OSV-96 削岩", length: 1.47 }
}

pub fn m82a1() -> GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::Vec3::new(x, y, z));
    let rzm = |x: f32, y: f32, z: f32| t(x, y, z) * rz();

    let steel_dark = [0.22, 0.24, 0.27];
    let steel = [0.45, 0.48, 0.52];
    let steel_light = [0.6, 0.63, 0.67];
    let poly_black = [0.13, 0.14, 0.16];
    let black = [0.08, 0.08, 0.10];
    let brake_grey = [0.30, 0.31, 0.34];

    let parts = vec![
        // 大型制退器主体
        (rzm(0.0, 0.0, 0.7075), frustum(0.040, 0.052, 0.065, 20, true), brake_grey),
        // 制退器后箍
        (rzm(0.0, 0.0, 0.678), cylinder(0.043, 0.012, 20), steel),
        // 制退器排气凹槽（模拟孔）
        (rzm(0.0, 0.0, 0.705), cylinder(0.043, 0.022, 20), black),
        // 制退器中环
        (t(0.0, 0.0, 0.72), torus_arc(0.047, 0.007, 0.0, std::f32::consts::TAU, 20, 6), steel_light),
        // 制退器前环
        (t(0.0, 0.0, 0.735), torus_arc(0.052, 0.009, 0.0, std::f32::consts::TAU, 24, 8), steel_light),
        // 枪管
        (rzm(0.0, 0.0, 0.30), cylinder(0.025, 0.75, 24), steel_dark),
        // 枪管节套
        (rzm(0.0, 0.0, -0.02), cylinder(0.034, 0.08, 20), steel),
        // 方形护木
        (t(0.0, 0.0, 0.34), beveled_box(0.075, 0.085, 0.40, 0.008, 2), poly_black),
        // 护木前帽
        (rzm(0.0, 0.0, 0.56), cylinder(0.041, 0.015, 20), steel),
        // 护木箍
        (t(0.0, 0.0, 0.26), torus_arc(0.042, 0.006, 0.0, std::f32::consts::TAU, 20, 6), steel),
        // 粗壮机匣
        (t(0.0, 0.0, -0.18), beveled_box(0.085, 0.10, 0.44, 0.012, 2), steel_dark),
        // 顶部导轨
        (t(0.0, 0.056, -0.16), beveled_box(0.028, 0.018, 0.32, 0.004, 2), black),
        // 提把
        (t(0.0, 0.09, -0.10), beveled_box(0.022, 0.045, 0.14, 0.006, 2), poly_black),
        // 照门座
        (t(0.0, 0.065, -0.30), beveled_box(0.02, 0.05, 0.03, 0.004, 1), black),
        // 握把
        (t(0.0, -0.05, -0.20) * Mat4::from_rotation_x(-0.30), beveled_box(0.042, 0.15, 0.065, 0.01, 2), poly_black),
        // 扳机护圈
        (t(0.0, -0.045, -0.155) * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2), torus_arc(0.04, 0.007, 0.0, std::f32::consts::TAU, 20, 6), black),
        // 10发弹匣
        (t(0.0, -0.06, -0.02), beveled_box(0.055, 0.13, 0.16, 0.01, 2), steel_dark),
        // 弹匣底板
        (t(0.0, -0.135, -0.02), beveled_box(0.057, 0.02, 0.16, 0.006, 2), black),
        // 缓冲管（托）
        (rzm(0.0, 0.0, -0.56), cylinder(0.028, 0.30, 20), steel_dark),
        // 缓冲管卡箍
        (rzm(0.0, 0.0, -0.42), cylinder(0.035, 0.04, 20), steel),
        // 贴腮板
        (t(0.0, 0.045, -0.60), beveled_box(0.05, 0.03, 0.12, 0.008, 2), black),
        // 托底板
        (t(0.0, -0.005, -0.72), beveled_box(0.08, 0.115, 0.05, 0.01, 2), poly_black),
        // 准星
        (t(0.0, 0.06, 0.545), beveled_box(0.012, 0.035, 0.012, 0.002, 1), black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "M82A1 巴雷特", length: 1.49 }
}
