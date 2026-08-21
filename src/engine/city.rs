//! 手工绘制的现代城市战场（2026-08-21 起替代随机种子生成）。
//!
//! 架空 21 世纪战场：55m 街区网格的现代城区。
//! - 街区：中央广场（纪念碑）/ 写字楼群 / 仓库园区 / 公园树阵 / 商铺街 / 军事哨卡 / 停车场残骸
//! - 街道：双向主街 + 路灯 + 消防栓 + 中央隔离路障 + 人行道（地面纹理）
//! - 边界：混凝土围墙 + 4 座大门
//! - 障碍含半高/中心高/材质 tint：建筑是真正的高楼（不再是 2.4m 平板），
//!   树冠/消防栓/玻璃幕墙有专属颜色（消防栓红、树冠绿、玻璃蓝灰）。
//! 布局与 procedural::generate_city_ground_texture 共享本模块常量（街道/街区同源）。

use crate::engine::game::{LevelMap, MapObstacle, ObstacleKind};

/// 街道网格间距（米）：主街中心线位于 k*STREET_EVERY（k=-3..3）
pub const STREET_EVERY: f32 = 55.0;
/// 沥青半宽（米）
pub const ROAD_HALF: f32 = 5.0;
/// 人行道宽度（米，街沿到街区）
pub const SIDEWALK: f32 = 4.0;
/// 城市半宽（米；围墙位置 ±CITY_WALL）
pub const CITY_WALL: f32 = 215.0;

// ---- 常用材质色（线性 RGB）----
const CONCRETE: [f32; 3] = [0.58, 0.57, 0.55];
const CONCRETE_LIGHT: [f32; 3] = [0.64, 0.63, 0.60];
const GLASS_BLUE: [f32; 3] = [0.16, 0.22, 0.30];
const METAL_GRAY: [f32; 3] = [0.42, 0.45, 0.50];
const DARK: [f32; 3] = [0.22, 0.23, 0.26];
const TREE_LEAF: [f32; 3] = [0.22, 0.42, 0.14];
const TREE_BARK: [f32; 3] = [0.34, 0.24, 0.15];
const HYDRANT_RED: [f32; 3] = [0.68, 0.14, 0.12];
const SANDBAG: [f32; 3] = [0.60, 0.52, 0.34];
const CONTAINER_RUST: [f32; 3] = [0.50, 0.26, 0.14];
const CONTAINER_BLUE: [f32; 3] = [0.18, 0.32, 0.50];
const CONTAINER_GREEN: [f32; 3] = [0.22, 0.38, 0.22];
const WRECK_TAN: [f32; 3] = [0.34, 0.32, 0.30];
const WOOD_BENCH: [f32; 3] = [0.46, 0.34, 0.18];
const TENT_CAMO: [f32; 3] = [0.36, 0.40, 0.26];
const FLAG_POLE: [f32; 3] = [0.70, 0.71, 0.73];

/// 街区中心（索引 0..5 → -137.5..137.5，与街道网格对齐）
fn bc(i: usize) -> f32 {
    -137.5 + i as f32 * STREET_EVERY
}

/// 便捷构造：种类 + 位置 + 半尺寸 + (半高, 中心高, tint)
fn ob(
    o: &mut Vec<MapObstacle>,
    kind: ObstacleKind,
    x: f32,
    z: f32,
    hw: f32,
    hd: f32,
    hh: f32,
    y: f32,
    tint: Option<[f32; 3]>,
) {
    o.push(MapObstacle::new(kind, x, z, hw, hd).shaped(hh, y, tint));
}

/// 在 (cx,cz) 放置一座现代写字楼（裙楼 + 塔楼 + 屋顶机房 + 玻璃幕墙横带）
fn office_tower(o: &mut Vec<MapObstacle>, cx: f32, cz: f32) {
    // 裙楼（3 层高，覆盖街区大半）
    ob(o, ObstacleKind::Building, cx, cz, 15.0, 15.0, 3.0, 3.0, Some(CONCRETE_LIGHT));
    // 塔楼（12 层）
    ob(o, ObstacleKind::Building, cx, cz, 8.0, 8.0, 12.0, 15.0, Some(CONCRETE));
    // 屋顶机房
    ob(o, ObstacleKind::Building, cx, cz, 3.0, 3.0, 1.5, 28.5, Some(METAL_GRAY));
    // 玻璃幕墙横带（塔楼四面上中下三层）
    for fy in [7.0f32, 13.0, 19.0] {
        for (dx, dz, hw, hd) in [
            (0.0f32, 8.12f32, 8.1f32, 0.12f32),
            (0.0, -8.12, 8.1, 0.12),
            (8.12, 0.0, 0.12, 8.1),
            (-8.12, 0.0, 0.12, 8.1),
        ] {
            ob(
                o,
                ObstacleKind::Block,
                cx + dx,
                cz + dz,
                hw,
                hd,
                0.55,
                fy,
                Some(GLASS_BLUE),
            );
        }
    }
}

