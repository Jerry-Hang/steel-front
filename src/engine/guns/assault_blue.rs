use crate::engine::meshgen::{beveled_box, cylinder};
use crate::engine::guns::{assemble, GunMesh, rz};
use glam::Mat4;

pub fn hk416() -> crate::engine::guns::GunMesh {
    let parts = vec![
        // 上机匣（圆角盒）
        (Mat4::from_translation(glam::vec3(0.0, 0.03, 0.0)), beveled_box(0.07, 0.085, 0.24, 0.012, 8), [0.22, 0.24, 0.27]),
        // 下机匣（黑聚合物）
        (Mat4::from_translation(glam::vec3(0.0, -0.045, 0.0)), beveled_box(0.06, 0.08, 0.20, 0.010, 8), [0.13, 0.14, 0.16]),
        // 枪管
        (Mat4::from_translation(glam::vec3(0.0, 0.035, 0.225)) * rz(), cylinder(0.012, 0.21, 20), [0.45, 0.48, 0.52]),
        // 方形护木
        (Mat4::from_translation(glam::vec3(0.0, 0.035, 0.19)), beveled_box(0.065, 0.065, 0.22, 0.008, 6), [0.13, 0.14, 0.16]),
        // 护木散热凹槽（左右各3）
        (Mat4::from_translation(glam::vec3(0.0335, 0.035, 0.115)), beveled_box(0.005, 0.02, 0.032, 0.002, 4), [0.08, 0.08, 0.10]),
        (Mat4::from_translation(glam::vec3(-0.0335, 0.035, 0.115)), beveled_box(0.005, 0.02, 0.032, 0.002, 4), [0.08, 0.08, 0.10]),
        (Mat4::from_translation(glam::vec3(0.0335, 0.035, 0.19)), beveled_box(0.005, 0.02, 0.032, 0.002, 4), [0.08, 0.08, 0.10]),
        (Mat4::from_translation(glam::vec3(-0.0335, 0.035, 0.19)), beveled_box(0.005, 0.02, 0.032, 0.002, 4), [0.08, 0.08, 0.10]),
        (Mat4::from_translation(glam::vec3(0.0335, 0.035, 0.265)), beveled_box(0.005, 0.02, 0.032, 0.002, 4), [0.08, 0.08, 0.10]),
        (Mat4::from_translation(glam::vec3(-0.0335, 0.035, 0.265)), beveled_box(0.005, 0.02, 0.032, 0.002, 4), [0.08, 0.08, 0.10]),
        // 顶部导轨（贯穿机匣与护木）
        (Mat4::from_translation(glam::vec3(0.0, 0.078, 0.09)), beveled_box(0.045, 0.010, 0.44, 0.004, 4), [0.08, 0.08, 0.10]),
        // 直弹匣（微弯，上段直、下段后倾）
        (Mat4::from_translation(glam::vec3(0.0, -0.135, -0.005)), beveled_box(0.036, 0.10, 0.045, 0.006, 4), [0.08, 0.08, 0.10]),
        (Mat4::from_translation(glam::vec3(0.0, -0.24, -0.02)) * Mat4::from_rotation_x(-0.13), beveled_box(0.036, 0.11, 0.045, 0.006, 4), [0.08, 0.08, 0.10]),
        // 缓冲管
        (Mat4::from_translation(glam::vec3(0.0, 0.02, -0.23)) * rz(), cylinder(0.015, 0.22, 16), [0.22, 0.24, 0.27]),
        // 缓冲管螺帽
        (Mat4::from_translation(glam::vec3(0.0, 0.02, -0.12)) * rz(), cylinder(0.017, 0.02, 12), [0.45, 0.48, 0.52]),
        // 伸缩托
        (Mat4::from_translation(glam::vec3(0.0, -0.005, -0.36)), beveled_box(0.055, 0.11, 0.10, 0.012, 6), [0.25, 0.32, 0.20]),
        // 贴腮板
        (Mat4::from_translation(glam::vec3(0.0, 0.048, -0.365)), beveled_box(0.045, 0.028, 0.08, 0.006, 4), [0.13, 0.14, 0.16]),
        // 托底板
        (Mat4::from_translation(glam::vec3(0.0, -0.005, -0.417)), beveled_box(0.055, 0.11, 0.014, 0.006, 4), [0.08, 0.08, 0.10]),
        // A2握把（斜后下）
        (Mat4::from_translation(glam::vec3(0.0, -0.115, 0.07)) * Mat4::from_rotation_x(-0.45), beveled_box(0.04, 0.13, 0.045, 0.008, 5), [0.25, 0.32, 0.20]),
        // 扳机护圈
        (Mat4::from_translation(glam::vec3(0.0, -0.108, 0.02)), beveled_box(0.042, 0.008, 0.07, 0.003, 4), [0.13, 0.14, 0.16]),
        // 扳机
        (Mat4::from_translation(glam::vec3(0.0, -0.096, 0.02)), beveled_box(0.01, 0.025, 0.012, 0.002, 4), [0.08, 0.08, 0.10]),
        // 拉机柄
        (Mat4::from_translation(glam::vec3(0.0, 0.068, -0.155)), beveled_box(0.018, 0.014, 0.07, 0.003, 4), [0.08, 0.08, 0.10]),
        // 抛壳窗盖
        (Mat4::from_translation(glam::vec3(0.035, 0.04, -0.01)), beveled_box(0.004, 0.02, 0.05, 0.001, 3), [0.08, 0.08, 0.10]),
        // 准星座
        (Mat4::from_translation(glam::vec3(0.0, 0.09, 0.27)), beveled_box(0.022, 0.014, 0.02, 0.002, 4), [0.08, 0.08, 0.10]),
        // 准星柱
        (Mat4::from_translation(glam::vec3(0.0, 0.108, 0.27)) * rz(), cylinder(0.0035, 0.024, 8), [0.08, 0.08, 0.10]),
        // 折叠表尺
        (Mat4::from_translation(glam::vec3(0.0, 0.095, -0.05)), beveled_box(0.03, 0.028, 0.02, 0.003, 4), [0.08, 0.08, 0.10]),
        // 红点镜身
        (Mat4::from_translation(glam::vec3(0.0, 0.105, 0.02)), beveled_box(0.032, 0.034, 0.07, 0.008, 5), [0.08, 0.08, 0.10]),
        // 红点镜片
        (Mat4::from_translation(glam::vec3(0.0, 0.105, 0.055)) * rz(), cylinder(0.011, 0.006, 12), [0.22, 0.24, 0.27]),
        // 红点电池仓
        (Mat4::from_translation(glam::vec3(0.0, 0.124, 0.02)), beveled_box(0.036, 0.014, 0.04, 0.004, 4), [0.08, 0.08, 0.10]),
        // 鸟笼消焰器
        (Mat4::from_translation(glam::vec3(0.0, 0.035, 0.355)) * rz(), cylinder(0.016, 0.05, 16), [0.45, 0.48, 0.52]),
        // 消焰器收口环
        (Mat4::from_translation(glam::vec3(0.0, 0.035, 0.367)) * rz(), cylinder(0.018, 0.012, 16), [0.60, 0.63, 0.67]),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "HK416 A8 游隼", length: 0.80 }
}

