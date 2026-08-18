use crate::engine::meshgen::{beveled_box, cylinder, sphere};
use crate::engine::guns::{assemble, GunMesh, rz};
use glam::Mat4;

pub fn sv98() -> crate::engine::guns::GunMesh {
    const STEEL_DARK: [f32; 3] = [0.22, 0.24, 0.27];
    const STEEL: [f32; 3] = [0.45, 0.48, 0.52];
    const STEEL_LIGHT: [f32; 3] = [0.60, 0.63, 0.67];
    const BLACK: [f32; 3] = [0.08, 0.08, 0.10];
    const WOOD: [f32; 3] = [0.45, 0.30, 0.16];
    const SUPPRESSOR_GRAY: [f32; 3] = [0.30, 0.31, 0.34];

    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::Vec3::new(x, y, z));
    let tz = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::Vec3::new(x, y, z)) * rz();
    let ts = |x: f32, y: f32, z: f32, s: f32| {
        Mat4::from_translation(glam::Vec3::new(x, y, z)) * Mat4::from_scale(glam::Vec3::splat(s))
    };

    let parts = vec![
        // 枪托底板
        (t(0.0, -0.005, 0.008), beveled_box(0.047, 0.056, 0.016, 0.006, 4), BLACK),
        // 木枪托（短托，无托腮）
        (t(0.0, -0.005, 0.225), beveled_box(0.045, 0.050, 0.42, 0.014, 4), WOOD),
        // 木前托
        (t(0.0, -0.022, 0.465), beveled_box(0.040, 0.040, 0.06, 0.010, 4), WOOD),
        // 机匣
        (t(0.0, 0.0, 0.575), beveled_box(0.042, 0.055, 0.24, 0.008, 4), STEEL_DARK),
        // 粗枪管
        (tz(0.0, 0.0, 0.945), cylinder(0.017, 0.50, 24), STEEL),
        // 枪口制退器
        (tz(0.0, 0.0, 1.23), cylinder(0.024, 0.07, 24), SUPPRESSOR_GRAY),
        // 内藏弹仓
        (t(0.0, -0.042, 0.64), beveled_box(0.034, 0.045, 0.08, 0.005, 4), STEEL_DARK),
        // 扳机护圈
        (t(0.0, -0.028, 0.54), beveled_box(0.026, 0.024, 0.08, 0.005, 3), BLACK),
        // 瞄准镜筒
        (tz(0.0, 0.10, 0.60), cylinder(0.022, 0.18, 24), BLACK),
        // 瞄准镜前镜环
        (tz(0.0, 0.10, 0.705), cylinder(0.028, 0.03, 24), STEEL_LIGHT),
        // 瞄准镜后镜环
        (tz(0.0, 0.10, 0.495), cylinder(0.028, 0.03, 24), STEEL_LIGHT),
        // 瞄准镜前镜片
        (tz(0.0, 0.10, 0.706), cylinder(0.016, 0.012, 20), BLACK),
        // 瞄准镜后镜片
        (tz(0.0, 0.10, 0.504), cylinder(0.014, 0.012, 20), BLACK),
        // 瞄准镜支架
        (t(0.0, 0.055, 0.53), beveled_box(0.020, 0.048, 0.020, 0.004, 3), STEEL_DARK),
        (t(0.0, 0.055, 0.67), beveled_box(0.020, 0.048, 0.020, 0.004, 3), STEEL_DARK),
        // 拉机柄
        (t(0.060, 0.030, 0.66), beveled_box(0.085, 0.008, 0.008, 0.003, 3), STEEL),
        // 拉机柄球
        (ts(0.105, 0.040, 0.66, 0.012), sphere(10, 8), STEEL),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "SV-98M 针叶", length: 1.27 }
}

pub fn m2010() -> crate::engine::guns::GunMesh {
    const STEEL_DARK: [f32; 3] = [0.22, 0.24, 0.27];
    const STEEL: [f32; 3] = [0.45, 0.48, 0.52];
    const STEEL_LIGHT: [f32; 3] = [0.60, 0.63, 0.67];
    const POLY: [f32; 3] = [0.13, 0.14, 0.16];
    const BLACK: [f32; 3] = [0.08, 0.08, 0.10];
    const SUPPRESSOR_GRAY: [f32; 3] = [0.30, 0.31, 0.34];

    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::Vec3::new(x, y, z));
    let tz = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::Vec3::new(x, y, z)) * rz();

    let parts = vec![
        // 折叠聚合物枪托
        (t(0.0, -0.006, 0.20), beveled_box(0.042, 0.050, 0.40, 0.012, 4), POLY),
        // 托底板
        (t(0.0, -0.006, 0.008), beveled_box(0.044, 0.056, 0.016, 0.006, 4), BLACK),
        // 折叠铰链
        (t(0.0, 0.0, 0.425), beveled_box(0.046, 0.058, 0.05, 0.008, 4), STEEL),
        // 机匣
        (t(0.0, 0.0, 0.58), beveled_box(0.042, 0.058, 0.26, 0.008, 4), STEEL_DARK),
        // 聚合物护木
        (t(0.0, -0.020, 0.83), beveled_box(0.038, 0.040, 0.26, 0.010, 4), POLY),
        // 粗枪管
        (tz(0.0, 0.0, 0.96), cylinder(0.017, 0.50, 24), STEEL),
        // 枪口制退器
        (tz(0.0, 0.0, 1.245), cylinder(0.025, 0.07, 24), SUPPRESSOR_GRAY),
        // 弹匣
        (t(0.0, -0.052, 0.625), beveled_box(0.036, 0.075, 0.11, 0.006, 4), STEEL_DARK),
        // 扳机护圈
        (t(0.0, -0.030, 0.52), beveled_box(0.026, 0.026, 0.09, 0.005, 3), BLACK),
        // 瞄准镜筒
        (tz(0.0, 0.11, 0.60), cylinder(0.022, 0.20, 24), BLACK),
        // 瞄准镜前镜环
        (tz(0.0, 0.11, 0.705), cylinder(0.028, 0.03, 24), STEEL_LIGHT),
        // 瞄准镜后镜环
        (tz(0.0, 0.11, 0.495), cylinder(0.028, 0.03, 24), STEEL_LIGHT),
        // 瞄准镜前镜片
        (tz(0.0, 0.11, 0.706), cylinder(0.016, 0.012, 20), BLACK),
        // 瞄准镜后镜片
        (tz(0.0, 0.11, 0.504), cylinder(0.014, 0.012, 20), BLACK),
        // 瞄准镜支架
        (t(0.0, 0.06, 0.52), beveled_box(0.020, 0.052, 0.020, 0.004, 3), STEEL_DARK),
        (t(0.0, 0.06, 0.68), beveled_box(0.020, 0.052, 0.020, 0.004, 3), STEEL_DARK),
        // 拉机柄
        (t(0.060, 0.032, 0.68), beveled_box(0.085, 0.008, 0.008, 0.003, 3), STEEL),
        // 拉机柄球（垂直圆柱）
        (t(0.105, 0.046, 0.68), cylinder(0.009, 0.03, 12), STEEL),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "M2010 ESR 界标", length: 1.28 }
}