/// 仓库 + 天窗 + 集装箱堆场
fn warehouse(o: &mut Vec<MapObstacle>, cx: f32, cz: f32) {
    ob(o, ObstacleKind::Building, cx, cz, 17.0, 10.0, 5.0, 5.0, Some(CONCRETE_LIGHT));
    // 屋顶天窗（长条玻璃）
    ob(o, ObstacleKind::Block, cx - 5.0, cz, 2.0, 7.0, 0.25, 10.3, Some(GLASS_BLUE));
    ob(o, ObstacleKind::Block, cx + 5.0, cz, 2.0, 7.0, 0.25, 10.3, Some(GLASS_BLUE));
    // 集装箱堆（单层 + 双层一组）
    let colors = [CONTAINER_RUST, CONTAINER_BLUE, CONTAINER_GREEN, CONTAINER_RUST, CONTAINER_BLUE];
    for (i, c) in colors.iter().enumerate() {
        let (cx2, cz2) = match i {
            0 => (cx + 12.0, cz - 6.0),
            1 => (cx + 12.0, cz + 6.0),
            2 => (cx - 12.0, cz - 6.0),
            3 => (cx - 12.0, cz + 6.0),
            _ => (cx + 12.0, cz),
        };
        ob(o, ObstacleKind::Block, cx2, cz2, 3.05, 1.2, 1.3, 1.3, Some(*c));
        if i == 4 {
            // 叠放第二层
            ob(o, ObstacleKind::Block, cx2, cz2, 3.05, 1.2, 1.3, 3.9, Some(CONTAINER_GREEN));
        }
    }
}

/// 公园树阵（树干 + 树冠，各 3x3 棵）
fn park(o: &mut Vec<MapObstacle>, cx: f32, cz: f32) {
    for dx in [-13.0f32, 0.0, 13.0] {
        for dz in [-13.0f32, 0.0, 13.0] {
            let tx = cx + dx;
            let tz = cz + dz;
            // 树干
            ob(o, ObstacleKind::Tree, tx, tz, 0.22, 0.22, 1.6, 1.6, Some(TREE_BARK));
            // 树冠（2 层方块近似）
            ob(o, ObstacleKind::Tree, tx, tz, 1.8, 1.8, 1.1, 3.3, Some(TREE_LEAF));
            ob(o, ObstacleKind::Tree, tx, tz, 1.3, 1.3, 0.9, 4.5, Some(TREE_LEAF));
        }
    }
}

/// 商铺街（3 座 3 层沿街楼 + 店面玻璃带）
fn shops(o: &mut Vec<MapObstacle>, cx: f32, cz: f32) {
    for i in 0..3 {
        let bx = cx + (i as f32 - 1.0) * 14.0;
        let tint = if i == 1 { CONCRETE } else { CONCRETE_LIGHT };
        ob(o, ObstacleKind::Building, bx, cz, 6.2, 4.5, 6.0, 6.0, Some(tint));
        // 店面玻璃带（临街面）
        ob(o, ObstacleKind::Block, bx, cz + 4.62, 5.8, 0.12, 0.9, 1.5, Some(GLASS_BLUE));
        // 屋顶水箱
        ob(o, ObstacleKind::Block, bx, cz, 1.0, 1.0, 0.7, 12.5, Some(METAL_GRAY));
    }
}

/// 中央广场块（花坛 + 长椅；纪念碑只放一块）
fn plaza(o: &mut Vec<MapObstacle>, cx: f32, cz: f32, monument: bool) {
    // 花坛（4 角）
    for (dx, dz) in [(-16.0f32, -16.0f32), (16.0, -16.0), (-16.0, 16.0), (16.0, 16.0)] {
        ob(o, ObstacleKind::Building, cx + dx, cz + dz, 1.6, 1.6, 0.5, 0.5, Some(CONCRETE));
        ob(o, ObstacleKind::Tree, cx + dx, cz + dz, 1.2, 1.2, 0.4, 0.9, Some(TREE_LEAF));
    }
    // 长椅
    for (dx, dz) in [(-8.0f32, 0.0f32), (8.0, 0.0), (0.0, -8.0), (0.0, 8.0)] {
        ob(o, ObstacleKind::Block, cx + dx, cz + dz, 1.0, 0.4, 0.35, 0.35, Some(WOOD_BENCH));
    }
    if monument {
        // 纪念碑：基座 + 立柱 + 顶帽
        ob(o, ObstacleKind::Building, cx, cz, 2.6, 2.6, 0.5, 0.5, Some(CONCRETE_LIGHT));
        ob(o, ObstacleKind::Building, cx, cz, 0.9, 0.9, 3.6, 4.0, Some(FLAG_POLE));
        ob(o, ObstacleKind::Building, cx, cz, 1.1, 1.1, 0.4, 7.9, Some(FLAG_POLE));
        // 四根旗杆
        for (dx, dz) in [(-4.0f32, -4.0f32), (4.0, -4.0), (-4.0, 4.0), (4.0, 4.0)] {
            ob(o, ObstacleKind::Block, cx + dx, cz + dz, 0.06, 0.06, 2.5, 2.5, Some(FLAG_POLE));
        }
    }
}

