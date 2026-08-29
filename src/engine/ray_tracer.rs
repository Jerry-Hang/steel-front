//! 全景路径追踪基准（2026-08-29 立项）：以 RT core 的真实光照作为「记录数据」与
//! 后续光照烘焙的参照；特定位置/室内场景启用完整路径追踪。
//!
//! 阶段状态：
//!  - 阶段1（扩展启用）✅
//!  - 阶段2（AS 构建）本文件：盒体场景 BLAS + TLAS（Vulkan 加速结构）
//!  - 阶段3（ray-query 计算通道）：着色器 = build.rs 手写 SPIR-V（naga 不支持 WGSL ray-query）
//!  - 阶段4（采集）：pt_ref.png

/// 阶段2 输入：场景盒体集合（AABB 中心/半宽；地面特殊大盒）
#[derive(Debug, Clone, Copy)]
pub struct PtBox {
    pub center: [f32; 3],
    pub half: [f32; 3],
    /// 材质：0=地面 1=混凝土 2=金属 3=树冠
    pub material: u32,
}

/// 盒体三角化：AABB → 24 顶点（12 三角），供 BLAS 三角形几何
pub fn box_triangles(b: &PtBox, out_verts: &mut [f32; 72]) {
    let (cx, cy, cz) = (b.center[0], b.center[1], b.center[2]);
    let (hx, hy, hz) = (b.half[0], b.half[1], b.half[2]);
    let mut i = 0;
    macro_rules! v {
        ($x:expr, $y:expr, $z:expr, $nx:expr, $ny:expr, $nz:expr, $u:expr, $vv:expr) => {{
            out_verts[i] = $x; out_verts[i + 1] = $y; out_verts[i + 2] = $z;
            out_verts[i + 3] = $nx; out_verts[i + 4] = $ny; out_verts[i + 5] = $nz;
            out_verts[i + 6] = $u; out_verts[i + 7] = $vv;
            i += 8;
        }};
    }
    // 6 面 × 4 顶点（平面法线）
    v!(cx - hx, cy - hy, cz - hz, -1.0, 0.0, 0.0, 0.0, 0.0);
    v!(cx - hx, cy - hy, cz + hz, -1.0, 0.0, 0.0, 0.0, 1.0);
    v!(cx - hx, cy + hy, cz + hz, -1.0, 0.0, 0.0, 1.0, 1.0);
    v!(cx - hx, cy + hy, cz - hz, -1.0, 0.0, 0.0, 1.0, 0.0);
    v!(cx + hx, cy - hy, cz + hz, 1.0, 0.0, 0.0, 0.0, 0.0);
    v!(cx + hx, cy - hy, cz - hz, 1.0, 0.0, 0.0, 0.0, 1.0);
    v!(cx + hx, cy + hy, cz - hz, 1.0, 0.0, 0.0, 1.0, 1.0);
    v!(cx + hx, cy + hy, cz + hz, 1.0, 0.0, 0.0, 1.0, 0.0);
    v!(cx - hx, cy - hy, cz + hz, 0.0, -1.0, 0.0, 0.0, 0.0);
    v!(cx + hx, cy - hy, cz + hz, 0.0, -1.0, 0.0, 0.0, 1.0);
    v!(cx + hx, cy - hy, cz - hz, 0.0, -1.0, 0.0, 1.0, 1.0);
    v!(cx - hx, cy - hy, cz - hz, 0.0, -1.0, 0.0, 1.0, 0.0);
    v!(cx - hx, cy + hy, cz + hz, 0.0, 1.0, 0.0, 0.0, 0.0);
    v!(cx + hx, cy + hy, cz + hz, 0.0, 1.0, 0.0, 0.0, 1.0);
    v!(cx + hx, cy + hy, cz - hz, 0.0, 1.0, 0.0, 1.0, 1.0);
    v!(cx - hx, cy + hy, cz - hz, 0.0, 1.0, 0.0, 1.0, 0.0);
    v!(cx - hx, cy - hy, cz - hz, 0.0, 0.0, -1.0, 0.0, 0.0);
    v!(cx + hx, cy - hy, cz - hz, 0.0, 0.0, -1.0, 0.0, 1.0);
    v!(cx + hx, cy + hy, cz - hz, 0.0, 0.0, -1.0, 1.0, 1.0);
    v!(cx - hx, cy + hy, cz - hz, 0.0, 0.0, -1.0, 1.0, 0.0);
    v!(cx - hx, cy - hy, cz + hz, 0.0, 0.0, 1.0, 0.0, 0.0);
    v!(cx + hx, cy - hy, cz + hz, 0.0, 0.0, 1.0, 0.0, 1.0);
    v!(cx + hx, cy + hy, cz + hz, 0.0, 0.0, 1.0, 1.0, 1.0);
    v!(cx - hx, cy + hy, cz + hz, 0.0, 0.0, 1.0, 1.0, 0.0);
}

/// 盒体三角形索引（12 三角 / 四边形对角化）
pub fn box_indices() -> [u32; 36] {
    let mut idx = [0u32; 36];
    for f in 0..6u32 {
        let b = f * 4;
        idx[(f * 6) as usize] = b;
        idx[(f * 6 + 1) as usize] = b + 1;
        idx[(f * 6 + 2) as usize] = b + 2;
        idx[(f * 6 + 3) as usize] = b;
        idx[(f * 6 + 4) as usize] = b + 2;
        idx[(f * 6 + 5) as usize] = b + 3;
    }
    idx
}

/// PT 基准参数（与游戏光照同语义，便于对比）
pub const PT_SUN_DIR: [f32; 3] = [-0.4, 0.9, -0.3];
pub const PT_SUN_COLOR: [f32; 3] = [1.0, 0.95, 0.85];
pub const PT_SUN_INTENSITY: f32 = 1.5;
pub const PT_AMBIENT_COLOR: [f32; 3] = [0.5, 0.55, 0.6];
pub const PT_AMBIENT_INTENSITY: f32 = 0.5;
