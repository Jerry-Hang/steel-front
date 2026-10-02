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

/// PT 曝光的**标定值**：光栅把反照率乘在 tone 之外（`alb×(1-exp(-1.55L))`），
/// PT 物理正确在之内；该值使两模型在 albedo 0.1~0.8 区间分区均值互差 ≤15%（2026-09-19 §19）。
///
/// 🔴 **2026-10-02：0.4 → 0.1，与 `assets/rt/pt_panorama.glsl` 的增益修复严格成对。**
/// 那条修复把"每帧 SPP 个样本之和"改为"每帧均值"后再进时域 EMA，消掉了稳态显示增益
/// = SPP/win 的缺陷（静止 16/64=0.25、运动 64/1=64 ⇒ 同一像素在走与站之间摆 **256 倍**）。
/// 代价是**静止**稳态亮度恰好提高 4 倍（0.25 → 1.0），所以标定值同步 ÷4，
/// 把 §19 标定所依据的静态参照帧亮度**原样保持**。⇒ 这两个数是一个事实的两半，
/// **改一个必须改另一个**：只改着色器 ⇒ PT 整体亮 4 倍；只改这里 ⇒ 静止 PT 暗 4 倍。
/// 复测判据：改前/改后静态参照帧均值灰差 <3%；静止 vs 行走 <10%（旧值 256×）。
/// 已知遗留：§19 的分区偏差表是在旧的非均匀增益下测的，静态端数值不变，
/// 但当时"行走中的 PT"未被该表覆盖——那正是本条修掉的部分，表可择机复测。
///
/// ⚠️ 它是**标定常数**而不是玩家选项：它绑死在光栅那条 tone 曲线上，乱动就等于把 PT 与光栅
/// 的对照关系破坏掉。所以它进 `config.rs`（可持久化、可被 `RV3D_PT_EXPOSURE` 覆盖做 A/B），
/// **不进设置面板**（面板里放一个玩家随手可改的标定值，只会造出一堆假的画面 bug）。
pub const PT_EXPOSURE_DEFAULT: f32 = 0.1;
/// 允许区间（配置文件与 `RV3D_PT_EXPOSURE` 共用；越界一律夹回来）
pub const PT_EXPOSURE_MIN: f32 = 0.05;
pub const PT_EXPOSURE_MAX: f32 = 4.0;

/// PT 取景参数（每帧由 main.rs 注入，打包为 5×vec4 push constants）
#[derive(Clone, Copy, Default)]
pub struct PtParams {
    pub cam: glam::Vec3,
    /// 相机前向（直接取 camera.forward()，与光栅化同源，不重推 yaw/pitch 公式）
    pub fwd: glam::Vec3,
    /// **垂直**半角正切（`camera.fov` = `perspective_rh` 的 fov_y，与光栅同源）；
    /// 水平项由着色器乘 aspect 还原——2026-09-29 之前着色器两轴共用此值，
    /// PT 帧相对游戏视角水平拉伸 1.6 倍（判据见 pt_panorama.glsl 取景段注释）。
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
        // 兜底值必须引用标定常量，不能写字面量：这里原先写死 `0.4`，是旧标定值的手抄副本，
        // 2026-10-02 标定值随增益修复改成 0.1 时，它就成了全仓唯一还认为标定是 0.4 的地方。
        // （当前不可达：config 与 RV3D_PT_EXPOSURE 都夹在 PT_EXPOSURE_MIN=0.05 之上。）
        let exp = if self.exposure > 1e-4 {
            self.exposure
        } else {
            PT_EXPOSURE_DEFAULT
        };
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

/// 砌块皮肤的最小跨度（米，取盒子最长轴）——🔴 **必须与 `build.rs` WGSL 里的
/// `MASONRY_MIN_SPAN` 同值**，两处任一改动都要同步。
///
/// 为什么 PT 侧要在 CPU 判：光栅那条判据长在**顶点着色器**里（`marker_span` 读实例矩阵
/// 对角元，写进 `flat_flag` 的 1.05 子区间），而 PT 没有顶点阶段，只有盒子的
/// `center/half/tint`。`PtBox::half` 就是真实半尺寸（自 §22.14 起），所以跨度与光栅的
/// `marker_span` 同值，判据可以逐字搬过来。
/// 值不是调出来的：实测全城 1789 件 marker 里被拦的最大 1.45m、放行最小 2.20m，
/// 中间 0.75m 空档（判据与验证见 docs/PROGRESS.md §22.7）。
pub const MASONRY_MIN_SPAN: f32 = 1.5;

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

