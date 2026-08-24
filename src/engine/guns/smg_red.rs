use crate::engine::meshgen::{beveled_box, cylinder, frustum, torus_arc};
use crate::engine::guns::{assemble, rz, t, rx};
use std::f32::consts::PI;

const STEEL_L: [f32; 3] = [0.62, 0.65, 0.70];
const STEEL_D: [f32; 3] = [0.30, 0.33, 0.37];
const POLY_BLACK: [f32; 3] = [0.16, 0.17, 0.19];
const DEEP_BLACK: [f32; 3] = [0.10, 0.10, 0.12];
const WOOD: [f32; 3] = [0.50, 0.34, 0.17];
const OLIVE: [f32; 3] = [0.28, 0.35, 0.22];
const SAND: [f32; 3] = [0.48, 0.39, 0.26];
const TAU: f32 = 2.0 * PI;

pub fn pp19() -> crate::engine::guns::GunMesh {
    let parts = vec![
        // 机匣
        (t(0.0, 0.01, 0.0), beveled_box(0.055, 0.06, 0.22, 0.010, 4), POLY_BLACK),
        // 机匣盖
        (t(0.0, 0.055, 0.0), beveled_box(0.042, 0.014, 0.19, 0.006, 4), DEEP_BLACK),
        // 螺旋弹筒
        (t(0.0, -0.055, 0.065) * rz(), cylinder(0.035, 0.20, 12), STEEL_D),
        // 弹筒前盖
        (t(0.0, -0.055, 0.166) * rz(), cylinder(0.037, 0.012, 12), DEEP_BLACK),
        // 螺旋肋条
        (t(0.0, -0.055, 0.01) * rx(0.3), torus_arc(0.035, 0.0045, 0.0, TAU, 8, 6), DEEP_BLACK),
        (t(0.0, -0.055, 0.07) * rx(0.6), torus_arc(0.035, 0.0045, 0.0, TAU, 8, 6), DEEP_BLACK),
        (t(0.0, -0.055, 0.13) * rx(0.9), torus_arc(0.035, 0.0045, 0.0, TAU, 8, 6), DEEP_BLACK),
        // 短枪管
        (t(0.0, 0.01, 0.185) * rz(), cylinder(0.016, 0.14, 16), STEEL_L),
        // 消焰器
        (t(0.0, 0.01, 0.285) * rz(), frustum(0.02, 0.024, 0.06, 16, true), DEEP_BLACK),
        // 护木
        (t(0.0, 0.01, 0.15), beveled_box(0.05, 0.048, 0.12, 0.010, 4), POLY_BLACK),
        // 准星座
        (t(0.0, 0.043, 0.235), beveled_box(0.024, 0.032, 0.02, 0.006, 4), DEEP_BLACK),
        // 准星
        (t(0.0, 0.07, 0.235), beveled_box(0.006, 0.022, 0.006, 0.002, 2), STEEL_L),
        // 表尺
        (t(0.0, 0.065, -0.02), beveled_box(0.03, 0.02, 0.03, 0.005, 4), DEEP_BLACK),
        // 拉机柄
        (t(0.038, 0.025, 0.03), beveled_box(0.024, 0.02, 0.014, 0.004, 4), DEEP_BLACK),
        // 握把
        (t(0.0, -0.062, -0.11) * rx(0.3), beveled_box(0.032, 0.075, 0.05, 0.010, 4), POLY_BLACK),
        // 扳机护圈
        (t(0.0, -0.025, -0.05), torus_arc(0.03, 0.005, PI, TAU, 8, 6), DEEP_BLACK),
        // 扳机
        (t(0.0, -0.045, -0.045), beveled_box(0.008, 0.024, 0.006, 0.002, 2), STEEL_D),
        // 折叠托臂 L/R
        (t(-0.026, 0.02, -0.24), beveled_box(0.016, 0.035, 0.19, 0.006, 4), STEEL_D),
        (t(0.026, 0.02, -0.24), beveled_box(0.016, 0.035, 0.19, 0.006, 4), STEEL_D),
        // 肩托
        (t(0.0, 0.03, -0.35), beveled_box(0.06, 0.06, 0.03, 0.010, 4), POLY_BLACK),
        // 托铰链
        (t(0.0, 0.02, -0.14), beveled_box(0.036, 0.022, 0.022, 0.006, 4), STEEL_D),
        // 快慢机
        (t(0.032, 0.0, 0.02), beveled_box(0.014, 0.03, 0.008, 0.003, 2), DEEP_BLACK),
    ];
    let (verts, indices) = assemble(&parts);
    crate::engine::guns::GunMesh { verts, indices, display_name: "PP-19-01 勇士", length: 0.68 }
}

