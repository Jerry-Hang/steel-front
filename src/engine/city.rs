//! 手绘现代城市地图（默认关卡）—— 2026-09-01 几何重构版。
//!
//! ## 这次重构解决什么
//! 旧版整座 5×5 街区只产出 **43 个障碍盒**：一个"街区"就是**一个盒子**，而且
//! 36 个街区里有 20 个是几乎空的开阔地。原因不是没人想细化，而是当时的 marker
//! 只能画盒子、形状还要靠 tint 颜色猜（见 `engine::geom`），且全城被塞在
//! `MAX_MARKER_INSTANCES=1024` 的预算里，细化一次就要多付一份物理。
//! 结果就是用户描述的"跟纸片一样"：没有进深、没有凹窗、没有檐口、没有体量转折。
//!
//! ## 现在的做法
//! - **形状成为数据**：`engine::geom::Shape`（立方/圆柱/二十面体/球）。灯杆、树干、
//!   消防栓、通风管、穹顶终于可以是圆的。
//! - **容量 1024 → 4096**，够一栋楼拆成十几个部件。
//! - **结构件与装饰件分表**（`LevelMap::obstacles` / `LevelMap::decor`）：挑檐、窗带、
//!   壁柱、屋顶设备只进渲染，不进刚体表、不进导航网格、不可摧毁。画面拿到全部细节，
//!   物理只付结构体的钱，也顺手消灭了"屋顶空调机在街面留下一圈隐形墙"。
//! - **所有几何用"外尺寸 + 底/顶标高"描述**（见 [`Part`]），不再手写半尺寸和中心高。
//!   旧代码 `shaped(12.0, 15.0, ...)`（半高 12、中心 15）让塔楼碰撞盒从 y=3 才开始，
//!   **楼底下 3 米子弹直接穿过去**。用标高描述从源头避免这类算错。
//! - **零共面**（这是"透视/Z-fighting 穿帮"的正解，不靠 depth bias 压）：
//!   1. 任何贴在墙上的件，背面必须**埋进墙里**，不得与墙面共面（见 `INSET_BURIED`）；
//!   2. 任何两层表面之间至少留 `RELIEF_STEP`（14cm）台阶，掠射角下不会落到同一批像素；
//!   3. 任何落地件底面留 `UNDER_GROUND`（5cm）在地下，不与地形平面共面。
//!   三条都有对应单元测试兜着（`city_layout_tests`）。

use crate::engine::game::{LevelMap, MapObstacle, ObstacleKind};
use crate::engine::geom::Shape;
use crate::engine::props::{PropPlacement, PropSet};

/// 街道网格间距（米）：主街中心线位于 k*STREET_EVERY（k=-3..3）
pub const STREET_EVERY: f32 = 55.0;
/// 沥青半宽（米）
pub const ROAD_HALF: f32 = 5.0;
/// 人行道宽度（米，街沿到街区）
pub const SIDEWALK: f32 = 4.0;
/// 城市半宽（米；围墙位置 ±CITY_WALL）
pub const CITY_WALL: f32 = 215.0;
/// 层高（米）——立面窗带、层线、女儿墙全部按它对齐
pub const FLOOR_H: f32 = 3.15;
/// 街区腹地边长（米）：STREET_EVERY 减去两侧道路与人行道
pub const BLOCK_EDGE: f32 = STREET_EVERY - 2.0 * (ROAD_HALF + SIDEWALK);
/// 街廓（含人行道）边长
pub const KERB_EDGE: f32 = BLOCK_EDGE + 2.0 * SIDEWALK;

/// 相邻两层表面之间的最小台阶（米）。小于这个值，掠射角下两面会落到同一批像素上。
const RELIEF_STEP: f32 = 0.14;
/// 贴墙件的背面埋入墙内的深度（米）——保证背面永远在实体里，不与墙面共面。
const INSET_BURIED: f32 = 0.20;
/// 落地件埋入地下的深度（米）——不与地形平面共面。
const UNDER_GROUND: f32 = -0.05;
/// 任何几何件的最小轴长（米）。低于它就是在制造纸片。
const MIN_AXIS: f32 = 0.20;

// ---- 常用材质色（线性 RGB）----
// 刻意引入砖红/奶油/赭石等强色相立面：旧版城市 90% 是同一支灰色混凝土，
// 实测整帧平均饱和度只有 0.05（正常户外场景 0.15+），"假"有一半是因为它几乎无色。
const CONCRETE: [f32; 3] = [0.56, 0.55, 0.53];
const CONCRETE_LIGHT: [f32; 3] = [0.66, 0.64, 0.60];
const CONCRETE_DARK: [f32; 3] = [0.40, 0.39, 0.38];
const BRICK_RED: [f32; 3] = [0.46, 0.22, 0.16];
const PLASTER_CREAM: [f32; 3] = [0.70, 0.63, 0.48];
const STUCCO_OCHRE: [f32; 3] = [0.58, 0.44, 0.27];
const GRANITE: [f32; 3] = [0.34, 0.33, 0.35];
const GLASS_BLUE: [f32; 3] = [0.13, 0.19, 0.27];
const GLASS_DARK: [f32; 3] = [0.07, 0.09, 0.13];
const METAL_GRAY: [f32; 3] = [0.42, 0.45, 0.50];
const METAL_RUST: [f32; 3] = [0.40, 0.24, 0.14];
const ROOF_MEMBRANE: [f32; 3] = [0.26, 0.27, 0.25];
const DARK: [f32; 3] = [0.20, 0.21, 0.24];
const TREE_LEAF: [f32; 3] = [0.20, 0.40, 0.13];
const TREE_LEAF_2: [f32; 3] = [0.26, 0.44, 0.16];
const TREE_LEAF_3: [f32; 3] = [0.15, 0.32, 0.12];
const TREE_BARK: [f32; 3] = [0.30, 0.21, 0.13];
const HYDRANT_RED: [f32; 3] = [0.62, 0.12, 0.10];
const SANDBAG: [f32; 3] = [0.58, 0.50, 0.33];
const CONTAINER_RUST: [f32; 3] = [0.50, 0.26, 0.14];
const CONTAINER_BLUE: [f32; 3] = [0.18, 0.32, 0.50];
const CONTAINER_GREEN: [f32; 3] = [0.22, 0.38, 0.22];
const WRECK_TAN: [f32; 3] = [0.34, 0.32, 0.30];
const WRECK_GLASS: [f32; 3] = [0.28, 0.26, 0.24];
const WOOD_BENCH: [f32; 3] = [0.46, 0.34, 0.18];
const TENT_CAMO: [f32; 3] = [0.36, 0.40, 0.26];
const FLAG_POLE: [f32; 3] = [0.70, 0.71, 0.73];
const CURB_STONE: [f32; 3] = [0.52, 0.51, 0.48];
const LAMP_GLOW: [f32; 3] = [0.85, 0.82, 0.70];

/// 一栋楼的立面配色（同栋楼内部一致，街区间切换）。
#[derive(Copy, Clone)]
struct Palette {
    wall: [f32; 3],
    trim: [f32; 3],
    glass: [f32; 3],
    base: [f32; 3],
}

const PALETTES: [Palette; 5] = [
    Palette { wall: CONCRETE_LIGHT, trim: CONCRETE, glass: GLASS_BLUE, base: GRANITE },
    Palette { wall: BRICK_RED, trim: CONCRETE_LIGHT, glass: GLASS_DARK, base: GRANITE },
    Palette { wall: PLASTER_CREAM, trim: CONCRETE, glass: GLASS_BLUE, base: CONCRETE_DARK },
    Palette { wall: STUCCO_OCHRE, trim: PLASTER_CREAM, glass: GLASS_DARK, base: GRANITE },
    Palette { wall: CONCRETE, trim: METAL_GRAY, glass: GLASS_BLUE, base: CONCRETE_DARK },
];

fn palette_at(k: i32) -> Palette {
    PALETTES[k.rem_euclid(PALETTES.len() as i32) as usize]
}