pub fn mk18() -> crate::engine::guns::GunMesh {
    let parts = vec![
        // 上机匣（圆角盒）
        (Mat4::from_translation(glam::vec3(0.0, 0.03, 0.0)), beveled_box(0.07, 0.085, 0.18, 0.012, 8), [0.22, 0.24, 0.27]),
        // 下机匣（黑聚合物）
        (Mat4::from_translation(glam::vec3(0.0, -0.045, 0.0)), beveled_box(0.06, 0.08, 0.16, 0.010, 8), [0.13, 0.14, 0.16]),
        // 10.3寸枪管
        (Mat4::from_translation(glam::vec3(0.0, 0.035, 0.205)) * rz(), cylinder(0.011, 0.23, 20), [0.45, 0.48, 0.52]),
        // 短护木（方形断面）
        (Mat4::from_translation(glam::vec3(0.0, 0.035, 0.15)), beveled_box(0.062, 0.062, 0.14, 0.008, 6), [0.13, 0.14, 0.16]),
        // 护木散热凹槽（左右各2）
        (Mat4::from_translation(glam::vec3(0.032, 0.035, 0.12)), beveled_box(0.005, 0.018, 0.03, 0.002, 4), [0.08, 0.08, 0.10]),
        (Mat4::from_translation(glam::vec3(-0.032, 0.035, 0.12)), beveled_box(0.005, 0.018, 0.03, 0.002, 4), [0.08, 0.08, 0.10]),
        (Mat4::from_translation(glam::vec3(0.032, 0.035, 0.18)), beveled_box(0.005, 0.018, 0.03, 0.002, 4), [0.08, 0.08, 0.10]),
        (Mat4::from_translation(glam::vec3(-0.032, 0.035, 0.18)), beveled_box(0.005, 0.018, 0.03, 0.002, 4), [0.08, 0.08, 0.10]),
        // 顶部导轨（贯穿机匣与护木）
        (Mat4::from_translation(glam::vec3(0.0, 0.078, 0.065)), beveled_box(0.045, 0.010, 0.34, 0.004, 4), [0.08, 0.08, 0.10]),
        // 直弹匣（微弯，上段直、下段后倾）
        (Mat4::from_translation(glam::vec3(0.0, -0.135, -0.005)), beveled_box(0.036, 0.10, 0.045, 0.006, 4), [0.08, 0.08, 0.10]),
        (Mat4::from_translation(glam::vec3(0.0, -0.24, -0.02)) * Mat4::from_rotation_x(-0.13), beveled_box(0.036, 0.11, 0.045, 0.006, 4), [0.08, 0.08, 0.10]),
        // 缓冲管
        (Mat4::from_translation(glam::vec3(0.0, 0.02, -0.17)) * rz(), cylinder(0.015, 0.16, 16), [0.22, 0.24, 0.27]),
        // 缓冲管螺帽
        (Mat4::from_translation(glam::vec3(0.0, 0.02, -0.10)) * rz(), cylinder(0.017, 0.018, 12), [0.45, 0.48, 0.52]),
        // 伸缩托
        (Mat4::from_translation(glam::vec3(0.0, -0.005, -0.29)), beveled_box(0.055, 0.11, 0.08, 0.012, 6), [0.25, 0.32, 0.20]),
        // 贴腮板
        (Mat4::from_translation(glam::vec3(0.0, 0.048, -0.2925)), beveled_box(0.045, 0.028, 0.065, 0.006, 4), [0.13, 0.14, 0.16]),
        // 托底板
        (Mat4::from_translation(glam::vec3(0.0, -0.005, -0.336)), beveled_box(0.055, 0.11, 0.012, 0.006, 4), [0.08, 0.08, 0.10]),
        // A2握把（斜后下）
        (Mat4::from_translation(glam::vec3(0.0, -0.115, 0.065)) * Mat4::from_rotation_x(-0.45), beveled_box(0.04, 0.13, 0.045, 0.008, 5), [0.25, 0.32, 0.20]),
        // 扳机护圈
        (Mat4::from_translation(glam::vec3(0.0, -0.108, 0.015)), beveled_box(0.042, 0.008, 0.07, 0.003, 4), [0.13, 0.14, 0.16]),
        // 扳机
        (Mat4::from_translation(glam::vec3(0.0, -0.096, 0.018)), beveled_box(0.01, 0.025, 0.012, 0.002, 4), [0.08, 0.08, 0.10]),
        // 拉机柄
        (Mat4::from_translation(glam::vec3(0.0, 0.068, -0.135)), beveled_box(0.018, 0.014, 0.06, 0.003, 4), [0.08, 0.08, 0.10]),
        // 抛壳窗盖
        (Mat4::from_translation(glam::vec3(0.035, 0.04, -0.01)), beveled_box(0.004, 0.02, 0.05, 0.001, 3), [0.08, 0.08, 0.10]),
        // 准星座
        (Mat4::from_translation(glam::vec3(0.0, 0.09, 0.20)), beveled_box(0.022, 0.014, 0.02, 0.002, 4), [0.08, 0.08, 0.10]),
        // 准星柱
        (Mat4::from_translation(glam::vec3(0.0, 0.108, 0.20)) * rz(), cylinder(0.0035, 0.024, 8), [0.08, 0.08, 0.10]),
        // 折叠表尺
        (Mat4::from_translation(glam::vec3(0.0, 0.095, -0.06)), beveled_box(0.03, 0.028, 0.02, 0.003, 4), [0.08, 0.08, 0.10]),
        // 消音器
        (Mat4::from_translation(glam::vec3(0.0, 0.035, 0.39)) * rz(), cylinder(0.022, 0.12, 24), [0.30, 0.31, 0.34]),
        // 消音器前环
        (Mat4::from_translation(glam::vec3(0.0, 0.035, 0.445)) * rz(), cylinder(0.024, 0.012, 24), [0.45, 0.48, 0.52]),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "MK18 隼爪", length: 0.79 }
}