pub fn pp9() -> crate::engine::guns::GunMesh {
    let parts = vec![
        // 细长机匣
        (t(0.0, 0.01, -0.05), beveled_box(0.055, 0.06, 0.22, 0.010, 4), POLY_BLACK),
        // 一体消音器
        (t(0.0, 0.0, 0.24) * rz(), cylinder(0.026, 0.34, 12), DEEP_BLACK),
        // 消音器前盖
        (t(0.0, 0.0, 0.405) * rz(), cylinder(0.027, 0.02, 12), DEEP_BLACK),
        // 钢箍肋条
        (t(0.0, 0.0, 0.16), torus_arc(0.026, 0.004, 0.0, TAU, 8, 6), STEEL_L),
        (t(0.0, 0.0, 0.27), torus_arc(0.026, 0.004, 0.0, TAU, 8, 6), STEEL_L),
        (t(0.0, 0.0, 0.37), torus_arc(0.026, 0.004, 0.0, TAU, 8, 6), STEEL_L),
        // 连接环
        (t(0.0, 0.0, 0.08) * rz(), cylinder(0.028, 0.018, 12), STEEL_D),
        // 顶部导轨
        (t(0.0, 0.046, -0.03), beveled_box(0.03, 0.012, 0.16, 0.004, 4), DEEP_BLACK),
        // 护木
        (t(0.0, 0.0, 0.005), beveled_box(0.048, 0.05, 0.12, 0.010, 4), POLY_BLACK),
        // 准星座
        (t(0.0, 0.045, 0.33), beveled_box(0.02, 0.03, 0.02, 0.005, 4), DEEP_BLACK),
        // 准星
        (t(0.0, 0.075, 0.33), beveled_box(0.005, 0.02, 0.005, 0.002, 2), STEEL_L),
        // 表尺
        (t(0.0, 0.058, -0.05), beveled_box(0.028, 0.02, 0.02, 0.005, 4), DEEP_BLACK),
        // 拉机柄
        (t(0.036, 0.03, -0.05), beveled_box(0.022, 0.02, 0.014, 0.004, 4), DEEP_BLACK),
        // 握把
        (t(0.0, -0.06, -0.11) * rx(0.25), beveled_box(0.032, 0.075, 0.05, 0.010, 4), POLY_BLACK),
        // 扳机护圈
        (t(0.0, -0.022, -0.075), torus_arc(0.028, 0.005, PI, TAU, 8, 6), DEEP_BLACK),
        // 扳机
        (t(0.0, -0.04, -0.07), beveled_box(0.008, 0.022, 0.006, 0.002, 2), STEEL_D),
        // 弹匣
        (t(0.0, -0.09, -0.03) * rx(0.18), beveled_box(0.035, 0.11, 0.05, 0.008, 4), STEEL_D),
        // 弹匣底板
        (t(0.0, -0.145, -0.015), beveled_box(0.038, 0.015, 0.052, 0.005, 4), DEEP_BLACK),
        // 折叠托臂 L/R
        (t(-0.025, 0.02, -0.27), beveled_box(0.014, 0.032, 0.17, 0.006, 4), STEEL_D),
        (t(0.025, 0.02, -0.27), beveled_box(0.014, 0.032, 0.17, 0.006, 4), STEEL_D),
        // 肩托
        (t(0.0, 0.028, -0.365), beveled_box(0.055, 0.058, 0.028, 0.010, 4), SAND),
        // 托铰链
        (t(0.0, 0.02, -0.175), beveled_box(0.034, 0.02, 0.02, 0.005, 4), STEEL_D),
    ];
    let (verts, indices) = assemble(&parts);
    crate::engine::guns::GunMesh { verts, indices, display_name: "PP-9 胡蜂", length: 0.79 }
}

