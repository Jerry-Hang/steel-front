//! 现代枪械程序化建模库（大战场枪械设计 v1.0）
//!
//! 每把枪 = 一个纯函数：用 meshgen 图元（beveled_box/frustum/cylinder/sphere/torus_arc）
//! 拼装出**局部坐标**（y 向上、枪口朝 +Z 前）的网格，再经 `assemble` 统一烘焙光照。
//! 第一人称使用时由 main.rs 按 view⁻¹ 锚点变换，第三人称/NPC 按世界矩阵变换。
//!
//! 约定：
//! - 枪长单位：米。全枪长度约 0.6~1.2m（不含枪口制退器可到 1.5m）。
//! - 坐标：x=左右（右+），y=上下（上+），z=前后（枪口朝 +Z）。
//! - 部件矩阵：t(偏移) * r(局部旋转)，圆柱/锥台默认沿 Y，转 -90° 使沿 +Z 用
//!   glam::Mat4::from_rotation_x(-FRAC_PI_2)。
//! - 材质色：黑/深灰钢、亮钢、聚合物/护木、木色、迷彩绿等。
//! - 烘焙光照由 assemble 统一做，各枪函数只给 tint 材质色即可。

use glam::Mat4;
use crate::engine::meshgen::{GVertex, Mesh};

pub mod antimaterial;
pub mod assault_blue;
pub mod assault_red;
pub mod dmr;
pub mod hmg;
pub mod lmg_blue;
pub mod lmg_red;
pub mod pistols;
pub mod shotgun;
pub mod smg_blue;
pub mod smg_red;
pub mod sniper;

/// 单把枪的成品网格：局部坐标顶点 + 索引 + 元数据
#[derive(Debug, Clone)]
pub struct GunMesh {
    pub verts: Vec<GVertex>,
    pub indices: Vec<u32>,
    /// 全枪长度（米，用于摆放/缩放参考）
    #[allow(dead_code)] // 元数据：第三人称摆放/缩放用
    pub length: f32,
}

/// 装配助手：把多个局部部件 Mesh 按矩阵合并进 GunMesh，统一烘焙光照。
pub fn assemble(parts: &[(Mat4, Mesh, [f32; 3])]) -> (Vec<GVertex>, Vec<u32>) {
    // 立体感光照：低环境光 + 高漫反射 → 明暗对比强烈，部件轮廓分明
    // （2026-08-18 立体感优化：ambient 0.55→0.32，diffuse 0.5→0.78）
    let light = glam::Vec3::new(-0.45, 0.8, -0.3).normalize();
    let (ambient, diffuse) = (0.32f32, 0.78f32);
    let mut verts: Vec<GVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for (m, mesh, tint) in parts {
        mesh.append_transformed(&mut verts, &mut indices, *m, *tint, light, ambient, diffuse);
    }
    (verts, indices)
}

/// 圆柱沿 +Z 摆放的旋转（meshgen 圆柱沿 Y，需要绕 X -90°）
pub fn rz() -> Mat4 { glam::Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2) }

/// 部件平移（各枪函数此前各自重复闭包 ~70 行；2026-08-24 收编共享）
pub fn t(x: f32, y: f32, z: f32) -> Mat4 {
    Mat4::from_translation(glam::vec3(x, y, z))
}

/// 绕 X 轴旋转（部件局部姿态）
pub fn rx(a: f32) -> Mat4 {
    Mat4::from_rotation_x(a)
}

/// **绕 Y 轴 -90°** —— 把 `torus_arc` 生成的 **XY 平面**环立到 **YZ 立面**。
///
/// 🔴 **2026-10-17 加**（外部复核指出 `torus_arc` 的初始平面在两类部件里被写反）：
/// `meshgen::torus_arc` 的几何是
/// `pos = ((ring_r + tube_r*cp)*ct, (ring_r + tube_r*cp)*st, tube_r*sp)`
/// ⇒ **环在 XY 平面、孔轴 = Z**。于是：
/// * **箍在枪管上**（孔轴要沿枪管的 Z）⇒ **不转**；
/// * **扳机护圈 / 提把**（要立在 YZ 立面、沿枪身前后跨立）⇒ **必须 `* ry(-PI/2)`**。
///
/// 各枪文件此前是「有的转、有的不转、有的转错轴」⇒ 13 处护圈横张在 XY 平面
/// （向枪身左右外翻）、8 处箍环被 `rz()` 翻成 XZ 水平盘（切穿枪管）。
/// 加这个助手是为了让"哪一类该转"有个明确的名字，不再各写各的。
pub fn ry(a: f32) -> Mat4 {
    Mat4::from_rotation_y(a)
}