/// 军事哨卡（沙袋墙 + 混凝土路障 + 帐篷 + 车辆残骸）
fn staging(o: &mut Vec<MapObstacle>, cx: f32, cz: f32) {
    ob(o, ObstacleKind::Barrier, cx, cz - 8.0, 5.0, 0.55, 0.55, 0.55, Some(SANDBAG));
    ob(o, ObstacleKind::Barrier, cx, cz + 8.0, 5.0, 0.55, 0.55, 0.55, Some(SANDBAG));
    ob(o, ObstacleKind::Building, cx - 10.0, cz, 3.0, 0.4, 0.5, 0.5, Some(CONCRETE));
    ob(o, ObstacleKind::Building, cx + 10.0, cz, 3.0, 0.4, 0.5, 0.5, Some(CONCRETE));
    ob(o, ObstacleKind::Ruin, cx + 6.0, cz - 5.0, 2.2, 1.1, 0.8, 0.8, Some(WRECK_TAN));
    ob(o, ObstacleKind::Building, cx - 6.0, cz + 5.0, 2.0, 2.0, 1.5, 1.5, Some(TENT_CAMO));
}

/// 停车场残骸（4 辆废车 + 2 路障）
fn parking(o: &mut Vec<MapObstacle>, cx: f32, cz: f32) {
    let tints = [WRECK_TAN, [0.30, 0.32, 0.36], [0.36, 0.30, 0.24], [0.28, 0.30, 0.28]];
    for (i, t) in tints.iter().enumerate() {
        let (dx, dz) = match i {
            0 => (-10.0, -8.0),
            1 => (-3.0, -8.0),
            2 => (4.0, 4.0),
            _ => (11.0, 4.0),
        };
        ob(o, ObstacleKind::Ruin, cx + dx, cz + dz, 2.2, 1.1, 0.8, 0.8, Some(*t));
    }
    ob(o, ObstacleKind::Building, cx, cz + 10.0, 3.0, 0.4, 0.5, 0.5, Some(CONCRETE));
    ob(o, ObstacleKind::Building, cx, cz - 10.0, 3.0, 0.4, 0.5, 0.5, Some(CONCRETE));
}

/// 街区角色表（索引 0..5 × 0..5）：P=广场 O=写字楼 W=仓库 G=公园 S=商铺 M=哨卡 C=停车场
fn block_role(i: usize, j: usize) -> char {
    match (i, j) {
        (2, 2) | (2, 3) | (3, 2) | (3, 3) => 'P',
        (1, 1) | (1, 4) | (4, 1) | (4, 4) => 'O',
        (0, 0) | (0, 5) | (5, 0) | (5, 5) => 'W',
        (2, 1) | (3, 1) | (2, 4) | (3, 4) => 'G',
        (1, 2) | (1, 3) | (4, 2) | (4, 3) => 'S',
        (0, 2) | (0, 3) | (5, 2) | (5, 3) | (2, 0) | (3, 0) | (2, 5) | (3, 5) => 'M',
        _ => 'C',
    }
}

/// 街道设施（路灯/消防栓/中央隔离路障）
fn street_furniture(o: &mut Vec<MapObstacle>) {
    // 主干道（x=0 / z=0）路灯：两侧沿路每 55m 一盏（避开路口）
    for k in [-3i32, -2, -1, 1, 2, 3] {
        let t = k as f32 * STREET_EVERY;
        ob(o, ObstacleKind::Block, 7.0, t, 0.06, 0.06, 3.4, 3.4, Some(DARK));
        ob(o, ObstacleKind::Block, -7.0, t, 0.06, 0.06, 3.4, 3.4, Some(DARK));
        ob(o, ObstacleKind::Block, t, 7.0, 0.06, 0.06, 3.4, 3.4, Some(DARK));
        ob(o, ObstacleKind::Block, t, -7.0, 0.06, 0.06, 3.4, 3.4, Some(DARK));
    }
    // 消防栓：内圈 5x5 路口各一个（人行道角）
    for k in -2i32..=2 {
        for m in -2i32..=2 {
            if k == 0 && m == 0 {
                continue; // 原点让给玩家出生点
            }
            let x = k as f32 * STREET_EVERY + 8.0;
            let z = m as f32 * STREET_EVERY + 8.0;
            ob(o, ObstacleKind::Block, x, z, 0.13, 0.13, 0.3, 0.3, Some(HYDRANT_RED));
        }
    }
    // 中央隔离路障：大道入口（混凝土矮墙）
    for (x, z) in [
        (14.0f32, 0.0f32),
        (-14.0, 0.0),
        (0.0, 14.0),
        (0.0, -14.0),
        (69.0, 0.0),
        (-69.0, 0.0),
        (0.0, 69.0),
        (0.0, -69.0),
    ] {
        let (hw, hd) = if x.abs() > 0.1 { (3.0, 0.4) } else { (0.4, 3.0) };
        ob(o, ObstacleKind::Building, x, z, hw, hd, 0.5, 0.5, Some(CONCRETE));
    }
}