pub fn vss() -> crate::engine::guns::GunMesh {
    let parts = vec![
        // 机匣
        (t(0.0, 0.01, -0.05), beveled_box(0.055, 0.06, 0.22, 0.010, 4), OLIVE),
        // 一体消音器
        (t(0.0, 0.0, 0.24) * rz(), cylinder(0.024, 0.32, 12), DEEP_BLACK),
        // 消音器前盖
        (t(0.0, 0.0, 0.395) * rz(), cylinder(0.025, 0.022, 12), DEEP_BLACK),
        // 消音器肋条
        (t(0.0, 0.0, 0.14), torus_arc(0.024, 0.0035, 0.0, TAU, 8, 6), DEEP_BLACK),
        (t(0.0, 0.0, 0.24), torus_arc(0.024, 0.0035, 0.0, TAU, 8, 6), DEEP_BLACK),
        (t(0.0, 0.0, 0.34), torus_arc(0.024, 0.0035, 0.0, TAU, 8, 6), DEEP_BLACK),
        // 连接环
        (t(0.0, 0.0, 0.075) * rz(), cylinder(0.027, 0.018, 12), STEEL_D),
        // 护木
        (t(0.0, 0.0, 0.03), beveled_box(0.05, 0.05, 0.10, 0.010, 4), OLIVE),
        // 准星座
        (t(0.0, 0.04, 0.32), beveled_box(0.02, 0.028, 0.018, 0.005, 4), DEEP_BLACK),
        // 准星
        (t(0.0, 0.07, 0.32), beveled_box(0.005, 0.02, 0.005, 0.002, 2), STEEL_L),
        // 瞄准镜镜身
        (t(0.0, 0.085, -0.05) * rz(), cylinder(0.018, 0.15, 12), DEEP_BLACK),
        // 物镜
        (t(0.0, 0.085, 0.04) * rz(), frustum(0.018, 0.026, 0.03, 16, true), DEEP_BLACK),
        // 目镜
        (t(0.0, 0.085, -0.13) * rz(), frustum(0.018, 0.023, 0.025, 16, true), DEEP_BLACK),
        // 镜座
        (t(0.0, 0.07, -0.03), beveled_box(0.03, 0.028, 0.06, 0.006, 4), STEEL_D),
        // 拉机柄
        (t(0.036, 0.03, -0.04), beveled_box(0.02, 0.022, 0.02, 0.005, 4), DEEP_BLACK),
        // 木枪托
        (t(0.0, 0.02, -0.30), beveled_box(0.055, 0.09, 0.24, 0.012, 4), WOOD),
        // 枪托底板
        (t(0.0, 0.02, -0.44), beveled_box(0.058, 0.095, 0.02, 0.006, 4), DEEP_BLACK),
        // 木握把
        (t(0.0, -0.055, -0.15) * rx(0.3), beveled_box(0.032, 0.08, 0.05, 0.010, 4), WOOD),
        // 扳机护圈
        (t(0.0, -0.02, -0.10), torus_arc(0.028, 0.005, PI, TAU, 8, 6), DEEP_BLACK),
        // 扳机
        (t(0.0, -0.038, -0.095), beveled_box(0.008, 0.02, 0.006, 0.002, 2), STEEL_D),
        // 10发弹匣
        (t(0.0, -0.075, -0.045) * rx(0.2), beveled_box(0.035, 0.09, 0.05, 0.008, 4), STEEL_D),
        // 弹匣底板
        (t(0.0, -0.115, -0.035), beveled_box(0.038, 0.014, 0.052, 0.005, 4), DEEP_BLACK),
    ];
    let (verts, indices) = assemble(&parts);
    crate::engine::guns::GunMesh { verts, indices, display_name: "VSS Vintorez", length: 0.86 }
}

