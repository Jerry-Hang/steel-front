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
pub fn box_triangles(b: &PtBox, out_verts: &mut [f32; 192]) {
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
    // 6 面 × 4 顶点 × 8 分量，必须刚好填满 out_verts：谁改了面数或顶点步长，
    // 这里会先炸，而不是把半截几何送进 BLAS。
    debug_assert_eq!(i, out_verts.len());
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
///
/// 2026-09-08：删掉 `PT_SUN_COLOR` / `PT_AMBIENT_COLOR` / `PT_AMBIENT_INTENSITY`。
/// 它们是 2026-09-01 光照配平**之前**的旧值（见 `game.rs` 那条配平注释），既没人读，
/// 又会诱导后来的人拿过期数字去"对齐"PT 与光栅化。PT 的 sun 颜色实际取
/// `Vec3::splat(1.0) * PT_SUN_INTENSITY`（`main.rs`），与光栅化的 directional 色不同源，
/// 这是 PT 恢复时要一并校准的点。
pub const PT_SUN_DIR: [f32; 3] = [-0.4, 0.9, -0.3];
pub const PT_SUN_INTENSITY: f32 = 1.5;

/// PT 取景参数（每帧由 main.rs 注入，打包为 5×vec4 push constants）
#[derive(Clone, Copy, Default)]
pub struct PtParams {
    pub cam: glam::Vec3,
    /// 相机前向（直接取 camera.forward()，与光栅化同源，不重推 yaw/pitch 公式）
    pub fwd: glam::Vec3,
    pub tan_half_fov: f32,
    pub bounces: u32,
    /// 表面→太阳（与 DirectionalLight::direction 同语义）
    pub sun_dir: glam::Vec3,
    pub sun_color: glam::Vec3,
    pub exposure: f32,
}

impl PtParams {
    /// 打包：必须与 assets/rt/pt_panorama.glsl 的 `PC { vec4 a,b,c,d,e,f,g }` 逐字段一致
    ///
    /// `box_tri_end` 现为**预留**参数（g.x）：pt3 实测 ray query 的图元索引是几何内
    /// 局部编号，分流改用 `rayQueryGetIntersectionGeometryIndexEXT`，g 保留给未来。
    pub fn pack(
        &self,
        w: u32,
        h: u32,
        frame: u32,
        reset: bool,
        spp_target: u32,
        move_amount: f32,
        box_tri_end: u32,
    ) -> [[f32; 4]; 7] {
        let tan = if self.tan_half_fov > 1e-4 {
            self.tan_half_fov
        } else {
            (60.0f32.to_radians() * 0.5).tan()
        };
        let s = self.sun_dir.normalize_or_zero();
        let f = if self.fwd.length_squared() > 1e-6 { self.fwd } else { glam::Vec3::NEG_Z };
        let exp = if self.exposure > 1e-4 { self.exposure } else { 0.4 };
        [
            [w as f32, h as f32, tan, self.bounces.clamp(1, 8) as f32],
            [self.cam.x, self.cam.y, self.cam.z, 0.0],
            [f.x, f.y, f.z, 0.0],
            [s.x, s.y, s.z, 0.0],
            [self.sun_color.x, self.sun_color.y, self.sun_color.z, exp],
            [
                frame as f32,
                if reset { 1.0 } else { 0.0 },
                spp_target.max(1) as f32,
                move_amount.clamp(0.0, 1.0),
            ],
            [box_tri_end as f32, 0.0, 0.0, 0.0],
        ]
    }

    /// 取景指纹：相机或光照变了才清累积重开，否则不同视角样本会混成拖影。
    /// 量化粒度必须**粗于站立时的呼吸/后坐抖动**——旧实现统一按 1mm 量化，
    /// 实机每帧位置都在变 => 每帧复位 => 累积永远停在 1 spp（画面始终满屏噪点）。
    pub fn signature(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        {
            let mut mix = |v: u64| {
                h ^= v;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            };
            // 位置 ~0.5m（盖住 bob/震屏），朝向 ~3°，光照/曝光基本静态
            for (i, c) in self.cam.to_array().iter().enumerate() {
                mix(((c * 2.0) as i64 as u64) << (i * 3));
            }
            for (i, c) in self.fwd.to_array().iter().enumerate() {
                mix(((c * 20.0) as i64 as u64) << (i * 3));
            }
            for c in self
                .sun_dir
                .to_array()
                .iter()
                .chain(self.sun_color.to_array().iter())
            {
                mix((*c * 100.0) as i64 as u64);
            }
            mix(self.view_signature_bits() as u64);
        }
        h
    }

    fn view_signature_bits(&self) -> i64 {
        ((self.tan_half_fov as f64 * 1e3) as i64) ^ ((self.exposure as f64 * 1e2) as i64)
            ^ (self.bounces as i64)
    }
}

/// 场景盒上限：BLAS 顶点/索引/材质缓冲按此容量一次性分配，换场景只重写内容 +
/// 重建 BLAS（TLAS 恒为单实例——所有盒合入同一 BLAS，故实例数与场景无关）。
///
/// 🔴 **2026-09-14：512 → 1024**（未结案 #10 的处置）。
///
/// 512 **不够**，而且不够得很隐蔽：实测 `marker=547 > 512`，即**每次都有 35 个盒子
/// 被 `renderer.rs` 的 `boxes.len().min(PT_MAX_BOXES)` 静默丢掉**。
/// PT 是低 spp 的噪点图，少几十个盒子肉眼看不出来 —— 这个"看不见的缺失"一直躺在那里。
///
/// **代价（按本文件与 `renderer.rs` 的分配公式实算）**：
/// `cap=512` 合计 ≈ **0.92 MB**（顶点 0.38 + 索引 0.07 + 材质 0.01 + BLAS ≈ 0.47）；
/// `cap=1024` 合计 ≈ **1.84 MB** ⇒ **多不到 1 MB**，且 87% 余量。
/// PT 默认关（`config.rs:40` 的 `pt_enable: false`）⇒ 对正常路径的内存**零影响**。
///
/// **为什么不用"CPU 侧按视锥裁剪"那条路**：它更省显存，但要求调用方改成
/// "每帧按当前视锥挑盒子"，而 PT 的**累积语义要求同一场景的盒子集合稳定**
/// （只有 `signature()` 变了才重建）—— 按视锥裁剪会让集合随视角抖动，
/// 反而**破坏累积**（这正是"PT 永不收敛"那一类问题的形态）。
/// **⇒ 在这个具体场景里，加容量不只是更省事，它是更正确的解。**
///
/// 🔴 **2026-09-19：1024 → 2048**。同一族坑第三次复发：街墙分段 + 水池/绕序
/// 修复后 marker 涨到 **1789 > 1024**，`pt_set_scene_markers` 的
/// `markers.take(PT_MAX_BOXES - 1)` 又在静默丢 765 个——而且 take 截断让
/// `build_pt_as` 里那次性告警**根本不会触发**（传进去的已经 ≤ 容量）。
/// 代价按 09-14 的公式同比翻倍：≈ **3.7 MB**，PT 默认关 ⇒ 正常路径零影响。
/// 告警闩保留，但截断点挪到 take 之前先比对（见 renderer.rs）。
pub const PT_MAX_BOXES: usize = 2048;

/// 路径追踪 GPU 资源集（构建/记录/销毁）
pub struct PtAssets {
    pub tlas: ash::vk::AccelerationStructureKHR,
    pub blas: ash::vk::AccelerationStructureKHR,
    pub tlas_buf: ash::vk::Buffer,
    pub tlas_mem: ash::vk::DeviceMemory,
    pub blas_buf: ash::vk::Buffer,
    pub blas_mem: ash::vk::DeviceMemory,
    pub verts_buf: ash::vk::Buffer,
    pub verts_mem: ash::vk::DeviceMemory,
    pub idx_buf: ash::vk::Buffer,
    pub idx_mem: ash::vk::DeviceMemory,
    pub inst_buf: ash::vk::Buffer,
    pub inst_mem: ash::vk::DeviceMemory,
    /// 每盒材质 UBO（vec4 = albedo.rgb + 光泽度），容量 PT_MAX_BOXES
    pub mat_buf: ash::vk::Buffer,
    pub mat_mem: ash::vk::DeviceMemory,
    /// AS 构建 scratch（自有且常驻，取代旧实现每次 record 新建 2MB 不释放的泄漏）
    pub scratch_buf: ash::vk::Buffer,
    pub scratch_mem: ash::vk::DeviceMemory,
    /// BLAS scratch 字节数（TLAS 用后半段，避免两次构建共享同一地址的资源冲突）
    pub scratch_blas: u64,
    /// BLAS 创建时并入的道具三角形数（0 = 无道具几何）。`record_pt_build` 与
    /// 描述符重写都按这个值走——道具缓冲句柄/三角形数变了必须整体重建 BLAS。
    pub prop_tris: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// pack 的第 7 槽 g.x = 盒体三角形边界（道具路径的分流判据）。
    /// 2026-09-19 道具进 BLAS 专项：着色器与 Rust 侧的 PC 布局靠这条测试钉死。
    #[test]
    fn pack_appends_box_tri_end_as_seventh_vec4() {
        let p = PtParams {
            cam: glam::Vec3::ZERO,
            fwd: glam::Vec3::NEG_Z,
            tan_half_fov: 0.5,
            bounces: 6,
            sun_dir: glam::Vec3::Y,
            sun_color: glam::Vec3::ONE,
            exposure: 0.2,
        };
        let pc = p.pack(2560, 1600, 7, true, 256, 0.5, 13_200);
        assert_eq!(pc.len(), 7, "PC 必须是 7×vec4 = 112B");
        assert_eq!(pc[6][0], 13_200.0, "g.x 必须是 boxTriEnd");
        assert_eq!(pc[0][0], 2560.0);
        assert_eq!(pc[5][1], 1.0, "reset 标志仍在 f.y");
    }
}