pub fn mrad() -> crate::engine::guns::GunMesh {
    const STEEL_DARK: [f32; 3] = [0.22, 0.24, 0.27];
    const STEEL: [f32; 3] = [0.45, 0.48, 0.52];
    const STEEL_LIGHT: [f32; 3] = [0.60, 0.63, 0.67];
    const POLY: [f32; 3] = [0.13, 0.14, 0.16];
    const BLACK: [f32; 3] = [0.08, 0.08, 0.10];
    const DESERT: [f32; 3] = [0.45, 0.36, 0.24];
    const SUPPRESSOR_GRAY: [f32; 3] = [0.30, 0.31, 0.34];

    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::Vec3::new(x, y, z));
    let tz = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::Vec3::new(x, y, z)) * rz();

    let parts = vec![
        // 模块化枪托底盘
        (t(0.0, -0.005, 0.20), beveled_box(0.044, 0.048, 0.40, 0.012, 4), POLY),
        // 托底板
        (t(0.0, -0.005, 0.008), beveled_box(0.046, 0.054, 0.016, 0.006, 4), BLACK),
        // 可调托腮底座
        (t(0.0, 0.032, 0.16), beveled_box(0.036, 0.016, 0.16, 0.004, 3), BLACK),
        // 可调托腮衬垫
        (t(0.0, 0.050, 0.17), beveled_box(0.038, 0.014, 0.12, 0.006, 3), DESERT),
        // 托腮调节杆
        (t(0.0, 0.026, 0.235), beveled_box(0.008, 0.030, 0.008, 0.003, 3), STEEL_LIGHT),
        // 机匣连接块
        (t(0.0, 0.0, 0.425), beveled_box(0.046, 0.056, 0.05, 0.008, 4), STEEL),
        // 机匣
        (t(0.0, 0.0, 0.58), beveled_box(0.042, 0.060, 0.26, 0.008, 4), STEEL_DARK),
        // 沙漠色护木
        (t(0.0, -0.020, 0.83), beveled_box(0.038, 0.040, 0.26, 0.010, 4), DESERT),
        // 粗枪管
        (tz(0.0, 0.0, 0.96), cylinder(0.017, 0.50, 24), STEEL),
        // 枪口制退器
        (tz(0.0, 0.0, 1.245), cylinder(0.025, 0.07, 24), SUPPRESSOR_GRAY),
        // 弹匣
        (t(0.0, -0.052, 0.625), beveled_box(0.036, 0.070, 0.11, 0.006, 4), STEEL_DARK),
        // 扳机护圈
        (t(0.0, -0.030, 0.52), beveled_box(0.026, 0.026, 0.09, 0.005, 3), BLACK),
        // 瞄准镜筒
        (tz(0.0, 0.11, 0.60), cylinder(0.022, 0.20, 24), BLACK),
        // 瞄准镜前镜环
        (tz(0.0, 0.11, 0.705), cylinder(0.028, 0.03, 24), STEEL_LIGHT),
        // 瞄准镜后镜环
        (tz(0.0, 0.11, 0.495), cylinder(0.028, 0.03, 24), STEEL_LIGHT),
        // 瞄准镜前镜片
        (tz(0.0, 0.11, 0.706), cylinder(0.016, 0.012, 20), BLACK),
        // 瞄准镜后镜片
        (tz(0.0, 0.11, 0.504), cylinder(0.014, 0.012, 20), BLACK),
        // 瞄准镜支架
        (t(0.0, 0.06, 0.52), beveled_box(0.020, 0.052, 0.020, 0.004, 3), STEEL_DARK),
        (t(0.0, 0.06, 0.68), beveled_box(0.020, 0.052, 0.020, 0.004, 3), STEEL_DARK),
        // 拉机柄
        (t(0.060, 0.032, 0.68), beveled_box(0.085, 0.008, 0.008, 0.003, 3), STEEL),
        // 拉机柄球（垂直圆柱）
        (t(0.105, 0.046, 0.68), cylinder(0.009, 0.03, 12), STEEL),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "MRAD 巨石", length: 1.28 }
}