pub fn asval() -> crate::engine::guns::GunMesh {
    let parts = vec![
        // 机匣
        (t(0.0, 0.01, -0.05), beveled_box(0.055, 0.06, 0.22, 0.010, 4), OLIVE),
        // 机匣顶盖
        (t(0.0, 0.056, -0.03), beveled_box(0.042, 0.016, 0.17, 0.006, 4), POLY_BLACK),
        // 一体消音器
        (t(0.0, 0.0, 0.23) * rz(), cylinder(0.024, 0.30, 12), DEEP_BLACK),
        // 消音器前盖
        (t(0.0, 0.0, 0.375) * rz(), cylinder(0.025, 0.022, 12), DEEP_BLACK),
        // 钢箍肋条
        (t(0.0, 0.0, 0.15), torus_arc(0.024, 0.0035, 0.0, TAU, 8, 6), STEEL_L),
        (t(0.0, 0.0, 0.28), torus_arc(0.024, 0.0035, 0.0, TAU, 8, 6), STEEL_L),
        // 连接环
        (t(0.0, 0.0, 0.075) * rz(), cylinder(0.027, 0.018, 12), STEEL_D),
        // 护木（包裹消音器根部）
        (t(0.0, 0.0, 0.10), beveled_box(0.05, 0.05, 0.12, 0.010, 4), POLY_BLACK),
        // 准星座
        (t(0.0, 0.04, 0.31), beveled_box(0.02, 0.028, 0.018, 0.005, 4), DEEP_BLACK),
        // 准星
        (t(0.0, 0.07, 0.31), beveled_box(0.005, 0.02, 0.005, 0.002, 2), STEEL_L),
        // 红点镜身
        (t(0.0, 0.09, -0.04) * rz(), cylinder(0.014, 0.06, 16), DEEP_BLACK),
        // 红点物镜
        (t(0.0, 0.09, 0.0) * rz(), frustum(0.014, 0.019, 0.015, 16, true), DEEP_BLACK),
        // 镜座
        (t(0.0, 0.078, -0.04), beveled_box(0.026, 0.02, 0.04, 0.005, 4), STEEL_D),
        // 拉机柄
        (t(0.036, 0.03, -0.02), beveled_box(0.02, 0.022, 0.02, 0.005, 4), DEEP_BLACK),
        // 折叠托臂 L/R
        (t(-0.026, 0.02, -0.27), beveled_box(0.014, 0.034, 0.18, 0.006, 4), POLY_BLACK),
        (t(0.026, 0.02, -0.27), beveled_box(0.014, 0.034, 0.18, 0.006, 4), POLY_BLACK),
        // 肩托
        (t(0.0, 0.028, -0.375), beveled_box(0.055, 0.06, 0.03, 0.010, 4), POLY_BLACK),
        // 托铰链
        (t(0.0, 0.02, -0.175), beveled_box(0.034, 0.02, 0.02, 0.005, 4), STEEL_D),
        // 握把
        (t(0.0, -0.058, -0.13) * rx(0.28), beveled_box(0.032, 0.08, 0.05, 0.010, 4), POLY_BLACK),
        // 扳机护圈
        (t(0.0, -0.02, -0.09), torus_arc(0.028, 0.005, PI, TAU, 8, 6), DEEP_BLACK),
        // 扳机
        (t(0.0, -0.038, -0.085), beveled_box(0.008, 0.02, 0.006, 0.002, 2), STEEL_D),
        // 20发弹匣
        (t(0.0, -0.085, -0.045) * rx(0.18), beveled_box(0.036, 0.11, 0.05, 0.008, 4), STEEL_D),
    ];
    let (verts, indices) = assemble(&parts);
    crate::engine::guns::GunMesh { verts, indices, display_name: "AS Val", length: 0.78 }
}