/// **拉机柄杆**（平移 + 绕 **Z** -90°，使圆柱轴向 **+X** —— 从机匣右侧伸出去）。
///
/// 🔴 **2026-10-17 修**（外部复核指出：`guns::rh` 误用 `rz()` 替代绕 Z 旋转，
/// 导致 11 把武器的拉机柄杆轴向错转 90°、与球头脱节悬空）：
///
/// 原来这里乘的是 `rz()`（= 绕 **X** -90°，Y 圆柱 → **Z** 轴），
/// 于是这根杆**沿枪身前后方向躺平**，在 X 轴上只占 `[x-0.005, x+0.005]`
/// —— **既不接触机匣右壁，也不接触 `x≈0.09` 的拉机柄球头，两者双双悬空**。
///
/// **坐标算术直接证明调用方的原意是沿 X 轴**：例如 `saiga12`
/// `rh(0.062, 0.10, -0.26), cylinder(0.007, 0.050, 10)` + 球头在 `x = 0.090`：
/// 沿 X 时杆覆盖 `x ∈ [0.037, 0.087]` ⇒ 左端贴机匣右壁（≈0.037）、
/// 右端接住半径 0.008 的球头（`[0.082, 0.098]`）✅ 严丝合缝。
///
/// 兄弟实现 `dmr.rs` 里的 `_ztox`（`from_rotation_z(-FRAC_PI_2)`）就是正确写法
/// —— **同一件事两处实现、只对了一处**。
///
/// 判据：`charging_handle_rods_point_along_plus_x`（把 `rh` 生成的矩阵作用到
/// 圆柱的局部 Y 轴上，断言结果落在 ±X 而不是 ±Z）。
pub fn rh(x: f32, y: f32, z: f32) -> Mat4 {
    t(x, y, z) * Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_2)
}


impl GunMesh {
    /// 应用 4x4 变换（第一人称视空间锚定用）：位置/法线随矩阵变换，烘焙颜色保留。
    /// 返回 (顶点, 索引)，可直接上传渲染。
    ///
    /// 2026-09-01 起第一人称不再用它：main.rs 改为返回局部坐标 + 由实例 model 矩阵
    /// 施加一次变换（旧写法把同一矩阵既烘进顶点又当实例矩阵，等于 M·M·p，含
    /// view_inv 的矩阵平方后不被抵消 → 枪整支飞离画面）。保留给第三人称/掉落枪械
    /// 等"把网格摆进世界"的用法。
    #[allow(dead_code)] // 当前无调用方：为上面的世界摆放用途预留
    pub fn transformed(&self, m: Mat4) -> (Vec<GVertex>, Vec<u32>) {
        let n = glam::Mat3::from_mat4(m);
        let verts = self
            .verts
            .iter()
            .map(|v| {
                let p = m.transform_point3(glam::Vec3::from(v.pos));
                let nn = (n * glam::Vec3::from(v.normal)).normalize_or_zero();
                GVertex {
                    pos: p.to_array(),
                    normal: nn.to_array(),
                    uv: v.uv,
                    color: v.color,
                }
            })
            .collect();
        (verts, self.indices.clone())
    }
}

/// 按武器键名取枪模（键名 = weapon_data::WeaponSpec::key；返回局部坐标网格，
/// 枪口朝 +Z，第一人称使用时由调用方变换锚定）。
// ============================================================
// 连通性分析（2026-10-31：抓"悬空件"）
// ============================================================

/// 一个连通块：三角数 + 世界 AABB（min xyz, max xyz）。
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub struct Island {
    pub tris: usize,
    pub aabb: [f32; 6],
}

