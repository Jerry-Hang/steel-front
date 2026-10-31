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