    /// 从 `mix(a, b, move)` 形式里取出两个端点常量（a=静止端、b=运动端）。
    /// 故意不写死数字：判据绑的是着色器**当下**的字面量，改 SPP/win 会立刻把这条测试推到
    /// 对曝光标定提出新要求，而不是让它悄悄绿。
    fn mix_endpoints(src: &str, needle: &str) -> (f32, f32) {
        let line = src
            .lines()
            .map(str::trim_start)
            .find(|l| !l.starts_with("//") && l.contains(needle))
            .unwrap_or_else(|| panic!("着色器里找不到 `{needle}`，归一/窗口判据已失效"));
        let open = line.find("mix(").expect("mix(") + 4;
        let close = line[open..].find(')').expect("mix 的右括号");
        let args: Vec<f32> = line[open..open + close]
            .split(',')
            .take(2)
            .map(|s| {
                s.trim()
                    .trim_end_matches('f')
                    .parse::<f32>()
                    .unwrap_or_else(|_| panic!("`{needle}` 的端点必须是浮点字面量：{s:?}"))
            })
            .collect();
        (args[0], args[1])
    }

    /// PT 的显示增益必须与 `move`（⇒ SPP、时域窗口 win）**无关**，
    /// 且 Rust 侧的曝光标定必须与着色器的归一方式**成对**。
    ///
    /// 由来：2026-10-02 修掉的缺陷 —— `lum` 是每帧 SPP 个样本之和，却直接进 EMA，
    /// 显示端又除以饱和于 win 的 `acc.a` ⇒ 稳态增益 = SPP/win = 静止 0.25 / 运动 64，
    /// 同一个像素在走与站之间摆 **256 倍**。修复 = 入栈前除以 SPP + 显示端去掉除数，
    /// 并把标定值同步 ÷4（0.4 → 0.1）以保持 §19 静态参照帧标定。
    ///
    /// 这条守卫同时卡住两侧：只改着色器（回归 256×）或只改标定（静止 PT 暗/亮 4 倍）都会红。
    #[test]
    fn pt_display_gain_is_motion_independent_and_paired_with_exposure() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/rt/pt_panorama.glsl"),
        )
        .expect("必须真的读着色器源；拿抄件比对就是假守卫");

        // 1) 入栈前必须按当帧样本数归一
        let lum = src
            .lines()
            .map(str::trim_start)
            .find(|l| !l.starts_with("//") && l.contains("lum *="))
            .expect("找不到 lum 的曝光行");
        assert!(
            lum.contains("/ float(SPP)"),
            "`lum` 是 SPP 个样本之和，进 EMA 前必须除以 SPP，否则显示增益随运动摆 SPP/win 倍：{lum}"
        );

        // 2) 显示端不得再除累积样本数（与 1) 叠加就会多除一个 SPP）
        assert!(
            !src.contains("acc.rgb / max(acc.a"),
            "显示端仍除以 acc.a：与入栈前的归一叠加，静止 PT 会暗 SPP 倍"
        );
        assert!(
            src.lines()
                .map(str::trim_start)
                .any(|l| !l.starts_with("//") && l.replace(' ', "") == "vec3outc=acc.rgb;"),
            "显示端应直接取 acc.rgb（它已是每帧均值的 EMA）"
        );

        // 3) 标定值必须 = 改前标定 × 着色器当下的静止增益
        //    改前标定是**历史冻结值**（§19 在旧归一下测出），不是从别处抄来的活值。
        const PRE_FIX_PT_EXPOSURE: f32 = 0.4;
        let (spp_rest, _spp_move) = mix_endpoints(&src, "uint SPP = uint(round(mix(");
        let (win_rest, _win_move) = mix_endpoints(&src, "float win = mix(");
        let rest_gain = spp_rest / win_rest;
        let want = PRE_FIX_PT_EXPOSURE * rest_gain;
        let have = PT_EXPOSURE_DEFAULT;
        assert!(
            (have - want).abs() < 1e-6,
            "曝光标定与着色器归一不成对：静止增益 {spp_rest}/{win_rest} = {rest_gain} ⇒ \
             PT_EXPOSURE_DEFAULT 应为 {want}，实为 {have}"
        );
        // 反空转：这条测试若被写成恒真，rest_gain 会失去意义
        assert!(
            (rest_gain - 0.25).abs() < 1e-6,
            "静止端 SPP/win 已不是 16/64，本测试的成对判据需随之重写"
        );
    }

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