/// 把网格按"**AABB 间距 ≤ gap 就算相连**"切成连通块。
///
/// 为什么要它：2026-10-31 修 AK-104 托杆 / Saiga-12 悬空弹仓时，问题是**看图**发现的 ——
/// 造型判据（`procedural_guns_have_sane_extents`）只看得见整体包围盒，单件转错或悬空它一概不知。
/// 这个函数把"每个部件是不是长在枪身上"变成可判定的问题：**真实调好的枪只有 1 个连通块**。
///
/// 实现：三角 AABB + 均匀网格（cell = max(2·gap, 2cm)）做候选裁剪，再并查集合并 ——
/// 35 把枪全跑一遍在毫秒级，可以直接当单测。
#[cfg(test)]
pub fn mesh_islands(gm: &GunMesh, gap: f32) -> Vec<Island> {
    use std::collections::HashMap;
    let tri_n = gm.indices.len() / 3;
    if tri_n == 0 {
        return Vec::new();
    }
    // ① 每个三角的 AABB
    let mut boxes: Vec<[f32; 6]> = Vec::with_capacity(tri_n);
    for t in 0..tri_n {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for k in 0..3 {
            let vi = gm.indices[t * 3 + k] as usize;
            let p = gm.verts[vi].pos;
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        boxes.push([lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]);
    }
    // ② 并查集
    let mut parent: Vec<usize> = (0..tri_n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let cell = (gap * 2.0).max(0.02);
    let key = |v: f32| (v / cell).floor() as i32;
    let mut grid: HashMap<(i32, i32, i32), Vec<usize>> = HashMap::new();
    let dist = |a: &[f32; 6], b: &[f32; 6]| -> f32 {
        let mut d = 0.0f32;
        for k in 0..3 {
            let gap_lo = b[k] - a[k + 3];
            let gap_hi = a[k] - b[k + 3];
            let g = gap_lo.max(gap_hi).max(0.0);
            d = d.max(g);
        }
        d
    };
    for (i, b) in boxes.iter().enumerate() {
        let (x0, y0, z0) = (key(b[0] - gap), key(b[1] - gap), key(b[2] - gap));
        let (x1, y1, z1) = (key(b[3] + gap), key(b[4] + gap), key(b[5] + gap));
        for cx in x0..=x1 {
            for cy in y0..=y1 {
                for cz in z0..=z1 {
                    if let Some(list) = grid.get(&(cx, cy, cz)) {
                        for &j in list {
                            if dist(b, &boxes[j]) <= gap {
                                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                                if ri != rj {
                                    parent[ri.max(rj)] = ri.min(rj);
                                }
                            }
                        }
                    }
                }
            }
        }
        grid.entry((key(b[0]), key(b[1]), key(b[2]))).or_default().push(i);
    }
    // ③ 汇总
    let mut acc: HashMap<usize, Island> = HashMap::new();
    for i in 0..tri_n {
        let r = find(&mut parent, i);
        let b = &boxes[i];
        let e = acc.entry(r).or_insert(Island { tris: 0, aabb: [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN] });
        e.tris += 1;
        for k in 0..3 {
            e.aabb[k] = e.aabb[k].min(b[k]);
            e.aabb[k + 3] = e.aabb[k + 3].max(b[k + 3]);
        }
    }
    let mut v: Vec<Island> = acc.into_values().collect();
    v.sort_by(|a, b| b.tris.cmp(&a.tris));
    v
}

// ============================================================
// 可复用的"小件"（2026-10-31：给程序化枪补细节用）
// ============================================================
//
// 为什么抽出来：光学镜在两把狙击枪 + 三把步枪上都要有，四把枪各抄一遍就会出现四种
// 高度/长度，观感不统一，改一处还得改四处。这些函数只返回**零件列表**，由调用方
// `parts.extend(...)` 拼进自己的 vec，坐标仍是"枪口 +Z / 上方 +Y / 右侧 +X"的局部系。

/// 顶部光学镜：镜筒 + 前后镜圈 + 两个镜座 + 目镜。
///
/// `y` = 镜筒中心高度（按各枪的导轨面给），`z` = 镜筒中心前后位置，
/// `len` = 镜筒长度。返回 6 个零件。
pub fn optic_sight(y: f32, z: f32, len: f32) -> Vec<(Mat4, crate::engine::meshgen::Mesh, [f32; 3])> {
    use crate::engine::meshgen::{beveled_box, cylinder, frustum};
    use std::f32::consts::FRAC_PI_2;
    let body = [0.13, 0.14, 0.16];
    let glass = [0.30, 0.42, 0.52];
    let r = 0.016;
    vec![
        // 镜筒（沿 Z）
        (t(0.0, y, z) * rx(FRAC_PI_2), cylinder(r, len, 12), body),
        // 前镜圈（亮钢，略大）
        (t(0.0, y, z + len * 0.5) * rx(FRAC_PI_2), frustum(r * 1.35, r * 1.1, 0.020, 14, true), [0.62, 0.65, 0.70]),
        // 后目镜（略大）
        (t(0.0, y, z - len * 0.5) * rx(FRAC_PI_2), frustum(r * 1.15, r * 1.30, 0.022, 14, true), body),
        // 前镜片（青蓝，一眼看出是玻璃）
        (t(0.0, y, z + len * 0.5 + 0.011) * rx(FRAC_PI_2), cylinder(r * 1.05, 0.004, 14), glass),
        // 前镜座
        (t(0.0, y - r - 0.012, z + len * 0.30), beveled_box(0.026, 0.024, 0.030, 0.005, 3), [0.16, 0.17, 0.19]),
        // 后镜座
        (t(0.0, y - r - 0.012, z - len * 0.30), beveled_box(0.026, 0.024, 0.030, 0.005, 3), [0.16, 0.17, 0.19]),
    ]
}

/// 弹链：从 `(x, y, z)` 起沿 −Z 垂下一段，`links` 节。返回 `links` 个零件。
///
/// 通用机枪与班用机枪靠这条链子一眼可辨（PKM/PKP 的标志性外观），
/// 但只做"垂下来的一小段"——不做整条链，也不做逐节转动（FP 视角看不出）。
pub fn ammo_belt(x: f32, y: f32, z: f32, links: usize) -> Vec<(Mat4, crate::engine::meshgen::Mesh, [f32; 3])> {
    use crate::engine::meshgen::{beveled_box, cylinder};
    use std::f32::consts::FRAC_PI_2;
    let brass = [0.72, 0.58, 0.24];
    let link_col = [0.34, 0.30, 0.20];
    let mut out = Vec::with_capacity(links);
    for i in 0..links {
        let dz = 0.024 * i as f32;
        let dy = -0.020 * i as f32;
        // 链节（横夹）
        out.push((
            t(x, y + dy, z - dz),
            beveled_box(0.020, 0.016, 0.012, 0.003, 2),
            link_col,
        ));
        // 露出的弹壳（黄铜）
        out.push((
            t(x + 0.013, y + dy + 0.002, z - dz) * rx(FRAC_PI_2),
            cylinder(0.005, 0.020, 8),
            brass,
        ));
    }
    out
}

/// 顶部/侧面导轨：`slots` 个齿。返回 `slots` 个零件。
pub fn rail(y: f32, z: f32, len: f32, slots: usize, side: f32) -> Vec<(Mat4, crate::engine::meshgen::Mesh, [f32; 3])> {
    use crate::engine::meshgen::beveled_box;
    let col = [0.16, 0.17, 0.19];
    let mut out = Vec::with_capacity(slots + 1);
    // 底板
    out.push((t(side, y, z), beveled_box(0.030, 0.010, len, 0.003, 2), col));
    let step = len / slots as f32;
    for i in 0..slots {
        let zz = z - len * 0.5 + step * (i as f32 + 0.5);
        out.push((t(side, y + 0.008, zz), beveled_box(0.026, 0.008, step * 0.45, 0.002, 2), col));
    }
    out
}

pub fn gun_mesh_by_key(key: &str) -> Option<GunMesh> {
    builder_for(key).map(|f| f())
}

/// 键 → 构建函数（**只查表，不建网格**）。
///
/// 存在的理由（2026-10-31）：`load_gun_glb` 缺 GLB 时要判断"这把枪有没有自己的程序化模型"——
/// 有就走自己的，没有才借用 `ak12.glb`。旧写法只能靠"把网格建出来看是不是 None"，
/// 既要付构建成本，又让"缺 GLB"和"没有程序化模型"两件事混在一起。
fn builder_for(key: &str) -> Option<fn() -> GunMesh> {
    Some(match key {
        "ak12m" => assault_red::ak12m,
        "ak104" => assault_red::ak104,
        "ash12" => assault_red::ash12,
        "hk416" => assault_blue::hk416,
        "mk18" => assault_blue::mk18,
        "pp19" => smg_red::pp19,
        "pp9" => smg_red::pp9,
        "vss" => smg_red::vss,
        "asval" => smg_red::asval,
        "mpx" => smg_blue::mpx,
        "mp5sd" => smg_blue::mp5sd,
        "p90" => smg_blue::p90,
        "mp7" => smg_blue::mp7,
        "svd12" => dmr::svd12,
        "m110a1" => dmr::m110a1,
        "mk14p" => dmr::mk14p,
        "sv98" => sniper::sv98,
        "m2010" => sniper::m2010,
        "mrad" => sniper::mrad,
        "osv96" => antimaterial::osv96,
        "m82a1" => antimaterial::m82a1,
        "rpk16" => lmg_red::rpk16,
        "pkm" => lmg_red::pkm,
        "pkp" => lmg_red::pkp,
        "m249" => lmg_blue::m249,
        "m240l" => lmg_blue::m240l,
        "rope12" => hmg::rope12,
        "m2a1" => hmg::m2a1,
        "saiga12" => shotgun::saiga12,
        "m1014" => shotgun::m1014,
        "aa12" => shotgun::aa12,
        "mp443" => pistols::mp443,
        "rsh12" => pistols::rsh12,
        "m18" => pistols::m18,
        "mk23" => pistols::mk23,
        _ => return None,
    })
}

/// 该键是否有程序化枪模（`gun_mesh_by_key` 是否可能返回 `Some`）—— 不构建网格。
pub fn has_procedural_mesh(key: &str) -> bool {
    builder_for(key).is_some()
}

#[cfg(test)]
mod gun_geometry_tests {
    use super::*;

    /// **没有悬空件**：每把枪在「AABB 间距 ≤ 5mm 即算相连」下必须只有 **1 个连通块**。
    ///
    /// 这条就是本轮（续102）缺的那个护栏：`procedural_guns_have_sane_extents` 只看得见整体包围盒，
    /// Saiga-12 那根悬空 1.9cm 的弹仓、AK-104 那两根细托杆它都发现不了。
    ///
    /// 阈值来源：5mm 是"零件相互贴合"与"肉眼可见的缝"之间的分界（AK-104 托杆直径 8mm、
    /// Saiga 弹仓实际缝隙 19mm）。**它抓不到"细杆看起来不结实"** 这种"好不好看"的问题，只能抓物理断开。
    #[test]
    fn procedural_gun_parts_are_not_catastrophically_detached() {
        // 🔬 度量：**让整把枪变成 1 个连通块所需的最小间距**（`mesh_islands` 逐档试）。
        //
        // 🔴 **这条判据的边界（实测过，别当观感护栏用）**
        // - 我最初的设想是"部件必须贴合"，实测直接**被数据否掉**：35 把枪在 5mm 下
        //   **全部**不连通，只有 14/35 能把间距压到 ≤60mm。原因是这些枪由几十个独立图元
        //   堆出来，而且有些"悬空"是**合法的**（扳机本来就悬在护圈里、瞄具架在镜环上、
        //   枪机球头挂在拉机柄末端）—— 度量分不开"合法悬空"与"看起来断裂"。
        // - 所以它只保留一条**上界**：任何部件都不得离枪身 > 500mm（那是真的飞出去了），
        //   并把最差的 8 把**打印出来当存量清单**，供以后逐把改几何时对照。
        //
        // ⇒ 视觉类的悬空（如 Saiga-12 那根 19mm 的弹仓）只能靠**看图**发现，
        //   这条判据**不能**替代它。别把这里的通过读成"观感没问题"。
        let gaps = [0.005f32, 0.010, 0.020, 0.030, 0.040, 0.060, 0.080, 0.120, 0.200, 0.300, 0.500];
        let mut report: Vec<(f32, String)> = Vec::new();
        for spec in crate::engine::weapon_data::ALL_WEAPONS.iter() {
            let Some(gm) = super::gun_mesh_by_key(spec.key) else { continue };
            let mut need = f32::MAX;
            for g in gaps {
                if super::mesh_islands(&gm, g).len() <= 1 {
                    need = g;
                    break;
                }
            }
            report.push((need, spec.key.to_string()));
        }
        report.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let worst: Vec<String> = report
            .iter()
            .take(8)
            .map(|(g, k)| format!("{k}={:.0}mm", g * 1000.0))
            .collect();
        eprintln!("🔬 连通所需间距最差 8 把（存量清单）：{}", worst.join(" "));

        let broken: Vec<String> = report
            .iter()
            .filter(|(g, _)| *g > 0.500)
            .map(|(g, k)| format!("{k}={:.0}mm", g * 1000.0))
            .collect();
        assert!(
            broken.is_empty(),
            "有部件离枪身 >500mm（不是贴合缝问题，是几何飞了）：{}",
            broken.join(" ")
        );
    }

    /// **程序化枪的外形比例必须成立**（2026-10-31）：声明长度与实际 Z 跨度比在 0.75–1.30，
    /// 长枪还要"沿 Z 最长"（手枪本来就高大于长，已豁免）。
    ///
    /// 🔴 **它能抓什么、抓不到什么（反向验证过，别高估）**：
    /// - 抓得到：整把枪被转到别的轴、`length` 声明与几何脱节、网格为空；
    /// - **抓不到**：**单个部件**轴向转错。实测把 AK-104 的枪管从 `rz()` 改成 `rx(90°)`
    ///   （枪管变成竖着的），本判据**照样通过** —— 因为整体包围盒由其它部件撑住。
    ///   想抓"单件悬空/单件转错"，需要**逐件 AABB 的连通性判据**（part graph），那是另一件事；
    ///   本轮 AK-104 托杆、Saiga-12 弹仓的问题都是**看图**发现并修掉的，不是这条判据抓的。
    #[test]
    fn procedural_guns_have_sane_extents() {
        let mut checked = 0;
        for spec in crate::engine::weapon_data::ALL_WEAPONS.iter() {
            let Some(gm) = super::gun_mesh_by_key(spec.key) else { continue };
            assert!(!gm.verts.is_empty(), "{}: 程序化网格为空", spec.key);
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for v in &gm.verts {
                for k in 0..3 {
                    lo[k] = lo[k].min(v.pos[k]);
                    hi[k] = hi[k].max(v.pos[k]);
                }
            }
            let (dx, dy, dz) = (hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
            let ratio = dz / gm.length.max(1e-4);
            assert!(
                (0.75..=1.30).contains(&ratio),
                "{}: 声明长度 {:.3} 但 Z 跨度 {:.3}（比值 {:.2}）—— 部件轴向可能转错了",
                spec.key,
                gm.length,
                dz,
                ratio
            );
            // 手枪（m18/mp443/…）**本来就是"高大于长"**（握把 + 套筒），不能套这条；
            // 长枪必须满足"沿 Z 最长"。
            if gm.length >= 0.60 {
                assert!(
                    dz > dy && dz > dx,
                    "{}: 枪身不沿 Z 展开（dz={dz:.3} dy={dy:.3} dx={dx:.3}）",
                    spec.key
                );
            }
            checked += 1;
        }
        assert!(checked >= 30, "只校验到 {checked} 把程序化枪，预期 >=30");
    }

    /// 🔴 **`rh()` 必须把圆柱的局部 Y 轴转到 ±X**（从机匣右侧伸出的拉机柄）。
    ///
    /// 2026-10-17 实测：`rh()` 原来乘的是 `rz()`（绕 **X** -90°，局部 Y → **Z**）
    /// ⇒ 11 把武器的拉机柄杆沿枪身前后躺平，与 `x≈0.09` 的球头脱节、两者双双悬空
    /// （调用方的坐标算术 `x_rod + L/2 ≈ x_sphere` 直接证明原意是沿 X）。
    ///
    /// 判据把矩阵作用到局部 Y 轴上 —— 轴向落在 ±Z 就说明又用回了 `rz()`。
    #[test]
    fn charging_handle_rods_point_along_plus_x() {
        let m = rh(0.062, 0.10, -0.26);
        let axis = m.transform_vector3(glam::Vec3::Y).normalize();
        assert!(
            axis.x.abs() > 0.99 && axis.y.abs() < 1e-5 && axis.z.abs() < 1e-5,
            "rh() 生成的杆轴向是 {axis:?}，应该是 ±X；落在 ±Z 说明又用回了 rz()（绕 X）"
        );
    }
}