/// 边界围墙（混凝土，4 边，每边 7 段 + 4 座大门缺口）
fn perimeter(o: &mut Vec<MapObstacle>) {
    let w = CITY_WALL;
    for side in 0..4 {
        for k in -3i32..=3 {
            let t = k as f32 * STREET_EVERY; // 段中心 = ±165, ±110, ±55, 0
            let gap = t.abs() < 3.0; // 大门在 0（±110 处留侧门）
            let hw = 26.0;
            if !gap {
                match side {
                    0 => ob(o, ObstacleKind::Building, t, -w, hw, 0.6, 1.1, 1.1, Some(CONCRETE)),
                    1 => ob(o, ObstacleKind::Building, t, w, hw, 0.6, 1.1, 1.1, Some(CONCRETE)),
                    2 => ob(o, ObstacleKind::Building, -w, t, 0.6, hw, 1.1, 1.1, Some(CONCRETE)),
                    _ => ob(o, ObstacleKind::Building, w, t, 0.6, hw, 1.1, 1.1, Some(CONCRETE)),
                }
            }
        }
    }
}

/// 手工城市地图（默认关卡；完整布局见模块头注释）
pub fn generate_city() -> LevelMap {
    let mut o: Vec<MapObstacle> = Vec::new();
    perimeter(&mut o);
    for i in 0..6 {
        for j in 0..6 {
            let cx = bc(i);
            let cz = bc(j);
            match block_role(i, j) {
                'P' => plaza(&mut o, cx, cz, i == 2 && j == 2),
                'O' => office_tower(&mut o, cx, cz),
                'W' => warehouse(&mut o, cx, cz),
                'G' => park(&mut o, cx, cz),
                'S' => shops(&mut o, cx, cz),
                'M' => staging(&mut o, cx, cz),
                _ => parking(&mut o, cx, cz),
            }
        }
    }
    street_furniture(&mut o);
    LevelMap { obstacles: o }
}

/// 地面分区（供程序化地面纹理；与布局严格同源）：
/// 0=草地 1=沙土 2=沥青路 3=人行道 4=广场铺装 5=建筑地基
pub fn ground_zone(x: f32, z: f32) -> u8 {
    // 街道：|x mod 55| 或 |z mod 55| 在道路带内
    let fx = (x / STREET_EVERY).round();
    let fz = (z / STREET_EVERY).round();
    let dx = (x - fx * STREET_EVERY).abs();
    let dz = (z - fz * STREET_EVERY).abs();
    let on_x_street = dx <= ROAD_HALF;
    let on_z_street = dz <= ROAD_HALF;
    if on_x_street || on_z_street {
        return 2; // 沥青
    }
    let in_x_side = dx <= ROAD_HALF + SIDEWALK;
    let in_z_side = dz <= ROAD_HALF + SIDEWALK;
    if in_x_side || in_z_side {
        return 3; // 人行道
    }
    // 中央广场区（4 块）：|x|<50 && |z|<50（街道已排除）
    if x.abs() < 50.0 && z.abs() < 50.0 {
        return 4; // 广场铺装
    }
    // 街区：写字楼/仓库/商铺块 → 建筑地基；公园/哨卡/停车场 → 草地
    let i = ((x + 137.5) / STREET_EVERY).floor() as i32;
    let j = ((z + 137.5) / STREET_EVERY).floor() as i32;
    if (0..6).contains(&i) && (0..6).contains(&j) {
        match block_role(i as usize, j as usize) {
            'O' | 'W' | 'S' => return 5, // 建筑地基
            _ => return 0,               // 草地
        }
    }
    // 城郊：草地 + 沙土分域
    let n = (x * 0.001 + z * 0.0023).sin() * (x * 0.0017 - z * 0.001).cos();
    if n > 0.3 {
        1
    } else {
        0
    }
}
