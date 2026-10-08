// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

/// 世界坐标 → 网格坐标（±256 → 0..128）
pub(crate) fn world_to_grid(x: f32, z: f32) -> GridPos {
    let gx = ((x + GRID_HALF) / GRID_CELL).floor() as i32;
    let gz = ((z + GRID_HALF) / GRID_CELL).floor() as i32;
    GridPos::new(gx.clamp(0, GRID_SIZE as i32 - 1), gz.clamp(0, GRID_SIZE as i32 - 1))
}
/// 网格坐标 → 世界坐标（格中心）
pub(crate) fn grid_to_world(g: GridPos) -> (f32, f32) {
    let x = (g.x as f32 + 0.5) * GRID_CELL - GRID_HALF;
    let z = (g.y as f32 + 0.5) * GRID_CELL - GRID_HALF;
    (x, z)
}
/// 把一个障碍盒"够格"的格子标成阻挡，返回**新封的格数**。这是导航网格的**唯一建网规则**。
///
/// 判据 = 障碍 AABB 与该格（`GRID_CELL` 见方）的**重叠面积** ≥ [`CELL_BLOCK_MIN_OVERLAP_M2`]。
/// 逐格算 AABB∩格 的精确面积（不是"包围盒范围全封"），因为 4m 的格子对这个 1:1 的世界太粗：
/// 一件 6×0.9m 的中央隔离墩用"碰到就封"会封掉 3×2 = 6 格（96m²，实际占地 5.4m²），
/// 0.34m 的护柱封掉整整一格（16m²）—— 于是**几何上并不相连的装饰件在网格里连成一道墙**。
///
/// 实测后果（`reachable_mask` + 临时探针，数字见 `docs/PROGRESS.md` 2026-09-25 节）：
/// 程序化城市地图上玩家出生的十字路口被隔离墩/护柱/树/消防栓围成 **24 格的孤岛**
/// （全图可通行 13165 格），出生环 64 个采样点**一个都到不了玩家**；defense_line 的
/// 沙袋环同理（玩家所在连通域只剩 **4 格**）⇒ NPC 出生即在另一个连通域 ⇒ 永远走不到玩家 ⇒
/// `update_waves` 等不到 `npcs.is_empty()` ⇒ **波次永远清不掉**（未结案 #17 的真根因）。
///
/// 调用方：`apply_level`（生产）与单测的建网；**不许再写第二套循环**。
pub(crate) fn block_obstacle_cells(grid: &mut GridMap, ob: &MapObstacle) -> usize {
    let g0 = world_to_grid(ob.x - ob.half_w, ob.z - ob.half_d);
    let g1 = world_to_grid(ob.x + ob.half_w, ob.z + ob.half_d);
    let (hx, hz) = (GRID_CELL * 0.5, GRID_CELL * 0.5);
    // 长件（沙袋/矮墙/围墙/建筑）保守封格：NPC 必须绕着走，见 `CELL_BLOCK_LONG_EXTENT_M`
    let long = ob.half_w.max(ob.half_d) * 2.0 >= CELL_BLOCK_LONG_EXTENT_M;
    let mut newly_blocked = 0usize;
    for gx in g0.x..=g1.x {
        for gz in g0.y..=g1.y {
            let pos = GridPos::new(gx, gz);
            if !grid.in_bounds(pos) {
                continue;
            }
            let (cx, cz) = grid_to_world(pos);
            // AABB ∩ 格的逐轴重叠长度（≤0 表示该轴不重叠）；障碍比格宽时按格宽封顶
            let ox = (hx + ob.half_w - (ob.x - cx).abs()).min(GRID_CELL);
            let oz = (hz + ob.half_d - (ob.z - cz).abs()).min(GRID_CELL);
            if ox <= 0.0 || oz <= 0.0 {
                continue;
            }
            if !long && ox * oz < CELL_BLOCK_MIN_OVERLAP_M2 {
                continue;
            }
            if grid.is_passable(pos) {
                grid.block(pos);
                newly_blocked += 1;
            }
        }
    }
    newly_blocked
}
/// 障碍基础血量：按种类区分（墙 150 / 大块 300 / 路障 100）。
/// M1 步枪单发 25 伤害 → 6/12/4 发击穿；hp 归 0 即摧毁，从碰撞/阻挡/渲染中移除。
pub(crate) fn obstacle_max_hp(kind: ObstacleKind) -> f32 {
    match kind {
        ObstacleKind::Wall => 150.0,
        ObstacleKind::Block => 300.0,
        ObstacleKind::Barrier => 100.0,
        ObstacleKind::Tree => 60.0,
        ObstacleKind::Building => 500.0,
        ObstacleKind::Ruin => 120.0,
    }
}
/// 确定性 LCG（与 audio.rs 同款常数，零第三方依赖）：同一种子恒同布局，可测试
pub(crate) fn map_lcg_next(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *state
}
/// [0,1) 确定性随机数（取 LCG 高 24 位，避免低位周期过短）
pub(crate) fn map_lcg_unit(state: &mut u32) -> f32 {
    (map_lcg_next(state) >> 8) as f32 / (1u32 << 24) as f32
}
/// 把障碍盒径向推出安全环（若侵入 `ring_inner` 以内则沿原方向外推），并 clamp 到地图范围
pub(crate) fn push_out_of_safe_ring(ob: &mut MapObstacle, ring_inner: f32) {
    let d = (ob.x * ob.x + ob.z * ob.z).sqrt();
    if d < ring_inner && d > 1e-4 {
        let k = ring_inner / d;
        ob.x *= k;
        ob.z *= k;
    }
    ob.x = ob.x.clamp(-240.0, 240.0);
    ob.z = ob.z.clamp(-240.0, 240.0);
}
/// 关卡主题轮换：每 3 关一个周期。
///
/// 第 1 关 = 冒烟基准主题（58m 安全环 + 现状墙体布局），见 MAP_RING_INNER 注释；
/// 第 2/3 关依次切换大块/路障主题，安全环半径与密度同步变化（安全环都不低于 58m，
/// 保证任意关卡攻击态 NPC 站定与出生点弹道规则一致）。
pub(crate) fn theme_for_level(level: u32) -> MapTheme {
    match (level.saturating_sub(1)) % 3 {
        0 => MapTheme {
            ring_inner: MAP_RING_INNER,
            ring_outer: MAP_RING_OUTER,
            clusters_base: MAP_CLUSTERS,
            clusters_var: 5,
            kind: ObstacleKind::Wall,
        },
        1 => MapTheme {
            ring_inner: 64.0,
            ring_outer: 125.0,
            clusters_base: 8,
            clusters_var: 4,
            kind: ObstacleKind::Block,
        },
        _ => MapTheme {
            ring_inner: 60.0,
            ring_outer: 120.0,
            clusters_base: 9,
            clusters_var: 4,
            kind: ObstacleKind::Barrier,
        },
    }
}
/// 每种障碍的摆放风格（Wall = 第一关基准参数，勿改动）
pub(crate) fn kind_style(kind: ObstacleKind) -> KindStyle {
    match kind {
        ObstacleKind::Wall => KindStyle {
            min_boxes: 1,
            max_boxes: 3,
            min_w: 1.2,
            max_w: 3.2,
            min_d: 0.7,
            max_d: 1.7,
            gap: 0.6,
        },
        ObstacleKind::Block => KindStyle {
            min_boxes: 1,
            max_boxes: 2,
            min_w: 2.0,
            max_w: 4.0,
            min_d: 2.0,
            max_d: 4.0,
            gap: 1.0,
        },
        ObstacleKind::Barrier => KindStyle {
            min_boxes: 2,
            max_boxes: 4,
            min_w: 3.0,
            max_w: 5.0,
            min_d: 0.5,
            max_d: 0.8,
            gap: 0.5,
        },
        ObstacleKind::Tree => KindStyle {
            min_boxes: 1,
            max_boxes: 1,
            // 树干半宽收窄到 0.15..0.28（整宽 0.3..0.56m）：物理刚体与渲染 marker 同源
            // （同一 half_w/half_d），视觉/碰撞一致收窄 → 玩家可贴近树干（半径 0.5m 外即可绕行），
            // 不再有“看着细、撞着粗”的路障感。RNG 消耗顺序不变 → 摆放位置不变。
            min_w: 0.15,
            max_w: 0.28,
            min_d: 0.15,
            max_d: 0.28,
            gap: 1.5,
        },
        ObstacleKind::Building => KindStyle {
            min_boxes: 1,
            max_boxes: 1,
            min_w: 6.0,
            max_w: 9.0,
            min_d: 5.0,
            max_d: 7.0,
            gap: 2.0,
        },
        ObstacleKind::Ruin => KindStyle {
            min_boxes: 2,
            max_boxes: 4,
            min_w: 0.8,
            max_w: 2.0,
            min_d: 0.6,
            max_d: 1.5,
            gap: 0.8,
        },
    }
}
/// 程序化关卡地图：以 seed（= 关卡号）按主题生成障碍簇，分布在安全环带内。
///
/// 布局规则（注释即约定，勿随意改动）：
/// 1. 中央 theme.ring_inner 内刻意留空：攻击态 NPC 需原地站定（冒烟机制），
///    且玩家出生点/近距离战斗弹道不受阻挡（见 MAP_RING_INNER 注释）。
/// 2. 每簇沿切线方向并排 1..=3 个盒子（带间隙），形成可绕行的掩体墙。
/// 3. 盒子两两不重叠（最多重试 8 次，仍冲突则跳过），确定性可测。
pub(crate) fn generate_level_map(seed: u32) -> LevelMap {
    generate_level_map_with_theme(seed, theme_for_level(seed))
}
/// 以指定主题生成关卡布局：主题轮换测试 / 外部定制布局用。
///
/// Wall 主题的参数与 RNG 消耗顺序与旧版 generate_level_map 完全一致，
/// 保证第 1 关布局不因主题化而改变。
pub(crate) fn generate_level_map_with_theme(seed: u32, theme: MapTheme) -> LevelMap {
    let mut state = seed.wrapping_mul(0x9E37_79B9) ^ 0x5EED_1234;
    let clusters = theme.clusters_base + (state.wrapping_add(seed) % theme.clusters_var);
    let style = kind_style(theme.kind);
    let mut obstacles: Vec<MapObstacle> = Vec::new();
    let tau = std::f32::consts::TAU;
    for _ in 0..clusters {
        // 每簇 style 范围内个盒子，沿切线方向并排成墙/块
        let n_boxes = style.min_boxes as usize
            + (map_lcg_unit(&mut state) * (style.max_boxes - style.min_boxes + 1) as f32) as usize;
        let angle = map_lcg_unit(&mut state) * tau;
        let dir = (angle.cos(), angle.sin());
        let half_w = style.min_w + map_lcg_unit(&mut state) * (style.max_w - style.min_w);
        let half_d = style.min_d + map_lcg_unit(&mut state) * (style.max_d - style.min_d);
        let gap = style.gap;
        let span = n_boxes as f32 * (half_w * 2.0 + gap);
        // 簇中心到原点距离：内缘留出半墙余量，避免墙体侵入安全环
        let min_dist = theme.ring_inner + span * 0.5 + 1.0;
        let dist = min_dist + map_lcg_unit(&mut state) * (theme.ring_outer - min_dist).max(1.0);
        let cx = dir.0 * dist;
        let cz = dir.1 * dist;
        let tx = -dir.1; // 切线方向（垂直径向）
        let tz = dir.0;
        for i in 0..n_boxes {
            let off = (i as f32 - (n_boxes as f32 - 1.0) * 0.5) * (half_w * 2.0 + gap);
            let mut ob = MapObstacle::new(theme.kind, cx + tx * off, cz + tz * off, half_w, half_d);
            // 冲突检测 + 抖动重试：先径向推出安全环，再与已有障碍查重叠；
            // 重叠则小幅随机移位（最多 8 次），保证放置后同时满足安全环与非重叠约束
            let mut placed = false;
            for _ in 0..8 {
                push_out_of_safe_ring(&mut ob, theme.ring_inner);
                let mut overlap = false;
                for o in &obstacles {
                    if (ob.x - o.x).abs() < ob.half_w + o.half_w
                        && (ob.z - o.z).abs() < ob.half_d + o.half_d
                    {
                        overlap = true;
                        break;
                    }
                }
                if !overlap {
                    placed = true;
                    break;
                }
                ob.x += (map_lcg_unit(&mut state) - 0.5) * 6.0;
                ob.z += (map_lcg_unit(&mut state) - 0.5) * 6.0;
            }
            if placed {
                obstacles.push(ob);
            }
        }
    }

    // ---- 场景装饰（安全环外）：树木 / 建筑物 / 残骸，丰富战场外观 ----
    // 装饰放在 ring_outer 之外 20-60m 的环形带，避开战斗区（冒烟站定/弹道不受影响）；
    // 数量 10-16 个，确定性 LCG 派生；参与碰撞（可击穿，耐久见 obstacle_max_hp）。
    let deco_kinds = [ObstacleKind::Tree, ObstacleKind::Building, ObstacleKind::Ruin];
    let deco_count = 10 + (map_lcg_unit(&mut state) * 6.0) as usize;
    for _ in 0..deco_count {
        let kind = deco_kinds[((map_lcg_unit(&mut state) * 3.0) as usize) % 3];
        let style = kind_style(kind);
        let n_boxes = style.min_boxes as usize
            + (map_lcg_unit(&mut state) * (style.max_boxes - style.min_boxes + 1) as f32) as usize;
        let angle = map_lcg_unit(&mut state) * tau;
        let (dirx, dirz) = (angle.cos(), angle.sin());
        let half_w = style.min_w + map_lcg_unit(&mut state) * (style.max_w - style.min_w);
        let half_d = style.min_d + map_lcg_unit(&mut state) * (style.max_d - style.min_d);
        let dist = theme.ring_outer + 20.0 + map_lcg_unit(&mut state) * 40.0;
        let cx = dirx * dist;
        let cz = dirz * dist;
        let (tx, tz) = (-dirz, dirx);
        for i in 0..n_boxes {
            let off = (i as f32 - (n_boxes as f32 - 1.0) * 0.5) * (half_w * 2.0 + style.gap);
            let ob = MapObstacle::new(kind, cx + tx * off, cz + tz * off, half_w, half_d);
            let mut placed = true;
            for o in &obstacles {
                if (ob.x - o.x).abs() < ob.half_w + o.half_w
                    && (ob.z - o.z).abs() < ob.half_d + o.half_d
                {
                    placed = false;
                    break;
                }
            }
            if placed {
                obstacles.push(ob);
            }
        }
    }
    LevelMap { obstacles, decor: Vec::new(), props: Vec::new() }
}