/// 确定性哈希 → [0,1)。同一 (i,j) 恒同值，保证地图可复现、可测试。
fn hash01(i: i32, j: i32) -> f32 {
    let mut x = (i as u32).wrapping_mul(0x27d4_eb2d) ^ (j as u32).wrapping_mul(0x1656_67b1);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297a_2d39);
    x ^= x >> 15;
    (x & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// 在 [lo, hi] 之间的确定性插值
fn mixf(i: i32, j: i32, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * hash01(i, j)
}

/// 落地件的底面标高：任何"从 0 开始"的件都往下埋 5cm。
/// 与地形平面严格共面的底面会在掠射角上抢同一批像素（Z-fighting）。
fn on_ground(v: f32) -> f32 {
    if v <= 0.0 { UNDER_GROUND } else { v }
}

/// 一个几何件。全部用**外尺寸 + 底/顶标高**描述（米），不用半尺寸和中心高。
#[derive(Copy, Clone)]
struct Part {
    x: f32,
    z: f32,
    /// 全宽 / 全深（= AABB 边长；圆柱/球时为外接盒边长）
    w: f32,
    d: f32,
    base: f32,
    top: f32,
    tint: [f32; 3],
    shape: Shape,
    kind: ObstacleKind,
}

impl Part {
    fn new(
        kind: ObstacleKind,
        x: f32,
        z: f32,
        w: f32,
        d: f32,
        base: f32,
        top: f32,
        tint: [f32; 3],
    ) -> Self {
        Part { x, z, w, d, base, top, tint, shape: Shape::Legacy, kind }
    }
    fn shape(mut self, s: Shape) -> Self {
        self.shape = s;
        self
    }
    fn cyl(self) -> Self {
        self.shape(Shape::Cylinder)
    }
    fn ico(self) -> Self {
        self.shape(Shape::Ico)
    }
    fn sph(self) -> Self {
        self.shape(Shape::Sphere)
    }
    /// 只碰撞、不绘制。给 GLB 道具的结构碰撞核用——盒子留在表里供物理/AI/伤害按下标
    /// 使用，但 `main.rs` 组装 marker 时会跳过它，避免与 GLB 表面共面 z-fighting。
    fn invisible(self) -> Self {
        self.shape(Shape::None)
    }

    fn to_obstacle(&self) -> MapObstacle {
        MapObstacle::new(self.kind, self.x, self.z, self.w * 0.5, self.d * 0.5)
            .shaped(
                (self.top - self.base) * 0.5,
                (self.top + self.base) * 0.5,
                Some(self.tint),
            )
            .geom(self.shape)
    }
}

/// 城市构造器：结构件与装饰件分表（理由见模块头注释）。
struct City {
    solid: Vec<MapObstacle>,
    decor: Vec<MapObstacle>,
    /// GLB 道具摆放。见 `LevelMap::props` 的说明——它换掉的是 decor 里那批薄盒。
    props: Vec<PropPlacement>,
    /// 已加载的道具网格集。放在构造器里而不是逐层透传，是为了让二十多个生成函数
    /// 的签名保持原样——它们只需要 `&mut City`。
    set: PropSet,
    /// 已按名解析出的网格下标缓存，避免每摆一件就重扫一遍表。
    mesh_cache: std::collections::HashMap<&'static str, usize>,
}

impl City {
    fn new() -> Self {
        City {
            solid: Vec::new(),
            decor: Vec::new(),
            props: Vec::new(),
            set: PropSet::default(),
            mesh_cache: std::collections::HashMap::new(),
        }
    }

    /// 会挡人/挡子弹/挡寻路的几何。
    fn push(&mut self, p: Part) -> &mut Self {
        self.solid.push(p.to_obstacle());
        self
    }

    /// 只进渲染与路径追踪的几何（檐口、窗带、壁柱、屋顶设备、树冠、路缘石…）。
    fn deco(&mut self, p: Part) -> &mut Self {
        self.decor.push(p.to_obstacle());
        self
    }

    /// 一个从地面立到 `h` 的盒子（默认结构件）。
    fn slab(&mut self, kind: ObstacleKind, x: f32, z: f32, w: f32, d: f32, h: f32, tint: [f32; 3]) {
        self.push(Part::new(kind, x, z, w, d, UNDER_GROUND, h, tint));
    }

    /// 一圈矩形环（女儿墙/台缘）。4 条边各自成体，转角互相重叠——重叠无害，缺角才致命。
    ///
    /// **短边必须容得下两条厚边**：侧边长度取 `min(w,d) - 2*thick`，一旦 `thick` 接近
    /// 短边就会算出**负尺寸**，负缩放的盒子会翻面，背面剔除后渲染成两个朝下的尖漏斗
    /// （2026-09-01 实机截图里广场正中那个"信封状悬浮平板"就是这么来的——仓库卷帘门框
    /// `rim(..., 4.4, 0.40, 0.40, ...)` 算出 `0.40 - 0.80 = -0.40`）。
    /// 现在短于 2 个厚度的方向直接不生成侧边，并由 `no_degenerate_geometry` 测试兜底。
    fn rim(&mut self, x: f32, z: f32, w: f32, d: f32, thick: f32, base: f32, top: f32, tint: [f32; 3]) {
        let hw = w * 0.5;
        let hd = d * 0.5;
        let t = thick.max(MIN_AXIS);
        self.deco(Part::new(ObstacleKind::Building, x, z - hd + t * 0.5, w, t, base, top, tint));
        self.deco(Part::new(ObstacleKind::Building, x, z + hd - t * 0.5, w, t, base, top, tint));
        // 侧边沿 z 走，可用长度是 d 扣掉上下两条边；扣完放不下就不生成
        let side = d - 2.0 * t;
        if side >= MIN_AXIS {
            self.deco(Part::new(ObstacleKind::Building, x - hw + t * 0.5, z, t, side, base, top, tint));
            self.deco(Part::new(ObstacleKind::Building, x + hw - t * 0.5, z, t, side, base, top, tint));
        }
    }

    /// 按名解析网格下标（带缓存）。解析不到返回 None，调用方退回程序化盒。
    fn mesh_ix(&mut self, name: &'static str) -> Option<usize> {
        if let Some(&i) = self.mesh_cache.get(name) {
            return Some(i);
        }
        let i = self.set.index_of(name)?;
        self.mesh_cache.insert(name, i);
        Some(i)
    }

    /// 摆一件 GLB 道具。资产没加载到就返回 false，调用方据此退回程序化盒——
    /// 缺资产的后果必须是"画面旧一点"，不能是"城里没有楼"。
    fn prop(&mut self, name: &'static str, x: f32, z: f32, yaw: f32, scale: f32) -> bool {
        let Some(mesh) = self.mesh_ix(name) else { return false };
        self.props.push(PropPlacement::new(mesh, x, z, yaw, scale, false));
        true
    }

    /// 同 [`Self::prop`]，但带抬升量（堆叠件用）。
    fn prop_y(&mut self, name: &'static str, x: f32, y: f32, z: f32, yaw: f32, scale: f32) -> bool {
        let Some(mesh) = self.mesh_ix(name) else { return false };
        self.props.push(PropPlacement::at(mesh, x, y, z, yaw, scale, false));
        true
    }

    /// 道具套件里有没有这件网格（生成函数用它决定走 GLB 还是退回程序化盒）。
    fn has_prop(&self, name: &str) -> bool {
        self.set.index_of(name).is_some()
    }

    /// 按目标 footprint 与层数，挑长宽比最接近的建筑变体，返回（名字，等比缩放）。
    ///
    /// 只用等比缩放：非等比会把窗洞拉成平行四边形，比"楼不够准"更刺眼。
    /// 缩放取 `max` 而不是 `min`，保证 GLB 始终**不小于**碰撞盒——宁可让玩家撞在一面
    /// 看得见的墙上，也不要被一面无形的墙挡住（后者会被当成寻路 bug 报上来）。
    fn pick_building(&self, w: f32, d: f32, floors: u32) -> Option<(&'static str, f32)> {
        let cands: &[&'static str] = match floors {
            0 | 1 => &["building_shed"],
            2 => &["building_block", "building_wide", "building_corner"],
            3 => &["building_tall", "building_wide"],
            _ => &["panel_block", "building_tall"],
        };
        let target = (w / d.max(0.01)).ln();
        let mut best: Option<(f32, &'static str, f32)> = None;
        for name in cands {
            let i = self.set.index_of(name)?;
            let (hx, hz) = self.set.get(i)?.half_footprint();
            let (gw, gd) = (hx * 2.0, hz * 2.0);
            if gw < 0.5 || gd < 0.5 {
                continue;
            }
            let cost = ((gw / gd).ln() - target).abs();
            let scale = (w / gw).max(d / gd);
            if best.is_none() || best.unwrap().0 > cost {
                best = Some((cost, name, scale));
            }
        }
        best.map(|(_, n, s)| (n, s))
    }

    /// 店面应朝最近的那条街。GLB 正面在引擎里是 -Z（Blender 的 +Y 经 export_yup 转过来），
    /// 绕 +Y 转 θ 后正面方向为 (-sinθ, 0, -cosθ)，据此反解四个朝向。
    fn face_nearest_street(cx: f32, cz: f32) -> f32 {
        let fx = (cx / STREET_EVERY).round() * STREET_EVERY;
        let fz = (cz / STREET_EVERY).round() * STREET_EVERY;
        if (cx - fx).abs() <= (cz - fz).abs() {
            // 街在 ±X 侧（街线沿 z 延伸）
            if cx - fx >= 0.0 { core::f32::consts::FRAC_PI_2 } else { -core::f32::consts::FRAC_PI_2 }
        } else if cz - fz >= 0.0 {
            core::f32::consts::PI
        } else {
            0.0
        }
    }

    fn finish(self) -> LevelMap {
        LevelMap { obstacles: self.solid, decor: self.decor, props: self.props }
    }
}

/// 街区中心（索引 0..5 → -137.5..137.5，与街道网格对齐）
fn bc(i: usize) -> f32 {
    -137.5 + i as f32 * STREET_EVERY
}

/// 街区角色表（索引 0..5 × 0..5）：
/// P=中央广场 O=写字楼塔楼 W=仓库园区 G=公园 S=商铺街 M=混合街区(楼+哨卡) C=住宅街区
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

/// 该街区是否以铺装地面为主（决定 `ground_zone` 返回地基还是草地）。
fn block_is_paved(role: char) -> bool {
    !matches!(role, 'P' | 'G')
}

// ============================================================
// 建筑生成器
// ============================================================

/// 一栋真实体量的楼：结构核心 + 逐层窗带/层线 + 角壁柱 + 女儿墙 + 屋顶设备 + 底层店面。
///
/// 进深阶梯（每一级都 ≥ RELIEF_STEP，所以没有任何两面共面）：
/// 核心 0.00 → 窗带 +0.30 → 层线 +0.44 → 壁柱 +0.68 → 裙墙 +0.62 → 女儿墙 +0.62 → 压顶 +0.72
fn building(
    c: &mut City,
    cx: f32,
    cz: f32,
    w: f32,
    d: f32,
    floors: u32,
    pal: Palette,
    seed_i: i32,
    seed_j: i32,
    storefront: bool,
) {
    let h = FLOOR_H * floors as f32;
    let plinth_h = 1.05;

    // GLB 路线：立面细节（窗洞、窗台、角石、檐口、屋顶、店面）全部来自建模资产，
    // 下面那批薄盒一条都不再生成。碰撞核留在表里（物理/AI/伤害按下标依赖它），但标成
    // 不可见，否则它的侧面会和 GLB 外墙共面打 z-fighting——那正是缺陷 D11 的成因。
    if let Some((name, scale)) = c.pick_building(w, d, floors) {
        let yaw = City::face_nearest_street(cx, cz);
        if c.prop(name, cx, cz, yaw, scale) {
            c.push(
                Part::new(ObstacleKind::Building, cx, cz, w, d, UNDER_GROUND, h, pal.wall)
                    .invisible(),
            );
            return;
        }
    }

    // 结构核心：唯一的挡人盒，从地面到屋顶。
    c.push(Part::new(ObstacleKind::Building, cx, cz, w, d, UNDER_GROUND, h, pal.wall));

    // 底层裙墙（石材）：凸出核心 0.62m，把"楼坐在地上"这件事画出来。
    c.deco(Part::new(ObstacleKind::Building, cx, cz, w + 1.24, d + 1.24, UNDER_GROUND, plinth_h, pal.base));

    // 逐层：窗带（凸 0.30）+ 层线 collar（凸 0.44，压在窗带上下沿，形成水平阴影线）
    for f in 0..floors {
        let fy = (plinth_h).max(FLOOR_H * f as f32);
        if fy + 1.0 >= h {
            break;
        }
        let band_base = fy + 0.62;
        let band_top = (fy + FLOOR_H - 0.42).min(h - 0.05);
        if band_top - band_base < MIN_AXIS {
            continue;
        }
        c.deco(Part::new(
            ObstacleKind::Block,
            cx,
            cz,
            w + 0.60,
            d + 0.60,
            band_base,
            band_top,
            pal.glass,
        ));
        c.deco(Part::new(
            ObstacleKind::Building,
            cx,
            cz,
            w + 0.88,
            d + 0.88,
            band_top,
            (band_top + 0.46).min(h),
            pal.trim,
        ));
    }

    // 角壁柱：四根通高，凸出核心 0.68，把"纸盒直角"变成有厚度的转角。
    let pw = 0.78;
    for (sx, sz) in [(-1.0f32, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        c.deco(Part::new(
            ObstacleKind::Building,
            cx + sx * (w * 0.5 + pw * 0.5 - 0.10),
            cz + sz * (d * 0.5 + pw * 0.5 - 0.10),
            pw,
            pw,
            UNDER_GROUND,
            h + 0.55,
            pal.trim,
        ));
    }

    // 女儿墙 + 压顶：屋顶与天空的交界必须有厚度，否则楼读起来像一块平板。
    // 压顶只给 4 层以上的楼——低楼屋顶在街面上几乎看不见，不值得 4 个实例。
    let parapet = 1.05;
    c.rim(cx, cz, w + 1.24, d + 1.24, 0.42, h, h + parapet, pal.trim);
    if floors >= 4 {
        c.rim(cx, cz, w + 1.44, d + 1.44, 0.52, h + parapet, h + parapet + 0.22, pal.base);
    }

    // 屋面（深色防水卷材）：屋顶面积大、正对天光，与立面同色就会糊成一片。
    c.deco(Part::new(ObstacleKind::Block, cx, cz, w - 0.5, d - 0.5, h + 0.02, h + 0.26, ROOF_MEMBRANE));

    // 屋顶设备：楼梯间 + 两台机组 + 一根通风管（圆柱）。
    let ox = mixf(seed_i, seed_j, -0.22, 0.22) * w;
    let oz = mixf(seed_j, seed_i, -0.22, 0.22) * d;
    let deck = h + 0.26;
    c.deco(Part::new(ObstacleKind::Building, cx + ox, cz + oz, 3.2, 2.6, deck, deck + 2.6, pal.trim));
    c.deco(Part::new(ObstacleKind::Block, cx + ox + 3.6, cz + oz, 1.8, 1.4, deck, deck + 1.14, METAL_GRAY));
    c.deco(Part::new(ObstacleKind::Block, cx + ox + 3.6, cz + oz, 1.9, 1.5, deck + 1.14, deck + 1.40, CONCRETE_DARK));
    c.deco(Part::new(ObstacleKind::Block, cx - ox - 3.2, cz + oz - 2.6, 1.5, 1.5, deck, deck + 0.84, METAL_RUST));
    c.deco(
        Part::new(ObstacleKind::Block, cx - ox - 4.4, cz + oz + 2.4, 0.55, 0.55, deck, deck + 1.94, METAL_GRAY)
            .cyl(),
    );

    // 底层店面：凹入门廊 + 雨棚 + 圆柱。
    if storefront {
        let face = d * 0.5;
        c.deco(Part::new(
            ObstacleKind::Block,
            cx,
            cz + face + 0.10,
            w - 2.0,
            0.30,
            plinth_h,
            plinth_h + 2.30,
            GLASS_DARK,
        ));
        c.deco(Part::new(
            ObstacleKind::Building,
            cx,
            cz + face + 1.05,
            w - 0.6,
            2.30,
            plinth_h + 2.40,
            plinth_h + 2.86,
            pal.base,
        ));
        for k in 0..4 {
            let px = cx + (k as f32 - 1.5) * ((w - 2.6) / 3.0);
            c.deco(
                Part::new(
                    ObstacleKind::Block,
                    px,
                    cz + face + 1.85,
                    0.46,
                    0.46,
                    UNDER_GROUND,
                    plinth_h + 2.40,
                    pal.trim,
                )
                .cyl(),
            );
        }
    }
}

/// 低层联排：逐层通长窗带 + 腰线 + 逐开间窗台 + 阶梯近似坡屋顶。
///
/// 为什么不做"每开间每层一个窗盒"：那样一栋 5 层 6 开间的楼就是 120 个件，
/// 全城 8 个住宅街区直接冲破 4096 的 marker 预算（第一版实测 6964 件）。
/// 通长窗带 1 件覆盖四个立面，配逐开间窗台，视觉密度接近、成本 1/6。
fn row_houses(
    c: &mut City,
    cx: f32,
    cz: f32,
    w: f32,
    d: f32,
    floors: u32,
    pal: Palette,
    bays: u32,
) {
    let h = FLOOR_H * floors as f32 + 0.6;
    c.push(Part::new(ObstacleKind::Building, cx, cz, w, d, UNDER_GROUND, h, pal.wall));
    c.deco(Part::new(ObstacleKind::Building, cx, cz, w + 0.5, d + 0.5, UNDER_GROUND, 0.80, pal.base));

    // 坡屋顶：4 级递减近似。第 0 级就必须内缩，否则侧立面与墙身共面。
    let steps = 4i32;
    for s in 0..steps {
        let inset = (s + 1) as f32 * (d * 0.5) / (steps as f32 + 1.0);
        let rd = (d - 2.0 * inset).max(MIN_AXIS);
        c.deco(Part::new(
            ObstacleKind::Building,
            cx,
            cz,
            w + 0.7,
            rd,
            h + 0.10 + s as f32 * 0.60,
            h + 0.78 + s as f32 * 0.60,
            if s == steps - 1 { ROOF_MEMBRANE } else { pal.trim },
        ));
    }

    // 逐层：通长窗带（凸 0.30）+ 腰线（凸 0.44）
    for f in 0..floors {
        let fy = 1.05 + f as f32 * FLOOR_H;
        if fy + 1.9 > h {
            break;
        }
        c.deco(Part::new(
            ObstacleKind::Block,
            cx,
            cz,
            w + 0.60,
            d + 0.60,
            fy,
            fy + 1.45,
            pal.glass,
        ));
        c.deco(Part::new(
            ObstacleKind::Building,
            cx,
            cz,
            w + 0.88,
            d + 0.88,
            fy + 1.45,
            fy + 1.80,
            pal.trim,
        ));
    }

    // 逐开间竖梃：一件通高，把"一整条玻璃带"切回一格一格的窗
    let bay_w = w / bays as f32;
    for b in 1..bays {
        let bx = cx + (b as f32 - bays as f32 * 0.5) * bay_w;
        for side in [-1.0f32, 1.0] {
            c.deco(Part::new(
                ObstacleKind::Building,
                bx,
                cz + side * (d * 0.5 + 0.22),
                0.34,
                0.44,
                0.80,
                h - 0.10,
                pal.trim,
            ));
        }
    }
}

/// 仓库：混凝土壳体 + 下沉式天窗 + 凹进卷帘门 + 装卸平台 + 集装箱堆场。
fn warehouse(c: &mut City, cx: f32, cz: f32, seed_i: i32, seed_j: i32) {
    let w = mixf(seed_i, seed_j, 26.0, 34.0);
    let d = mixf(seed_j, seed_i, 18.0, 24.0);
    let h = mixf(seed_i, seed_j + 7, 8.0, 11.0);
    c.push(Part::new(ObstacleKind::Building, cx, cz, w, d, UNDER_GROUND, h, CONCRETE_LIGHT));
    c.deco(Part::new(ObstacleKind::Building, cx, cz, w + 0.7, d + 0.7, UNDER_GROUND, 1.2, GRANITE));
    c.deco(Part::new(ObstacleKind::Building, cx, cz, w + 0.9, d + 0.9, h - 0.9, h, CONCRETE_DARK));

    // 屋面天窗：下沉 0.5m 的玻璃槽 + 两侧挡边（有真实进深，不是贴皮）。
    // 2 条而不是 3 条：仓库只有 4 座，但每条天窗要 3 个件才不共面。
    for k in [-1i32, 1] {
        let z = cz + k as f32 * (d * 0.26);
        c.deco(Part::new(ObstacleKind::Block, cx, z, w - 6.0, 2.6, h - 0.55, h - 0.10, GLASS_BLUE));
        for s in [-1.0f32, 1.0] {
            c.deco(Part::new(
                ObstacleKind::Building,
                cx,
                z + s * 1.75,
                w - 6.0,
                0.42,
                h - 0.60,
                h + 0.22,
                CONCRETE_DARK,
            ));
        }
    }

    // 卷帘门：门叶凹在门框之后（门框凸 0.30，门叶凸 0.05，背面埋进墙里）
    let face = cz + d * 0.5;
    for k in 0..3 {
        let x = cx + (k as f32 - 1.0) * 8.0;
        c.deco(Part::new(
            ObstacleKind::Block,
            x,
            face - INSET_BURIED + 0.125,
            3.4,
            0.25,
            UNDER_GROUND,
            4.0,
            METAL_GRAY,
        ));
        c.rim(x, face + 0.15, 4.4, 0.40, 0.40, UNDER_GROUND, 4.5, CONCRETE_DARK);
    }
    // 装卸平台
    c.push(Part::new(ObstacleKind::Building, cx, face + 2.0, w - 4.0, 3.2, UNDER_GROUND, 1.15, CONCRETE));
    c.deco(Part::new(ObstacleKind::Building, cx, face + 3.75, w - 4.0, 0.40, 1.15, 1.42, CONCRETE_DARK));

    // 集装箱堆场：GLB 的 ISO 箱体（6.058 × 2.438 × 2.591）自带波纹、门端锁杆、
    // 八角角件与顶部加强筋，正好替掉原来"箱体 + 门端面 + 顶筋"三件套。
    let colors = [CONTAINER_RUST, CONTAINER_BLUE, CONTAINER_GREEN];
    const CONTAINER_PROPS: [&'static str; 3] =
        ["container_20ft", "container_navy", "container_green"];
    for i in 0..3i32 {
        for j in 0..2i32 {
            let bx = cx + w * 0.5 + 6.5 + i as f32 * 6.6;
            let bz = cz - d * 0.4 + j as f32 * 6.6;
            let col = colors[(i + j) as usize % 3];
            if c.prop(CONTAINER_PROPS[(i + j) as usize % 3], bx, bz, 0.0, 1.0) {
                c.push(
                    Part::new(ObstacleKind::Block, bx, bz, 6.1, 2.5, UNDER_GROUND, 2.6, col)
                        .invisible(),
                );
                if i == 1 && j == 0 {
                    // 第二层：抬一个箱高（2.85）叠在同一列上
                    c.prop_y(
                        CONTAINER_PROPS[(i + j + 1) as usize % 3],
                        bx,
                        2.85,
                        bz,
                        0.0,
                        1.0,
                    );
                    c.push(Part::new(ObstacleKind::Block, bx, bz, 6.1, 2.5, 2.85, 5.45, col).invisible());
                }
                continue;
            }
            c.push(Part::new(ObstacleKind::Block, bx, bz, 6.1, 2.5, UNDER_GROUND, 2.6, col));
            c.deco(Part::new(ObstacleKind::Block, bx + 3.2, bz, 0.30, 2.2, 0.15, 2.45, DARK));
            c.deco(Part::new(ObstacleKind::Block, bx, bz, 6.2, 2.6, 2.6, 2.85, CONCRETE_DARK));
            if i == 1 && j == 0 {
                c.push(Part::new(ObstacleKind::Block, bx, bz, 6.1, 2.5, 2.85, 5.45, CONTAINER_GREEN));
                c.deco(Part::new(ObstacleKind::Block, bx + 3.2, bz, 0.30, 2.2, 3.0, 5.30, DARK));
            }
        }
    }
}

/// 中央广场：纪念碑 + 花坛 + 长椅 + 旗杆（地面交给纹理，不放几何地台）。
fn plaza(c: &mut City, cx: f32, cz: f32, monument: bool) {
    // **不要在这里铺"整块薄抬台"。** 第一版铺了一张 34m×34m、只有 0.29m 厚的板，
    // 实机截图里它就是一张篮球场大小的混凝土桌板浮在地面上——正是本次重构要消灭的
    // "纸片"，只是被我搬到了地面尺度。而 `no_paper_thin_geometry` 豁免了底面埋进地下
    // 的件（那个豁免是为"人行道抬台=它就是地面"写的），于是测试放行、画面更假。
    // 广场地面交给地面纹理（ground_zone 的 zone 4 广场铺装），几何只留真具体量的构件。
    if monument {
        // 台基：三级"深踏面 + 低 riser"的真实台阶（出挑 0.85m / rise 0.20m），
        // 而不是三张薄饼。旧版 9.2m 宽只抬 0.34m，比例就是"板"。
        // 顶面一律按 (s+1)*0.20 算、不按 base+0.20 算：第一级底面要埋进地下 5cm
        // （避免与地形共面），若用 base+0.20 它的顶面就只剩 0.15，与第二级的 0.20
        // 之间留 5cm 悬空缝——`solid_obstacles_reach_the_ground` 就是为抓这个而写的。
        for s in 0..3i32 {
            let half = 6.6 - s as f32 * 0.85;
            c.push(Part::new(
                ObstacleKind::Building,
                cx,
                cz,
                half * 2.0,
                half * 2.0,
                on_ground(s as f32 * 0.20),
                (s + 1) as f32 * 0.20,
                if s == 0 { GRANITE } else { CONCRETE_LIGHT },
            ));
        }
        // 碑座 + 三段收分碑身 + 金属顶帽 + 球形刹尖
        c.push(Part::new(ObstacleKind::Building, cx, cz, 3.4, 3.4, 0.60, 1.30, GRANITE));
        c.push(Part::new(ObstacleKind::Building, cx, cz, 2.6, 2.6, 1.30, 5.6, CONCRETE_LIGHT));
        c.push(Part::new(ObstacleKind::Building, cx, cz, 2.2, 2.2, 5.6, 8.8, CONCRETE_LIGHT));
        c.push(Part::new(ObstacleKind::Building, cx, cz, 1.8, 1.8, 8.8, 11.2, CONCRETE));
        c.deco(Part::new(ObstacleKind::Building, cx, cz, 2.4, 2.4, 11.2, 11.62, METAL_GRAY));
        c.deco(Part::new(ObstacleKind::Block, cx, cz, 1.0, 1.0, 11.62, 12.7, FLAG_POLE).sph());
        for (dx, dz) in [(-7.5f32, -7.5), (7.5, -7.5), (-7.5, 7.5), (7.5, 7.5)] {
            c.push(Part::new(ObstacleKind::Block, cx + dx, cz + dz, 0.26, 0.26, UNDER_GROUND, 7.2, FLAG_POLE).cyl());
            c.deco(Part::new(ObstacleKind::Block, cx + dx, cz + dz, 0.30, 0.30, 7.2, 7.5, FLAG_POLE).sph());
            c.push(Part::new(ObstacleKind::Block, cx + dx, cz + dz, 0.9, 0.9, UNDER_GROUND, 0.45, GRANITE).cyl());
        }
    }

    // 花坛：石框 + 真灌木
    for (dx, dz) in [(-13.0f32, -13.0), (13.0, -13.0), (-13.0, 13.0), (13.0, 13.0)] {
        let (ax, az) = (cx + dx, cz + dz);
        c.push(Part::new(ObstacleKind::Building, ax, az, 4.0, 4.0, UNDER_GROUND, 0.62, GRANITE));
        c.deco(Part::new(ObstacleKind::Tree, ax, az, 3.4, 3.4, 0.55, 1.95, TREE_LEAF_2).sph());
        c.deco(Part::new(ObstacleKind::Tree, ax - 0.9, az + 0.6, 1.5, 1.5, 1.4, 2.7, TREE_LEAF_3).sph());
    }

    // 长椅：座面 + 靠背 + 四条腿（旧版是一块悬空的板）
    for (dx, dz, along) in [(-8.0f32, 0.0, true), (8.0, 0.0, true), (0.0, -8.0, false), (0.0, 8.0, false)] {
        bench(c, cx + dx, cz + dz, along);
    }

    // 广场本身要有内容，否则中央 4 块就是 110m×110m 的盐碱地（实测截图里画面
    // 55% 是这块空地）。全部限制在街区中心 ±13m 内，保证出生点 10m 净空不破。
    // 柱廊：两侧各 6 根圆柱 + 通长檐梁（有顶的步行空间，尺度感立刻不一样）
    for side in [-1.0f32, 1.0] {
        for k in 0..6i32 {
            let px = cx + (k as f32 - 2.5) * 4.2;
            let pz = cz + side * 12.5;
            c.push(Part::new(ObstacleKind::Block, px, pz, 0.62, 0.62, UNDER_GROUND, 4.6, PLASTER_CREAM).cyl());
            c.deco(Part::new(ObstacleKind::Block, px, pz, 0.9, 0.9, UNDER_GROUND, 0.34, GRANITE).cyl());
            c.deco(Part::new(ObstacleKind::Block, px, pz, 0.92, 0.92, 4.6, 4.95, GRANITE).cyl());
        }
        c.deco(Part::new(ObstacleKind::Building, cx, cz + side * 12.5, 25.0, 1.5, 4.95, 5.55, CONCRETE));
    }
    if !monument {
        // 喷泉水池（非纪念碑块）：石缘 + 内凹水面，给广场一个视觉中心
        c.push(Part::new(ObstacleKind::Building, cx, cz, 9.0, 9.0, UNDER_GROUND, 0.62, GRANITE));
        c.deco(Part::new(ObstacleKind::Block, cx, cz, 7.6, 7.6, 0.30, 0.56, GLASS_BLUE));
        c.push(Part::new(ObstacleKind::Block, cx, cz, 1.3, 1.3, 0.56, 2.3, CONCRETE_LIGHT).cyl());
        c.deco(Part::new(ObstacleKind::Block, cx, cz, 2.6, 2.6, 2.3, 2.55, GRANITE));
    }
    // 广场树阵（两排，避开中心与出生净空）
    for k in 0..3i32 {
        let px = cx + (k as f32 - 1.0) * 9.0;
        tree(c, px, cz - 18.5, k + 3, 1);
        tree(c, px, cz + 18.5, k + 3, 2);
    }
}

/// 一条长椅（`along` = 长边沿 x 轴）。
fn bench(c: &mut City, x: f32, z: f32, along: bool) {
    let (w, d) = if along { (2.4, 0.62) } else { (0.62, 2.4) };
    c.push(Part::new(ObstacleKind::Block, x, z, w, d, 0.42, 0.54, WOOD_BENCH));
    let (bw, bd) = if along { (w, 0.22) } else { (0.22, d) };
    let (bx, bz) = if along { (x, z - 0.20) } else { (x - 0.20, z) };
    c.push(Part::new(ObstacleKind::Block, bx, bz, bw, bd, 0.54, 1.18, WOOD_BENCH));
    for s in [-1.0f32, 1.0] {
        let (lx, lz) = if along { (x + s * (w * 0.5 - 0.28), z) } else { (x, z + s * (d * 0.5 - 0.28)) };
        c.push(Part::new(ObstacleKind::Block, lx, lz, 0.22, 0.5, UNDER_GROUND, 0.42, DARK));
    }
}

/// 公园：真树（圆柱树干 + 不压扁的二十面体树冠）+ 灌木 + 座椅 + 垃圾桶。
fn park(c: &mut City, cx: f32, cz: f32) {
    for ti in 0..3i32 {
        for tj in 0..3i32 {
            if ti == 1 && tj == 1 {
                continue;
            }
            tree(c, cx + (ti - 1) as f32 * 13.0, cz + (tj - 1) as f32 * 13.0, ti, tj);
        }
    }
    for (dx, dz, s) in [(-19.0f32, 6.0, 1), (19.0, -6.0, 2), (6.0, 19.0, 3), (-6.0, -19.0, 4)] {
        bush(c, cx + dx, cz + dz, s);
    }
    for dx in [-6.0f32, 6.0] {
        bench(c, cx + dx, cz - 4.0, true);
        let (bx, bz) = (cx + dx + 1.8, cz);
        c.push(Part::new(ObstacleKind::Block, bx, bz, 0.72, 0.72, UNDER_GROUND, 0.92, METAL_GRAY).cyl());
        c.deco(Part::new(ObstacleKind::Block, bx, bz, 0.8, 0.8, 0.92, 1.14, CONCRETE_DARK).cyl());
    }
}

/// 一棵有体积的树。
///
/// ## 两次误诊，都记下来免得再犯
/// 实机截图里树是"一簇彼此分离的扁平绿片"。我先归因给**顶点抖动撕裂**——错的：
/// `build.rs:59` 那段 foliage 揉皱在 `vs_main`（传统顶点管线）里，而本机日志明确跑
/// mesh 路径，mesh 路径的 `m_ico`/`m_sph`/`is_tree` 分支直接取 `ICO_POS`/`SPH_POS`，
/// **没有任何逐顶点位移**。接着想靠"12 顶点换 42 顶点"解决，也只对了一半。
///
/// 真因是**我自己叠出来的**：一棵树放了 3 个半径不同、圆心互相错开 0.4~0.5m 的
/// 多面体，而本引擎法线来自屏幕导数 → **每个三角面纯平着色**。三个凸多面体互相
/// 穿插 + 硬明暗切面，读起来就是一堆散乱碎玻璃。网格本身一直是水密的。
///
/// ## 现在的做法
/// 1. **两个冠而不是三个**，且**同心**（只允许很小的偏心），避免穿插产生的碎片轮廓；
/// 2. 一律 [`Shape::Sphere`]（42 顶点 / 80 三角），面数够多，平着色下也接近圆润；
/// 3. 冠高与冠宽接近等轴（不再压成饼），下缘落在树干顶附近，遮住"冠与干的接缝"。
fn tree(c: &mut City, x: f32, z: f32, i: i32, j: i32) {
    let k = mixf(i, j, 0.85, 1.25);
    let trunk_h = mixf(i + 5, j, 3.0, 4.4);
    if c.prop("tree_oak", x, z, mixf(i, j + 3, 0.0, std::f32::consts::TAU), k) {
        // 只留一根细碰撞柱，且必须**严格细于网格树干**（tree_oak 底部半径 0.34m），
        // 否则方盒会从圆干里戳出来——那是比"碎玻璃冠"更难看的穿帮。
        // 半宽 0.15 < 0.34·k（k≥0.85 → 0.29），留了一倍余量。
        c.push(Part::new(ObstacleKind::Tree, x, z, 0.30, 0.30, UNDER_GROUND, trunk_h, TREE_BARK).cyl());
        return;
    }
    // 回退：没有资产时维持原来的两冠同心方案（见上面的误诊记录，别再叠第三个冠）
    // 树干：圆柱，底部略粗（两段叠出锥形感）
    c.push(Part::new(ObstacleKind::Tree, x, z, 0.52 * k, 0.52 * k, UNDER_GROUND, trunk_h * 0.55, TREE_BARK).cyl());
    c.push(Part::new(ObstacleKind::Tree, x, z, 0.38 * k, 0.38 * k, trunk_h * 0.5, trunk_h + 0.6, TREE_BARK).cyl());
    let leaf = [TREE_LEAF, TREE_LEAF_2, TREE_LEAF_3][(i + j).rem_euclid(3) as usize];
    // 主冠：同心、接近等轴，把树干顶端口罩住
    let r = 2.15 * k;
    c.deco(
        Part::new(
            ObstacleKind::Tree,
            x,
            z,
            r * 2.0,
            r * 2.0,
            trunk_h - 0.35,
            trunk_h - 0.35 + r * 2.05,
            leaf,
        )
        .sph(),
    );
    // 副冠：小幅度偏心（0.22k 以内，不再互相穿插出尖角），抬高形成树形层次
    let r2 = 1.35 * k;
    c.deco(
        Part::new(
            ObstacleKind::Tree,
            x + 0.20 * k,
            z - 0.16 * k,
            r2 * 2.0,
            r2 * 2.0,
            trunk_h + r * 1.05,
            trunk_h + r * 1.05 + r2 * 2.0,
            leaf,
        )
        .sph(),
    );
}

/// 灌木：两团同心、不互相穿插的细分球。
fn bush(c: &mut City, x: f32, z: f32, seed: i32) {
    let r = mixf(seed, 1, 0.95, 1.45);
    c.deco(Part::new(ObstacleKind::Tree, x, z, r * 2.0, r * 2.0, UNDER_GROUND, r * 1.55, TREE_LEAF_2).sph());
    c.deco(
        Part::new(
            ObstacleKind::Tree,
            x + r * 0.22,
            z - r * 0.18,
            r * 1.3,
            r * 1.3,
            UNDER_GROUND,
            r * 1.95,
            TREE_LEAF_3,
        )
        .sph(),
    );
}

/// 哨卡/检查站：错缝堆叠的沙袋墙、HESCO 桶、帐篷、车辆残骸、拒马。
fn checkpoint(c: &mut City, cx: f32, cz: f32) {
    // 沙袋护墙：优先一整件 GLB（3.3 × 0.75 × 0.72）。旧版逐袋一个扁二十面体，
    // 15 件里每件的碰撞/弹道/渲染成本都要付一遍，而街面上它们本来就糊成一堵墙。
    if c.prop("sandbag_wall", cx, cz - 7.0, 0.0, 1.0) {
        c.push(
            Part::new(ObstacleKind::Barrier, cx, cz - 7.0, 3.3, 0.75, UNDER_GROUND, 0.72, SANDBAG)
                .invisible(),
        );
    } else {
        for row in 0..3i32 {
            for k in 0..5i32 {
                let px = cx + (k as f32 - 2.0) * 1.35 + (row % 2) as f32 * 0.6;
                let py = row as f32 * 0.36;
                c.push(
                    Part::new(ObstacleKind::Barrier, px, cz - 7.0, 1.45, 0.70, on_ground(py), py + 0.48, SANDBAG).sph(),
                );
            }
        }
    }
    // HESCO 防爆桶：一件 GLB = 网箱 +  liner + 顶部土脊，替掉原来的柱+盖两件套
    for k in 0..4i32 {
        let bx = cx + (k as f32 - 1.5) * 4.2;
        if c.prop("barrier_hesco", bx, cz + 7.0, 0.0, 1.0) {
            c.push(
                Part::new(ObstacleKind::Barrier, bx, cz + 7.0, 2.0, 1.1, UNDER_GROUND, 1.05, SANDBAG)
                    .invisible(),
            );
            continue;
        }
        c.push(Part::new(ObstacleKind::Block, bx, cz + 7.0, 2.0, 2.0, UNDER_GROUND, 1.5, SANDBAG).cyl());
        c.deco(Part::new(ObstacleKind::Barrier, bx, cz + 7.0, 1.7, 1.7, 1.5, 1.95, CONCRETE_DARK).sph());
    }
    // 帐篷：4 级递减近似双坡面
    let (tx, tz) = (cx - 12.0, cz);
    for s in 0..4i32 {
        c.push(Part::new(
            ObstacleKind::Building,
            tx,
            tz,
            5.0 - s as f32 * 1.2,
            3.6 - s as f32 * 0.8,
            on_ground(s as f32 * 0.62),
            (s + 1) as f32 * 0.62,
            TENT_CAMO,
        ));
    }
    // 车辆残骸
    wreck_car(c, cx + 12.0, cz + 2.0, WRECK_TAN);
    // 拒马
    for k in 0..3i32 {
        let px = cx + (k as f32 - 1.0) * 3.4;
        c.push(Part::new(ObstacleKind::Barrier, px, cz, 2.2, 0.24, UNDER_GROUND, 1.25, METAL_RUST));
        c.push(Part::new(ObstacleKind::Barrier, px, cz, 0.24, 2.2, UNDER_GROUND, 1.25, METAL_RUST));
    }
}

/// 一辆报废车：优先用 GLB（真轿车侧影），没有资产时退回盒子堆。
fn wreck_car(c: &mut City, x: f32, z: f32, tint: [f32; 3]) {
    if c.prop("car_wreck", x, z, mixf(x as i32, z as i32, 0.0, std::f32::consts::TAU), 1.0) {
        // 碰撞体沿用原来的车壳尺寸：网格含一扇敞开的车门（4.7 × 2.72），
        // 所以 4.4 × 2.1 的核必然埋在车身里，不会戳出轮廓。
        c.push(Part::new(ObstacleKind::Ruin, x, z, 4.4, 2.1, UNDER_GROUND, 1.55, tint).invisible());
        return;
    }
    c.push(Part::new(ObstacleKind::Ruin, x, z, 4.4, 2.1, UNDER_GROUND, 1.3, tint));
    c.push(Part::new(ObstacleKind::Ruin, x - 0.4, z, 2.0, 1.95, 1.3, 2.15, WRECK_GLASS));
    c.deco(Part::new(ObstacleKind::Ruin, x + 1.9, z, 0.5, 1.9, 0.35, 1.05, GLASS_DARK));
    for (ox, oz) in [(-1.45f32, -1.05), (1.45, -1.05), (-1.45, 1.05), (1.45, 1.05)] {
        c.deco(Part::new(ObstacleKind::Ruin, x + ox, z + oz, 0.78, 0.32, UNDER_GROUND, 0.78, DARK).cyl());
    }
}

/// 停车场：残骸车阵 + 车位缘石带 + 灯杆。
fn parking_lot(c: &mut City, cx: f32, cz: f32) {
    let tints = [WRECK_TAN, [0.30, 0.32, 0.36], [0.36, 0.30, 0.24], [0.26, 0.30, 0.28]];
    for (idx, t) in tints.iter().enumerate() {
        let (dx, dz) = match idx {
            0 => (-9.0, -7.0),
            1 => (-2.0, -7.0),
            2 => (5.0, 5.0),
            _ => (12.0, 5.0),
        };
        wreck_car(c, cx + dx, cz + dz, *t);
    }
    // 车位分隔：凸出地面的缘石带（旧版是与地面共面的贴皮，掠射角整片消失）
    for k in 0..4i32 {
        let z = cz - 9.0 + k as f32 * 6.0;
        c.deco(Part::new(ObstacleKind::Building, cx, z, 26.0, 0.24, UNDER_GROUND, 0.18, CONCRETE_LIGHT));
    }
    for dx in [-11.0f32, 11.0] {
        lamp_post(c, cx + dx, cz);
    }
}

/// 路灯：优先用 GLB（锥形杆 + 弯臂 + 灯头），没有资产时退回圆柱堆。
fn lamp_post(c: &mut City, x: f32, z: f32) {
    if c.prop("street_lamp", x, z, mixf(x as i32, z as i32, 0.0, std::f32::consts::TAU), 1.0) {
        // 碰撞只给一根埋在杆身里的细柱：street_lamp 杆底半径 0.13、往上收，
        // 半宽 0.10 恒小于它，所以盒子不会从圆杆里戳出来（戳出来比"灯杆是方的"更糟）。
        c.push(Part::new(ObstacleKind::Block, x, z, 0.20, 0.20, UNDER_GROUND, 4.6, DARK).cyl().invisible());
        return;
    }
    c.push(Part::new(ObstacleKind::Block, x, z, 0.34, 0.34, UNDER_GROUND, 2.4, DARK).cyl());
    c.push(Part::new(ObstacleKind::Block, x, z, 0.22, 0.22, 2.4, 6.6, DARK).cyl());
    c.push(Part::new(ObstacleKind::Block, x, z, 1.5, 0.24, 6.6, 6.85, DARK));
    c.deco(Part::new(ObstacleKind::Block, x + 0.62, z, 0.85, 0.5, 6.32, 6.62, LAMP_GLOW).sph());
    c.deco(Part::new(ObstacleKind::Block, x, z, 0.72, 0.72, UNDER_GROUND, 0.34, GRANITE).cyl());
}

/// 消防栓：圆体 + 顶帽 + 两个侧口 + 法兰底座。
fn hydrant(c: &mut City, x: f32, z: f32) {
    c.push(Part::new(ObstacleKind::Block, x, z, 0.42, 0.42, UNDER_GROUND, 0.78, HYDRANT_RED).cyl());
    c.deco(Part::new(ObstacleKind::Block, x, z, 0.46, 0.46, 0.78, 1.00, HYDRANT_RED).sph());
    c.deco(Part::new(ObstacleKind::Block, x, z, 0.72, 0.24, 0.38, 0.62, METAL_RUST));
    c.deco(Part::new(ObstacleKind::Block, x, z, 0.24, 0.72, 0.38, 0.62, METAL_RUST));
    c.deco(Part::new(ObstacleKind::Block, x, z, 0.62, 0.62, UNDER_GROUND, 0.20, GRANITE).cyl());
}

// ============================================================
// 街区装配
// ============================================================

/// 写字楼街区：1 座主塔 + 2 座裙楼。
fn office_block(c: &mut City, cx: f32, cz: f32, i: usize, j: usize) {
    let (pi, pj) = (i as i32, j as i32);
    let pal = palette_at(pi + pj);
    let floors = (6.0 + hash01(pi, pj) * 8.0).floor() as u32;
    building(c, cx, cz, 20.0, 20.0, floors, pal, pi, pj, false);
    building(c, cx - 15.5, cz + 13.0, 11.0, 9.0, 3, palette_at(pi * 2 + pj + 1), pi + 3, pj, true);
    building(c, cx + 14.0, cz - 14.0, 10.0, 11.0, 4, pal, pi, pj + 5, true);
    tree(c, cx - 6.0, cz + 20.0, pi, pj);
    hydrant(c, cx + 12.0, cz + 18.0);
}

/// 商铺街：3 座联排 + 骑楼雨棚与柱廊。
fn shop_block(c: &mut City, cx: f32, cz: f32, i: usize, j: usize) {
    let (pi, pj) = (i as i32, j as i32);
    for k in 0..3u32 {
        let bx = cx + (k as f32 - 1.0) * 14.5;
        let pal = palette_at(pi + pj + k as i32);
        let floors = 2 + (pi + j as i32 + k as i32).rem_euclid(3) as u32;
        row_houses(c, bx, cz, 12.0, 9.5, floors, pal, 3);
    }
    c.deco(Part::new(ObstacleKind::Building, cx, cz + 6.4, 44.0, 2.6, 3.6, 3.95, GRANITE));
    for k in 0..5i32 {
        let px = cx + (k as f32 - 2.0) * 10.5;
        let col = if k % 2 == 0 { CONCRETE_DARK } else { GRANITE };
        c.push(Part::new(ObstacleKind::Block, px, cz + 7.4, 0.4, 0.4, UNDER_GROUND, 3.6, col).cyl());
    }
    for dx in [-14.0f32, 14.0] {
        lamp_post(c, cx + dx, cz + 9.5);
    }
}

/// 沿街围合：4 条板楼贴着街区边界拼成一圈，中间留内院。
///
/// **这是本次重构最该做的一件事。** 第一版我把力气全花在"把单栋楼做细"上，
/// 实机截图却是 55% 空地面 + 40% 空天空、地平线上几个小盒子——因为
/// "55m 街区网格 + 每块一栋 20m 楼 + 其余全是空地"根本不是城市。
/// 真实街区的定义性特征是**建筑贴着街道红线建、背面围出内院**，
/// 街道因此有"墙"，人走在里面有尺度感和方向感。差别在这里，不在单体细节。
///
/// `open_side` 指定某一边不建（留给内院活动场地，如哨卡/停车）。
fn perimeter_ring(c: &mut City, cx: f32, cz: f32, floors: u32, seed_i: i32, seed_j: i32, open_side: Option<i32>) {
    let s = BLOCK_EDGE; // 37m：街区内缘（人行道以内）
    let depth = 11.0;
    for side in 0..4i32 {
        if open_side == Some(side) {
            continue;
        }
        let pal = palette_at(seed_i * 2 + seed_j * 3 + side);
        let (ox, oz, w, d, bays) = match side {
            0 => (0.0, -s * 0.5 + depth * 0.5, s - depth, depth, 5),
            1 => (0.0, s * 0.5 - depth * 0.5, s - depth, depth, 5),
            2 => (-s * 0.5 + depth * 0.5, 0.0, depth, s - depth, 4),
            _ => (s * 0.5 - depth * 0.5, 0.0, depth, s - depth, 4),
        };
        row_houses(c, cx + ox, cz + oz, w, d, floors, pal, bays);
    }
}

/// 住宅街区：沿街围合 + 内院（树、长椅、消防栓、车位）。
fn residential_block(c: &mut City, cx: f32, cz: f32, i: usize, j: usize) {
    let (pi, pj) = (i as i32, j as i32);
    let floors = 3 + (hash01(pi, pj + 2) * 3.0).floor() as u32; // 3..5 层
    perimeter_ring(c, cx, cz, floors, pi, pj, None);
    // 内院：树 + 长椅 + 消防栓 + 车位线
    tree(c, cx - 5.0, cz - 4.0, pi + 7, pj);
    tree(c, cx + 5.0, cz + 4.0, pi, pj + 7);
    bush(c, cx + 6.0, cz - 5.0, pi + pj);
    bench(c, cx - 6.0, cz + 5.0, true);
    hydrant(c, cx, cz);
    for k in 0..4i32 {
        let z = cz - 4.0 + k as f32 * 2.7;
        c.deco(Part::new(ObstacleKind::Building, cx + 11.0, z, 5.0, 0.24, UNDER_GROUND, 0.18, CONCRETE_LIGHT));
    }
}

/// 混合街区：三面围合 + 一面敞开的军事哨卡院。
fn mixed_block(c: &mut City, cx: f32, cz: f32, i: usize, j: usize) {
    let (pi, pj) = (i as i32, j as i32);
    let floors = 3 + (hash01(pi + 1, pj) * 2.0).floor() as u32;
    // 南侧敞开，让哨卡院子朝街打开（也是玩家可突入的缺口）
    perimeter_ring(c, cx, cz, floors, pi, pj, Some(1));
    checkpoint(c, cx + 2.0, cz + 2.0);
    lamp_post(c, cx - 8.0, cz + 15.0);
    lamp_post(c, cx + 8.0, cz + 15.0);
    tree(c, cx - 14.0, cz - 2.0, pi + 9, pj);
}

/// 街区边界：四条路缘石带 + 四角树池。
///
/// **不再铺整块 45m×45m 的人行道抬台**——那和广场地台是同一个错误（一张浮在地面上的
/// 桌板），只是薄一点所以没那么刺眼。大面积铺装一律交给地面纹理（zone 3 人行道），
/// 几何只保留**又长又窄**的路缘石：45m×0.55m×0.16m 的条带读作"街沿"，
/// 而 45m×45m×0.14m 的板读作"漂浮桌面"。差别在长宽比，不在厚度。
fn block_edges(c: &mut City, cx: f32, cz: f32) {
    let s = KERB_EDGE;
    let k = 0.55;
    for (dx, dz, w, d) in [
        (0.0f32, -s * 0.5 + k * 0.5, s, k),
        (0.0, s * 0.5 - k * 0.5, s, k),
        (-s * 0.5 + k * 0.5, 0.0, k, s),
        (s * 0.5 - k * 0.5, 0.0, k, s),
    ] {
        c.deco(Part::new(ObstacleKind::Building, cx + dx, cz + dz, w, d, UNDER_GROUND, 0.16, CURB_STONE));
    }
    for kk in [-1.0f32, 1.0] {
        for side in 0..4i32 {
            let (tx, tz) = match side {
                0 => (cx + kk * 12.0, cz - s * 0.5 + 2.2),
                1 => (cx + kk * 12.0, cz + s * 0.5 - 2.2),
                2 => (cx - s * 0.5 + 2.2, cz + kk * 12.0),
                _ => (cx + s * 0.5 - 2.2, cz + kk * 12.0),
            };
            c.deco(Part::new(ObstacleKind::Tree, tx, tz, 1.7, 1.7, UNDER_GROUND, 1.30, TREE_LEAF_3).sph());
        }
    }
}

// ============================================================
// 街道与边界
// ============================================================

/// 街道设施：路灯、消防栓、中央隔离带、路口护柱。
fn street_furniture(c: &mut City) {
    for k in [-3i32, -2, -1, 1, 2, 3] {
        let t = k as f32 * STREET_EVERY;
        for side in [-7.0f32, 7.0] {
            lamp_post(c, side, t);
            lamp_post(c, t, side);
        }
    }
    for k in -2i32..=2 {
        for m in -2i32..=2 {
            if k == 0 && m == 0 {
                continue;
            }
            hydrant(c, k as f32 * STREET_EVERY + 8.0, m as f32 * STREET_EVERY + 8.0);
        }
    }
    // 中央隔离带：混凝土墩 + 反光柱（圆柱）
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
        let (w, d) = if x.abs() > 0.1 { (6.0, 0.9) } else { (0.9, 6.0) };
        c.push(Part::new(ObstacleKind::Building, x, z, w, d, UNDER_GROUND, 0.95, CONCRETE));
        // 出挑取 +0.30（半宽多 0.15 = RELIEF_STEP）；原来的 +0.25 只多出 0.125，
        // 掠射角下墩身肩面会与压顶落到同一批像素上。与围墙压顶同一套数值。
        c.deco(Part::new(ObstacleKind::Building, x, z, w + 0.30, d + 0.30, 0.95, 1.20, CONCRETE_DARK));
        for s in [-1.0f32, 0.0, 1.0] {
            let (px, pz) = if x.abs() > 0.1 { (x + s * 2.2, z) } else { (x, z + s * 2.2) };
            c.push(Part::new(ObstacleKind::Block, px, pz, 0.22, 0.22, 1.20, 1.90, FLAG_POLE).cyl());
        }
    }
    // 路口护柱
    for k in -2i32..=2 {
        for m in -2i32..=2 {
            let gx = k as f32 * STREET_EVERY;
            let gz = m as f32 * STREET_EVERY;
            for (dx, dz) in [(9.5f32, 9.5), (-9.5, 9.5), (9.5, -9.5), (-9.5, -9.5)] {
                c.push(Part::new(ObstacleKind::Block, gx + dx, gz + dz, 0.34, 0.34, UNDER_GROUND, 0.85, GRANITE).cyl());
            }
        }
    }
}

/// 边界围墙：墙身 + 压顶 + 立柱 + 4 座大门。
///
/// 旧版每段半宽 26m、段间距 55m → 段之间留 3m 豁口，玩家能直接走出地图。
/// 现在段宽 = 间距，彻底闭合；大门处只留 12m 缺口，两侧用半段补齐。
fn perimeter(c: &mut City) {
    let w = CITY_WALL;
    let seg = STREET_EVERY;
    let gate = 12.0f32;
    for side in 0..4i32 {
        for k in -3i32..=3 {
            let t = k as f32 * seg;
            let is_gate = t.abs() < 0.5;
            let pieces: &[(f32, f32)] = if is_gate {
                &[(t - (seg + gate) * 0.25, (seg - gate) * 0.5), (t + (seg + gate) * 0.25, (seg - gate) * 0.5)]
            } else {
                &[(t, seg * 0.5)]
            };
            for (tc, half_len) in pieces {
                let (x, z, pw, pd) = match side {
                    0 => (*tc, -w, *half_len * 2.0, 1.3),
                    1 => (*tc, w, *half_len * 2.0, 1.3),
                    2 => (-w, *tc, 1.3, *half_len * 2.0),
                    _ => (w, *tc, 1.3, *half_len * 2.0),
                };
                c.push(Part::new(ObstacleKind::Building, x, z, pw, pd, UNDER_GROUND, 2.35, CONCRETE));
                c.deco(Part::new(ObstacleKind::Block, x, z, pw + 0.3, pd + 0.3, 2.35, 2.55, CONCRETE_DARK));
                for s in [-1.0f32, 1.0] {
                    let (lx, lz) = if side < 2 { (x + s * half_len, z) } else { (x, z + s * half_len) };
                    c.push(Part::new(ObstacleKind::Building, lx, lz, 1.9, 1.9, UNDER_GROUND, 2.95, GRANITE));
                    // 压顶半宽必须比柱身多出 RELIEF_STEP（0.14）：0.95+0.14=1.09 → 边长 2.18，
                    // 原来写 2.15 只多出 0.125，掠射角下柱肩与压顶会落到同一批像素上。
                    c.deco(Part::new(ObstacleKind::Block, lx, lz, 2.20, 2.20, 2.95, 3.16, CONCRETE_DARK));
                }
            }
        }
    }
    // 4 座大门的门柱（不做门扇，留出可通行的口）
    for (gx, gz, along_x) in [(0.0f32, -w, true), (0.0, w, true), (-w, 0.0, false), (w, 0.0, false)] {
        for s in [-1.0f32, 1.0] {
            let (dx, dz) = if along_x { (s * 5.5, 0.0) } else { (0.0, s * 5.5) };
            c.push(Part::new(ObstacleKind::Building, gx + dx, gz + dz, 2.4, 2.4, UNDER_GROUND, 4.4, GRANITE));
            c.deco(Part::new(ObstacleKind::Block, gx + dx, gz + dz, 2.8, 2.8, 4.4, 4.75, CONCRETE_DARK));
            c.push(Part::new(ObstacleKind::Block, gx + dx, gz + dz, 0.9, 0.9, 4.75, 5.5, FLAG_POLE).cyl());
            c.deco(Part::new(ObstacleKind::Block, gx + dx, gz + dz, 0.42, 0.42, 5.5, 5.92, LAMP_GLOW).sph());
        }
    }
}

/// 手工城市地图（默认关卡；完整布局见模块头注释）
pub fn generate_city() -> LevelMap {
    let mut c = City::new();
    // 道具套件在生成入口装载一次，之后由各生成函数通过 `City::prop` 取用。
    // 读不到目录只意味着退回纯程序化外观，不是错误——所以这里只记一条 info。
    c.set = match PropSet::load_dir("assets/props") {
        Ok(s) => {
            log::info!("props: 载入 {} 件 GLB 道具网格", s.len());
            s
        }
        Err(e) => {
            log::info!("props: 未载入（{e}），城市回退纯程序化几何");
            PropSet::default()
        }
    };
    perimeter(&mut c);
    for i in 0..6 {
        for j in 0..6 {
            let cx = bc(i);
            let cz = bc(j);
            block_edges(&mut c, cx, cz);
            match block_role(i, j) {
                'P' => plaza(&mut c, cx, cz, i == 2 && j == 2),
                'O' => office_block(&mut c, cx, cz, i, j),
                'W' => warehouse(&mut c, cx, cz, i as i32, j as i32),
                'G' => park(&mut c, cx, cz),
                'S' => shop_block(&mut c, cx, cz, i, j),
                'M' => mixed_block(&mut c, cx, cz, i, j),
                _ => residential_block(&mut c, cx, cz, i, j),
            }
        }
    }
    street_furniture(&mut c);
    // 摆放分类统计（长期保留）：哪类资产命中了、哪类其实一个都没摆上，直接看这行就知道。
    // 之前排查"建筑仍是悬浮板"时只能靠猜，因为日志只给了总数。
    if !c.props.is_empty() {
        let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
        for pl in &c.props {
            if let Some(m) = c.set.get(pl.mesh) {
                *counts.entry(m.name.as_str()).or_insert(0) += 1;
            }
        }
        let mut pairs: Vec<(&&str, &usize)> = counts.iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(a.1));
        log::info!(
            "props: 摆放 {} 处 —— {}",
            c.props.len(),
            pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    } else {
        log::info!("props: 摆放 0 处（全部几何仍为程序化盒）");
    }
    c.finish()
}

/// 地面分区（供程序化地面纹理；与布局严格同源）：
/// 0=草地 1=沙土 2=沥青路 3=人行道 4=广场铺装 5=建筑地基
pub fn ground_zone(x: f32, z: f32) -> u8 {
    let fx = (x / STREET_EVERY).round();
    let fz = (z / STREET_EVERY).round();
    let dx = (x - fx * STREET_EVERY).abs();
    let dz = (z - fz * STREET_EVERY).abs();
    if dx <= ROAD_HALF || dz <= ROAD_HALF {
        return 2;
    }
    if dx <= ROAD_HALF + SIDEWALK || dz <= ROAD_HALF + SIDEWALK {
        return 3;
    }
    if x.abs() < 50.0 && z.abs() < 50.0 {
        return 4;
    }
    let i = ((x + 137.5) / STREET_EVERY).floor() as i32;
    let j = ((z + 137.5) / STREET_EVERY).floor() as i32;
    if (0..6).contains(&i) && (0..6).contains(&j) {
        if block_is_paved(block_role(i as usize, j as usize)) {
            return 5;
        }
        return 0;
    }
    let n = (x * 0.001 + z * 0.0023).sin() * (x * 0.0017 - z * 0.001).cos();
    if n > 0.3 {
        1
    } else {
        0
    }
}

// ============================================================
// 布局单元测试
// ============================================================

#[cfg(test)]
mod city_layout_tests {
    use super::*;

    /// 结构件 + 装饰件总预算：marker 槽上限 8192，超出会被 CPU 剔除按插入顺序
    /// 静默截断——越靠后装的街区整片消失，且没有任何日志。
    /// 实测参考：4034 个 marker 时 `cull_us=20`，容量不是性能瓶颈，这一档留给继续加密。
    #[test]
    fn total_geometry_fits_marker_budget() {
        let m = generate_city();
        let total = m.obstacles.len() + m.decor.len();
        assert!(
            total <= 8192,
            "城市几何 {} 件（结构 {} + 装饰 {}）超过 MAX_MARKER_INSTANCES=8192",
            total,
            m.obstacles.len(),
            m.decor.len()
        );
        assert!(
            m.obstacles.len() > 300,
            "结构件只有 {}，城市又是空心的",
            m.obstacles.len()
        );
        assert!(m.decor.len() > 500, "装饰件只有 {}，细化没有落地", m.decor.len());
    }

    /// 没有任何几何件是"大张纸片"： footprint 两个方向都超过 2m、**又悬在空中**的件，
    /// 厚度必须 ≥ 20cm。旧版 0.12m × 6.8m 的灯杆/窗带/贴皮就是这么混进场景的。
    /// 埋进地里的薄板不在此列——人行道抬台就是 14cm 厚的铺装，它的底面看不见，
    /// 不是"一张浮在空中的卡片"。
    #[test]
    fn no_paper_thin_geometry() {
        let m = generate_city();
        for (i, ob) in m.render_geometry().enumerate() {
            let thick = ob.half_h * 2.0;
            let wide = (ob.half_w * 2.0).min(ob.half_d * 2.0);
            let long = (ob.half_w * 2.0).max(ob.half_d * 2.0);
            let bottom = ob.y - ob.half_h;
            if thick >= MIN_AXIS || wide < 2.0 || long < 2.0 || bottom <= 0.0 {
                continue;
            }
            panic!(
                "#{} 悬空 {:.2}m、{}m×{}m 的薄板，厚度只有 {:.3}m：({:.1},{:.1})",
                i,
                bottom,
                wide,
                long,
                thick,
                ob.x,
                ob.z
            );
        }
    }

    /// 悬空的高层结构件下方必须有东西托到地面，否则楼底下有一条子弹能穿过去的缝。
    /// （旧版塔楼 `shaped(12.0, 15.0, ...)` 底面在 y=3，0..3m 整圈是空的。）
    #[test]
    fn solid_obstacles_reach_the_ground() {
        let m = generate_city();
        for ob in &m.obstacles {
            if ob.kind != ObstacleKind::Building || ob.half_h * 2.0 <= 3.0 {
                continue;
            }
            let bottom = ob.y - ob.half_h;
            if bottom <= 0.0 {
                continue;
            }
            // 沿竖直方向贪心向上爬：只要"覆盖该点且底面不高于当前到达高度"的件
            // 能一路接到该件底面，中间就没有可穿透的空隙（纪念碑是三级台阶叠上来的，
            // 单一一件不可能从地面直接顶到 1.02m，所以必须按链判断而不是按单件判断）。
            let covers = |s: &MapObstacle| {
                (s.x - ob.x).abs() <= s.half_w + ob.half_w * 0.5
                    && (s.z - ob.z).abs() <= s.half_d + ob.half_d * 0.5
            };
            let mut reach = 0.0f32;
            loop {
                let mut next = reach;
                for s in m.obstacles.iter().filter(|s| covers(s)) {
                    if (s.y - s.half_h) <= reach + 0.01 {
                        next = next.max(s.y + s.half_h);
                    }
                }
                if next <= reach + 0.01 || next >= bottom {
                    reach = next.max(reach);
                    break;
                }
                reach = next;
            }
            assert!(
                reach >= bottom - 0.01,
                "高层结构件底面在 y={:.2} 但下方只堆到 y={:.2}：({:.1},{:.1}) —— 中间有可穿透的空隙",
                bottom,
                reach,
                ob.x,
                ob.z
            );
        }
    }

    /// 落地件不得与地形平面共面：底面必须埋到地下。
    #[test]
    fn ground_touching_parts_are_buried() {
        let m = generate_city();
        for ob in m.render_geometry() {
            let bottom = ob.y - ob.half_h;
            if bottom > -0.04 && bottom < 0.02 {
                // 只有"贴着地面"的件需要埋进地下；一层窗台/平台腿等有意抬高的不算
                assert!(
                    bottom < 0.0,
                    "落地件底面 y={:.3} 与地形共面，掠射角会 Z-fight：({:.1},{:.1})",
                    bottom,
                    ob.x,
                    ob.z
                );
            }
        }
    }

    /// 装饰件与结构件不得有完全重合的盒（同一批像素上打架）。
    #[test]
    fn decor_never_coincides_with_structure() {
        let m = generate_city();
        for a in &m.decor {
            for b in &m.obstacles {
                let same = (a.x - b.x).abs() < 1e-3
                    && (a.z - b.z).abs() < 1e-3
                    && (a.y - b.y).abs() < 1e-3
                    && (a.half_w - b.half_w).abs() < 1e-3
                    && (a.half_h - b.half_h).abs() < 1e-3
                    && (a.half_d - b.half_d).abs() < 1e-3;
                assert!(!same, "装饰件与结构件完全重合：({:.1},{:.1})", a.x, a.z);
            }
        }
    }

    /// 围墙必须闭合：任意相邻段之间不得有 >0.5m 的豁口（大门除外，大门是有意留的）。
    #[test]
    fn perimeter_has_no_unintended_gaps() {
        let m = generate_city();
        let w = CITY_WALL;
        let mut spans: Vec<(f32, f32)> = m
            .obstacles
            .iter()
            .filter(|o| (o.z + w).abs() < 3.0 && o.half_w > 5.0)
            .map(|o| (o.x - o.half_w, o.x + o.half_w))
            .collect();
        assert!(!spans.is_empty(), "北墙一段都没有");
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut cover = spans[0].1;
        for s in &spans[1..] {
            let gap = s.0 - cover;
            // 只允许中央大门一处豁口，且不超过 12m
            assert!(
                gap <= 0.5 || (gap <= 12.5 && s.0.abs() < 8.0),
                "围墙出现 {:.2}m 意外豁口（x≈{:.1}）",
                gap.max(0.0),
                s.0
            );
            cover = cover.max(s.1);
        }
    }

    /// 出生点安全：原点 10m 半径内不得有任何结构件。
    /// （中央隔离墩有意放在 11m 处，是出生区外的第一道掩体，不算侵入。）
    #[test]
    fn spawn_area_is_clear() {
        let m = generate_city();
        for ob in &m.obstacles {
            let near_x = ob.x.abs() - ob.half_w;
            let near_z = ob.z.abs() - ob.half_d;
            let d = near_x.max(0.0).hypot(near_z.max(0.0));
            assert!(d > 10.0, "结构件侵入出生区：({:.1},{:.1}) 距原点 {:.1}m", ob.x, ob.z, d);
        }
    }

    /// 不许存在任何退化尺寸：任一轴向 ≤ 0 或过薄都会产生翻面/漏斗状垃圾几何。
    ///
    /// 直接动因：`rim()` 曾用 `d - 2*thick` 算侧边长度，仓库卷帘门框传 `d=0.40,
    /// thick=0.40` 得到 **-0.40**，负缩放的盒子翻面后被背面剔除掉近正面，
    /// 于是实机画面里广场正中浮着一块"四个尖角朝下的信封"。这种错误光看代码看不出来，
    /// 但一条断言就能永久钉死。
    #[test]
    fn no_degenerate_geometry() {
        let m = generate_city();
        for (i, o) in m.render_geometry().enumerate() {
            for (axis, v) in [("w", o.half_w * 2.0), ("d", o.half_d * 2.0), ("h", o.half_h * 2.0)] {
                assert!(
                    v > 0.05,
                    "#{} 的 {} 轴尺寸为 {:.3}m（退化/翻面风险）：@({:.1},{:.1}) y={:.2} {:?}",
                    i,
                    axis,
                    v,
                    o.x,
                    o.z,
                    o.y,
                    o.kind
                );
            }
        }
    }

    /// 悬空的几何件必须"挂在"别的件上：水平方向重叠，且底面落在对方竖直跨度内
    /// （被包住）或正落在对方顶面上（被托住）。
    ///
    /// 与 `solid_obstacles_reach_the_ground` 互补：那条查结构件堆叠链是否通到地面，
    /// 这条查薄板类件是否被放到既不在地上也不在楼上的尴尬高度。
    ///
    /// ⚠ 这条断言写错过两次，两次都是**断言本身错**、不是场景错，记下来别再改回去：
    ///   1. 第一版只允许"底面 ≈ 结构件顶面"（±0.35m），于是把**嵌在墙体内的窗带**
    ///      （底面 1.67m、核心跨 -0.05..28m）误判成悬空——窗带是被核心包住而非压顶。
    ///   2. 第二版只拿结构件当支撑，于是把**花坛第二层灌木**（底面 1.4m）判成悬空，
    ///      而它本来就是叠在第一层灌木上的。装饰件之间的堆叠是合法的，
    ///      所以支撑源必须取全部几何（结构 + 装饰），并排除自己。
    #[test]
    fn decor_is_either_buried_or_attached() {
        let m = generate_city();
        let all: Vec<&MapObstacle> = m.render_geometry().collect();
        for o in m.render_geometry() {
            let bottom = o.y - o.half_h;
            if bottom <= 0.0 || bottom >= 3.0 {
                continue;
            }
            let attached = all.iter().any(|s| {
                if std::ptr::eq(*s, o) {
                    return false;
                }
                let overlaps_xz = (s.x - o.x).abs() <= s.half_w + o.half_w
                    && (s.z - o.z).abs() <= s.half_d + o.half_d;
                let s_bottom = s.y - s.half_h;
                let s_top = s.y + s.half_h;
                let enclosed = bottom > s_bottom && bottom < s_top;
                let rests = (s_top - bottom).abs() < 0.35;
                overlaps_xz && (enclosed || rests)
            });
            assert!(
                attached,
                "几何件悬空在 y={:.2}：水平方向没有包住它或托住它的其它件 @({:.1},{:.1}) {:?}",
                bottom,
                o.x,
                o.z,
                o.kind
            );
        }
    }

    /// 不许出现"整块地台板"：水平跨度大、却只抬起一点点的大平板。
    ///
    /// 这条是 `no_paper_thin_geometry` 的**补漏**。前者豁免了"底面埋进地下"的件
    /// （豁免理由：人行道抬台本身就是地面，不该被当纸片删掉），结果我转头就在广场
    /// 铺了一张 34m×34m×0.29m 的板，测试全绿、实机截图里它是一张浮在地上的桌板。
    /// 判别标准改成**语义**而不是纯几何：地台类件必须是小步幅的真实台阶，
    /// 大面积铺装一律交给地面纹理，不许用几何凑。
    #[test]
    fn no_giant_field_plates() {
        let m = generate_city();
        for ob in m.render_geometry() {
            let a = ob.half_w * 2.0;
            let b = ob.half_d * 2.0;
            let exposed = (ob.y + ob.half_h).max(0.0);
            if a > 20.0 && b > 20.0 && exposed > 0.05 && exposed < 0.9 {
                panic!(
                    "{:.0}m×{:.0}m 的地台板只抬起 {:.2}m（= 一张浮在地上的薄桌板）：({:.1},{:.1})",
                    a, b, exposed, ob.x, ob.z
                );
            }
        }
    }

    /// 确定性：同一份代码两次生成必须逐位一致。
    #[test]
    fn generation_is_deterministic() {
        let a = generate_city();
        let b = generate_city();
        assert_eq!(a.obstacles, b.obstacles);
        assert_eq!(a.decor, b.decor);
    }

    /// 地面纹理分区必须与楼群同源：铺装的街区不能画成草地。
    #[test]
    fn ground_zone_agrees_with_block_roles() {
        for i in 0..6usize {
            for j in 0..6usize {
                let (x, z) = (bc(i), bc(j));
                let role = block_role(i, j);
                let zone = ground_zone(x, z);
                if role == 'P' {
                    // 中央 4 块广场落在 ground_zone 的"广场铺装"特例里（zone 4）
                    assert_eq!(zone, 4, "街区 ({},{}) role={} 应是广场铺装", i, j, role);
                } else if block_is_paved(role) {
                    assert_eq!(zone, 5, "街区 ({},{}) role={} 应是地基铺装", i, j, role);
                } else {
                    assert_eq!(zone, 0, "街区 ({},{}) role={} 应是草地", i, j, role);
                }
            }
        }
    }

    /// 形状标签必须真的用上了：城市里应同时存在立方、圆柱与细分球。
    /// 这条断言的意义是防止有人把 `geom()` 调用又删回盒子。
    /// 2026-09-01：树冠/沙袋从 `Ico` 全量改成了 `Sphere`（12 顶点二十面体平着色
    /// 远看像碎玻璃），所以这里统计的是 Sphere 而不是 Ico。
    #[test]
    fn city_uses_more_than_boxes() {
        let m = generate_city();
        let mut cyl = 0;
        let mut sph = 0;
        for ob in m.render_geometry() {
            match ob.shape {
                Shape::Cylinder => cyl += 1,
                Shape::Sphere => sph += 1,
                _ => {}
            }
        }
        assert!(cyl > 100, "圆柱件只有 {}：灯杆/树干/桶又退回盒子了", cyl);
        assert!(sph > 100, "细分球件只有 {}：树冠/沙袋又退回二十面体或盒子了", sph);
    }

    /// 相邻表面之间必须有可辨识的台阶，否则掠射角下两面落到同一批像素。
    ///
    /// 2026-09-03 起泛化为遍历全部实体：原来只抽查 bc(1) 一栋楼，而那条断言的前提是
    /// "这栋楼的进深由凸出的盒子提供"。GLB 路线上线后立面进深来自网格，抽查点必然失效，
    /// 逐条核对反而能继续守住剩下那批纯盒子几何（广场、仓库、据点、街具）。
    ///
    /// **只在竖直区间相接或重叠时才比较水平出挑**：共面风险来自两面在空间上真的贴在一起。
    /// 纪念碑的金属顶帽(11.2~11.62)比下面第二层台身(5.6~8.8)只多出 0.1m，但两者隔着
    /// 一整段碑身，永远不会互相遮挡——第一版泛化漏了这个条件，把它误报成了违规。
    #[test]
    fn relief_steps_are_not_coplanar() {
        let m = generate_city();
        let mut checked = 0usize;
        for core in m
            .obstacles
            .iter()
            .filter(|o| o.shape != Shape::None && o.kind == ObstacleKind::Building)
        {
            let c0 = core.y - core.half_h;
            let c1 = core.y + core.half_h;
            for b in m.decor.iter().filter(|o| {
                if (o.x - core.x).abs() >= 0.1 || (o.z - core.z).abs() >= 0.1 {
                    return false;
                }
                if o.half_w <= core.half_w {
                    return false;
                }
                let b0 = o.y - o.half_h;
                let b1 = o.y + o.half_h;
                // 相接（含 5cm 容差）或重叠
                b0 < c1 + 0.05 && b1 > c0 - 0.05
            }) {
                assert!(
                    b.half_w - core.half_w >= RELIEF_STEP - 1e-3,
                    "{:?} 处的立面构件只凸出核心 {:.3}m，小于最小台阶 {:.2}m",
                    (core.x, core.z),
                    b.half_w - core.half_w,
                    RELIEF_STEP
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "一个凸出核心的立面构件都没查到：装饰表可能被清空了");
    }

    /// GLB 路线的对偶不变式：盒子可以隐形，但**必须**有网格盖住它。
    /// 否则玩家会撞上一面什么都看不见的墙——比纸盒楼严重得多。
    #[test]
    fn invisible_cores_must_be_covered_by_a_prop() {
        let m = generate_city();
        let set = match crate::engine::props::PropSet::load_dir("assets/props") {
            Ok(s) => s,
            // 资产没生成时城市走的是纯盒子回退路径，本条不适用
            Err(_) => return,
        };
        let mut covered = 0usize;
        for core in m.obstacles.iter().filter(|o| o.shape == Shape::None) {
            let mut hit = false;
            for p in &m.props {
                let Some((hw, hd)) = p.rotated_footprint(&set) else { continue };
                if (p.x - core.x).abs() <= hw + 0.5 && (p.z - core.z).abs() <= hd + 0.5 {
                    hit = true;
                    break;
                }
            }
            assert!(
                hit,
                "({:.1}, {:.1}) 的碰撞核不可见，但没有任何 GLB 道具盖住它——这会是一面无形墙",
                core.x,
                core.z
            );
            covered += 1;
        }
        assert!(
            covered > 0,
            "没有任何隐形碰撞核：建筑 GLB 路线可能根本没生效，检查 generate_city 的资产装载"
        );
    }
}
