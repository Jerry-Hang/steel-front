// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    /// Gribb–Hartmann：从 proj*view 提取 6 个视锥平面（法线朝内、归一化）
    ///
    /// 公式来源：G. Gribb, K. Hartmann, "Fast Extraction of Viewing Frustum
    /// Planes from the World-View-Projection Matrix" (2001)。
    /// 平面系数来自 M 的行向量组合：左=r3+r0、右=r3−r0、下=r3+r1、上=r3−r1；
    /// Vulkan NDC z∈[0,1]，故近=r2、远=r3−r2。内部满足 dot(n,c)+d ≥ 0。
    pub(crate) fn extract_frustum_planes(
        view: glam::Mat4,
        proj: glam::Mat4,
    ) -> [[f32; 4]; 6] {
        Self::extract_frustum_planes_from(proj * view)
    }
    /// 与 `extract_frustum_planes` 同一套数学，但直接吃**已相乘**的矩阵。
    ///
    /// 加它的理由：阴影 pass 需要的是**光源**视锥（`light_data.shadow.light_view_proj`），
    /// 而那一条路径上只有乘积、没有分开的 view/proj。把平面提取收敛到一处，
    /// 比在阴影 pass 里手抄一遍 6 个平面的符号组合安全 —— 抄错一个符号就是
    /// "影子随机缺一块"，且不报任何错。
    pub(crate) fn extract_frustum_planes_from(m4: glam::Mat4) -> [[f32; 4]; 6] {
        let m = m4.to_cols_array_2d(); // m[col][row]
        let row = |i: usize| [m[0][i], m[1][i], m[2][i], m[3][i]];
        let r0 = row(0);
        let r1 = row(1);
        let r2 = row(2);
        let r3 = row(3);

        let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
        let sub = |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
        let normalize = |p: [f32; 4]| {
            let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            [p[0] / len, p[1] / len, p[2] / len, p[3] / len]
        };

        [
            normalize(add(r3, r0)), // 左
            normalize(sub(r3, r0)), // 右
            normalize(add(r3, r1)), // 下
            normalize(sub(r3, r1)), // 上
            normalize(r2),          // 近
            normalize(sub(r3, r2)), // 远
        ]
    }
    /// 标量视锥剔除（回退路径）：对每个实例判 6 平面，d = dot(n,c)+d，
    /// 任一平面 d < -r 即剔除；全部平面 d >= -r 才可见。
    pub(crate) fn cull_spheres_scalar(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        for i in 0..cx.len() {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[i] + p[1] * cy[i] + p[2] * cz[i] + p[3];
                if d < -radii[i] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(i as u32);
            }
        }
    }
    /// AVX2 批量视锥剔除：8 实例/批（256 位 8×f32），6 平面点积全部向量化。
    /// 与标量版逐位一致：非 FMA，累加顺序严格 ((nx*x+ny*y)+nz*z)+d，
    /// 比较 d >= -r 得保留掩码并按实例序输出，无 NaN 输入（全部有限数）。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn cull_spheres_avx2(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        use std::arch::x86_64::*;
        let n = cx.len();
        debug_assert_eq!(n, cy.len());
        debug_assert_eq!(n, cz.len());
        debug_assert_eq!(n, radii.len());

        let mut i = 0usize;
        while i + 8 <= n {
            let xv = _mm256_loadu_ps(cx.as_ptr().add(i));
            let yv = _mm256_loadu_ps(cy.as_ptr().add(i));
            let zv = _mm256_loadu_ps(cz.as_ptr().add(i));
            let rv = _mm256_loadu_ps(radii.as_ptr().add(i));
            // 可见掩码：初始全 1，任一平面剔除则清零
            let mut vis = _mm256_castsi256_ps(_mm256_set1_epi32(-1));
            let neg_r = _mm256_sub_ps(_mm256_setzero_ps(), rv);
            for p in planes {
                let nx = _mm256_set1_ps(p[0]);
                let ny = _mm256_set1_ps(p[1]);
                let nz = _mm256_set1_ps(p[2]);
                let pd = _mm256_set1_ps(p[3]);
                // d = ((nx*x + ny*y) + nz*z) + pd，与标量加法顺序一致（无 FMA）
                let d = _mm256_add_ps(
                    _mm256_add_ps(
                        _mm256_add_ps(_mm256_mul_ps(nx, xv), _mm256_mul_ps(ny, yv)),
                        _mm256_mul_ps(nz, zv),
                    ),
                    pd,
                );
                // 保留条件 d >= -r（NaN 不可能出现，有序比较安全）
                let keep = _mm256_cmp_ps(d, neg_r, _CMP_GE_OQ);
                vis = _mm256_and_ps(vis, keep);
            }
            let mask = _mm256_movemask_ps(vis) as u32;
            for k in 0..8u32 {
                if mask & (1 << k) != 0 {
                    out.push((i + k as usize) as u32);
                }
            }
            i += 8;
        }
        // 尾部不足 8 个走标量
        for j in i..n {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[j] + p[1] * cy[j] + p[2] * cz[j] + p[3];
                if d < -radii[j] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(j as u32);
            }
        }
    }
    /// AVX-512 批量视锥剔除：16 实例/批（512 位 16×f32），6 平面点积全部向量化。
    /// 与标量版逐位一致：非 FMA，累加顺序严格 ((nx*x+ny*y)+nz*z)+d，
    /// 比较 d >= -r 得 16 位掩码并按实例序输出，无 NaN 输入（全部有限数）。
    /// 适用 Zen4/Zen5（7000/9000 系）原生 512 位单元，功耗增量可忽略。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx512f")]
    pub(crate) unsafe fn cull_spheres_avx512(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        use std::arch::x86_64::*;
        let n = cx.len();
        debug_assert_eq!(n, cy.len());
        debug_assert_eq!(n, cz.len());
        debug_assert_eq!(n, radii.len());

        let mut i = 0usize;
        while i + 16 <= n {
            let xv = _mm512_loadu_ps(cx.as_ptr().add(i));
            let yv = _mm512_loadu_ps(cy.as_ptr().add(i));
            let zv = _mm512_loadu_ps(cz.as_ptr().add(i));
            let rv = _mm512_loadu_ps(radii.as_ptr().add(i));
            // 可见掩码：初始全 1，任一平面剔除则清零
            let mut vis: __mmask16 = 0xFFFF;
            let neg_r = _mm512_sub_ps(_mm512_setzero_ps(), rv);
            for p in planes {
                let nx = _mm512_set1_ps(p[0]);
                let ny = _mm512_set1_ps(p[1]);
                let nz = _mm512_set1_ps(p[2]);
                let pd = _mm512_set1_ps(p[3]);
                // d = ((nx*x + ny*y) + nz*z) + pd，与标量加法顺序一致（无 FMA）
                let d = _mm512_add_ps(
                    _mm512_add_ps(
                        _mm512_add_ps(_mm512_mul_ps(nx, xv), _mm512_mul_ps(ny, yv)),
                        _mm512_mul_ps(nz, zv),
                    ),
                    pd,
                );
                // 保留条件 d >= -r（NaN 不可能出现，有序比较安全）
                let keep = _mm512_cmp_ps_mask(d, neg_r, _CMP_GE_OQ);
                vis &= keep;
            }
            for k in 0..16u32 {
                if vis & (1 << k) != 0 {
                    out.push((i + k as usize) as u32);
                }
            }
            i += 16;
        }
        // 尾部不足 16 个走标量
        for j in i..n {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[j] + p[1] * cy[j] + p[2] * cz[j] + p[3];
                if d < -radii[j] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(j as u32);
            }
        }
    }
    /// AVX（非 AVX2，第 3/4 代酷睿与初代锐龙）批量剔除：8 实例/批（256 位 8×f32）。
    /// 与标量逐位一致（非 FMA 累加顺序相同）；浮点 add/mul/cmp 在 AVX 即已具备，
    /// 无 AVX2 的 FMA/gather 需求，故可独立成档。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx")]
    pub(crate) unsafe fn cull_spheres_avx(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        use std::arch::x86_64::*;
        let n = cx.len();
        debug_assert_eq!(n, cy.len());
        debug_assert_eq!(n, cz.len());
        debug_assert_eq!(n, radii.len());

        let mut i = 0usize;
        while i + 8 <= n {
            let xv = _mm256_loadu_ps(cx.as_ptr().add(i));
            let yv = _mm256_loadu_ps(cy.as_ptr().add(i));
            let zv = _mm256_loadu_ps(cz.as_ptr().add(i));
            let rv = _mm256_loadu_ps(radii.as_ptr().add(i));
            let mut vis = _mm256_castsi256_ps(_mm256_set1_epi32(-1));
            let neg_r = _mm256_sub_ps(_mm256_setzero_ps(), rv);
            for p in planes {
                let nx = _mm256_set1_ps(p[0]);
                let ny = _mm256_set1_ps(p[1]);
                let nz = _mm256_set1_ps(p[2]);
                let pd = _mm256_set1_ps(p[3]);
                let d = _mm256_add_ps(
                    _mm256_add_ps(
                        _mm256_add_ps(_mm256_mul_ps(nx, xv), _mm256_mul_ps(ny, yv)),
                        _mm256_mul_ps(nz, zv),
                    ),
                    pd,
                );
                let keep = _mm256_cmp_ps(d, neg_r, _CMP_GE_OQ);
                vis = _mm256_and_ps(vis, keep);
            }
            let mask = _mm256_movemask_ps(vis) as u32;
            for k in 0..8u32 {
                if mask & (1 << k) != 0 {
                    out.push((i + k as usize) as u32);
                }
            }
            i += 8;
        }
        // 尾部不足 8 个走标量
        for j in i..n {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[j] + p[1] * cy[j] + p[2] * cz[j] + p[3];
                if d < -radii[j] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(j as u32);
            }
        }
    }
    /// SSE4.2（2008 年后所有 Intel/AMD 消费级）批量剔除：4 实例/批（128 位 4×f32）。
    /// 与标量逐位一致（非 FMA 累加顺序相同）；比标量约 2-3×，覆盖无 AVX 的老平台。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "sse4.2")]
    pub(crate) unsafe fn cull_spheres_sse(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        use std::arch::x86_64::*;
        let n = cx.len();
        debug_assert_eq!(n, cy.len());
        debug_assert_eq!(n, cz.len());
        debug_assert_eq!(n, radii.len());

        let mut i = 0usize;
        while i + 4 <= n {
            let xv = _mm_loadu_ps(cx.as_ptr().add(i));
            let yv = _mm_loadu_ps(cy.as_ptr().add(i));
            let zv = _mm_loadu_ps(cz.as_ptr().add(i));
            let rv = _mm_loadu_ps(radii.as_ptr().add(i));
            let mut vis = _mm_castsi128_ps(_mm_set1_epi32(-1));
            let neg_r = _mm_sub_ps(_mm_setzero_ps(), rv);
            for p in planes {
                let nx = _mm_set1_ps(p[0]);
                let ny = _mm_set1_ps(p[1]);
                let nz = _mm_set1_ps(p[2]);
                let pd = _mm_set1_ps(p[3]);
                let d = _mm_add_ps(
                    _mm_add_ps(
                        _mm_add_ps(_mm_mul_ps(nx, xv), _mm_mul_ps(ny, yv)),
                        _mm_mul_ps(nz, zv),
                    ),
                    pd,
                );
                let keep = _mm_cmp_ps(d, neg_r, _CMP_GE_OQ);
                vis = _mm_and_ps(vis, keep);
            }
            let mask = _mm_movemask_ps(vis) as u32;
            for k in 0..4u32 {
                if mask & (1 << k) != 0 {
                    out.push((i + k as usize) as u32);
                }
            }
            i += 4;
        }
        // 尾部不足 4 个走标量
        for j in i..n {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[j] + p[1] * cy[j] + p[2] * cz[j] + p[3];
                if d < -radii[j] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(j as u32);
            }
        }
    }
    /// NEON（AArch64，Apple Silicon / Android / 高通 X Elite 通用）批量剔除：
    /// 4 实例/批（128 位 4×f32）。与标量逐位一致（非 FMA 累加顺序相同）；
    /// Apple Silicon 的 SIMD 即标准 NEON（AMX 为私有协处理器，SVE 苹果不支持）。
    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn cull_spheres_neon(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        use std::arch::aarch64::*;
        let n = cx.len();
        debug_assert_eq!(n, cy.len());
        debug_assert_eq!(n, cz.len());
        debug_assert_eq!(n, radii.len());

        let mut i = 0usize;
        while i + 4 <= n {
            let xv = vld1q_f32(cx.as_ptr().add(i));
            let yv = vld1q_f32(cy.as_ptr().add(i));
            let zv = vld1q_f32(cz.as_ptr().add(i));
            let rv = vld1q_f32(radii.as_ptr().add(i));
            // 可见掩码：初始全 1（u32 lane），任一平面剔除则清零
            let mut vis = vdupq_n_u32(0xFFFF_FFFF);
            let neg_r = vsubq_f32(vdupq_n_f32(0.0), rv);
            for p in planes {
                let nx = vdupq_n_f32(p[0]);
                let ny = vdupq_n_f32(p[1]);
                let nz = vdupq_n_f32(p[2]);
                let pd = vdupq_n_f32(p[3]);
                // d = ((nx*x + ny*y) + nz*z) + pd，与标量加法顺序一致（无 FMA）
                let d = vaddq_f32(
                    vaddq_f32(
                        vaddq_f32(vmulq_f32(nx, xv), vmulq_f32(ny, yv)),
                        vmulq_f32(nz, zv),
                    ),
                    pd,
                );
                // 保留条件 d >= -r（NaN 不可能出现）
                let keep = vcgeq_f32(d, neg_r);
                vis = vandq_u32(vis, keep);
            }
            // 提取 4 位可见掩码（每 lane 全 1/全 0，取最低位）
            let mask = (vgetq_lane_u32(vis, 0) & 1)
                | ((vgetq_lane_u32(vis, 1) & 1) << 1)
                | ((vgetq_lane_u32(vis, 2) & 1) << 2)
                | ((vgetq_lane_u32(vis, 3) & 1) << 3);
            for k in 0..4u32 {
                if mask & (1 << k) != 0 {
                    out.push((i + k as usize) as u32);
                }
            }
            i += 4;
        }
        // 尾部不足 4 个走标量
        for j in i..n {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[j] + p[1] * cy[j] + p[2] * cz[j] + p[3];
                if d < -radii[j] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(j as u32);
            }
        }
    }
    /// 每帧视锥剔除 + 距离 LOD 分档：
    /// 设置世界障碍 marker（关卡切换时由 main.rs 调用；容量截断到 MAX_MARKER_INSTANCES）
    pub(crate) fn set_world_markers(&mut self, markers: &[WorldMarker]) {
        self.markers = markers
            .iter()
            .take(MAX_MARKER_INSTANCES as usize)
            .map(|m| InstanceData {
                model: m.model.to_cols_array(),
                tint: m.tint,
            })
            .collect();
    }
    /// 追加一批世界 marker（弹孔等每帧变化的实例），容量仍截断到 `MAX_MARKER_INSTANCES`。
    ///
    /// 与 `set_world_markers` 分开的理由是 **PT 场景不吃这一批**：弹孔每次开枪都变，
    /// 混进 `pt_set_scene_markers` 会让 BLAS 指纹每帧判"场景变了"而重建，
    /// 还会白占 PT 盒容量（`PT_MAX_BOXES`）。
    pub(crate) fn append_markers(&mut self, extra: &[WorldMarker]) {
        let room = (MAX_MARKER_INSTANCES as usize).saturating_sub(self.markers.len());
        self.markers
            .extend(extra.iter().take(room).map(|m| InstanceData {
                model: m.model.to_cols_array(),
                tint: m.tint,
            }));
    }
    /// 每帧上传世界障碍 marker 到实例 buffer 的 MARKER_SLOT_BASE 之后区域
    /// （跳过 65536 identity slot，见 MARKER_SLOT_BASE 注释），返回 (近档, 远档) 计数。
    /// marker 数量**不小**（全城实测 ~1789 件，容量 `MAX_MARKER_INSTANCES`=8192），
    /// 但历史上不做视锥剔除，只按距离分近/远档；且当前 `near_sq = f32::MAX`
    /// ⇒ 远档恒空（障碍 marker 恒走近档立方体，见下方 `upload_markers`）。
    pub(crate) fn upload_markers(&mut self, cam_pos: glam::Vec3) -> (u32, u32) {
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return (0, 0),
        };
        let stride = std::mem::size_of::<InstanceData>();
        if self.mesh_enabled {
            // mesh 路径：不做近/远压缩，顺序写槽位（几何由 shader 按距离自选）。
            // 返回 (count, 0)：计数仍供 draw 范围与性能日志使用。
            let count = self.markers.len() as u32;
            for (i, inst) in self.markers.iter().enumerate() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((MARKER_SLOT_BASE + i as u32) as usize) * stride),
                        stride,
                    );
                }
            }
            return (count, 0);
        }
        // 近/远档分界距离随画质预设变化（Medium 与原 LOD_DISTANCE 一致）。
        // 障碍 marker 恒走近档立方体：远档十字 quad 俯视呈"方块贴图+缝隙"（用户反馈）。
        let near_sq = f32::MAX;
        let mut near_count = 0u32;
        // 近档先写（base..base+near-1），远档紧随（base+near..），两遍遍历避免槽位交错
        for inst in &self.markers {
            let dx = inst.model[12] - cam_pos.x;
            let dy = inst.model[13] - cam_pos.y;
            let dz = inst.model[14] - cam_pos.z;
            if dx * dx + dy * dy + dz * dz < near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((MARKER_SLOT_BASE + near_count) as usize) * stride),
                        stride,
                    );
                }
                near_count += 1;
            }
        }
        let mut far_count = 0u32;
        for inst in &self.markers {
            let dx = inst.model[12] - cam_pos.x;
            let dy = inst.model[13] - cam_pos.y;
            let dz = inst.model[14] - cam_pos.z;
            if dx * dx + dy * dy + dz * dz >= near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(
                            ((MARKER_SLOT_BASE + near_count + far_count) as usize) * stride,
                        ),
                        stride,
                    );
                }
                far_count += 1;
            }
        }
        (near_count, far_count)
    }
    /// 设置自发光实体（爆炸闪光等；每帧由 main.rs 传入，容量截断到 MAX_EMISSIVE_INSTANCES）
    pub(crate) fn set_emissive_markers(&mut self, markers: &[WorldMarker]) {
        self.emissive_markers = markers
            .iter()
            .take(MAX_EMISSIVE_INSTANCES as usize)
            .map(|m| InstanceData {
                model: m.model.to_cols_array(),
                tint: m.tint,
            })
            .collect();
    }
    /// 每帧上传自发光实体到实例 buffer 的 EMISSIVE_SLOT_BASE 之后区域，返回 (近档, 远档) 计数。
    /// 与 marker 同构：量小不剔除，仅按距离分近/远档（shader 侧 flat+fade>1 直出自发光色）。
    pub(crate) fn upload_emissive(&mut self, cam_pos: glam::Vec3) -> (u32, u32) {
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return (0, 0),
        };
        let stride = std::mem::size_of::<InstanceData>();
        if self.mesh_enabled {
            let count = self.emissive_markers.len() as u32;
            for (i, inst) in self.emissive_markers.iter().enumerate() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((EMISSIVE_SLOT_BASE + i as u32) as usize) * stride),
                        stride,
                    );
                }
            }
            return (count, 0);
        }
        let near_sq = quality_params(self.quality).instance_lod_distance;
        let near_sq = near_sq * near_sq;
        let mut near_count = 0u32;
        for inst in &self.emissive_markers {
            let dx = inst.model[12] - cam_pos.x;
            let dy = inst.model[13] - cam_pos.y;
            let dz = inst.model[14] - cam_pos.z;
            if dx * dx + dy * dy + dz * dz < near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((EMISSIVE_SLOT_BASE + near_count) as usize) * stride),
                        stride,
                    );
                }
                near_count += 1;
            }
        }
        let mut far_count = 0u32;
        for inst in &self.emissive_markers {
            let dx = inst.model[12] - cam_pos.x;
            let dy = inst.model[13] - cam_pos.y;
            let dz = inst.model[14] - cam_pos.z;
            if dx * dx + dy * dy + dz * dz >= near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(
                            ((EMISSIVE_SLOT_BASE + near_count + far_count) as usize) * stride,
                        ),
                        stride,
                    );
                }
                far_count += 1;
            }
        }
        (near_count, far_count)
    }
    /// 设置 NPC 士兵可视化（由 main.rs 传入全部 NPC 的位置/朝向/配色/动画态；
    /// 15 段/人（尸体 15 段）展开存入 npc_parts，总段数截断到 MAX_NPC_INSTANCES）
    /// NPC 段数触顶时的**一次性**告警（2026-09-14）。
    ///
    /// 背景：`set_npc_visuals` / `set_dead_bodies` 里各有一处
    /// `if len < MAX_NPC_INSTANCES { push }`，超容时**静默丢弃** ——
    /// 不崩、不报 VUID，画面上只是"少了几个兵"。这就是未结案 #12 那一类失败模式
    /// （"溢出静默丢弃"），而它正是最难查的：没有任何东西告诉你人少了。
    ///
    /// 容量本身够用（`MAX_NPC_INSTANCES = 3072`，128v128 + 尸体实测峰值 2220），
    /// 所以这条**正常情况下永不触发**。它存在的意义是：一旦触发，
    /// 排查的人能立刻定位到"触顶"，而不必去怀疑剔除矩阵 / 模型 / 取景。
    ///
    /// 用闩而不是每次都记：这两个函数每帧都跑，每帧刷日志会把真正有用的行淹掉。
    pub(crate) fn warn_npc_cap_once(&mut self) {
        if self.npc_cap_warned {
            return;
        }
        self.npc_cap_warned = true;
        log::warn!(
            "npc: 段数触顶 MAX_NPC_INSTANCES={} (box={} cyl={} sph={}) => 超出部分被静默丢弃，场景里会少人",
            MAX_NPC_INSTANCES,
            self.npc_box_parts.len(),
            self.npc_cyl_parts.len(),
            self.npc_sph_parts.len()
        );
    }
    /// 每帧把 `soldier_parts` 写进实例缓冲的士兵区。返回实际写入数。
    ///
    /// **超出容量的部分直接不写**（并由调用方计数），**绝不越界** ——
    /// 越界写实例 storage buffer 的后果是驱动静默返回全零、几何塌成一点（铁律 B）。
    pub(crate) fn upload_soldiers(&mut self) -> u32 {
        self.soldier_drawn = 0;
        if self.soldier_vertex_count == 0 || self.soldier_parts.is_empty() {
            return 0;
        }
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return 0,
        };
        let stride = std::mem::size_of::<InstanceData>();
        let n = self.soldier_parts.len().min(MAX_SOLDIER_INSTANCES as usize);
        for (i, inst) in self.soldier_parts.iter().take(n).enumerate() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    inst as *const InstanceData as *const u8,
                    slot.add((SOLDIER_INSTANCE_BASE as usize + i) * stride),
                    stride,
                );
            }
        }
        self.soldier_drawn = n as u32;
        self.soldier_parts.clear();
        self.soldier_drawn
    }
    /// 每帧上传 NPC 士兵段到实例 buffer 的 NPC_SLOT_BASE 之后区域，
    /// 仿照 upload_markers 按距离分近/远档（不剔除，仅距离分档），返回 (近档, 远档) 计数。
    /// 上传 NPC 三几何区（盒/圆柱/球），每区按距离分近/远档。
    /// 返回 ((盒 near,far),(圆柱 near,far),(球 near,far))。
    pub(crate) fn upload_npcs(
        &mut self,
        cam_pos: glam::Vec3,
    ) -> ((u32, u32), (u32, u32), (u32, u32)) {
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return ((0, 0), (0, 0), (0, 0)),
        };
        let stride = std::mem::size_of::<InstanceData>();
        if self.mesh_enabled {
            // mesh 路径：全量上传（无分档），计数 = 各组长度
            for (i, inst) in self.npc_box_parts.iter().enumerate() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((NPC_SLOT_BASE + i as u32) as usize) * stride),
                        stride,
                    );
                }
            }
            for (i, inst) in self.npc_cyl_parts.iter().enumerate() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((NPC_CYL_SLOT_BASE + i as u32) as usize) * stride),
                        stride,
                    );
                }
            }
            for (i, inst) in self.npc_sph_parts.iter().enumerate() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((NPC_SPH_SLOT_BASE + i as u32) as usize) * stride),
                        stride,
                    );
                }
            }
            return (
                (self.npc_box_parts.len() as u32, 0),
                (self.npc_cyl_parts.len() as u32, 0),
                (self.npc_sph_parts.len() as u32, 0),
            );
        }
        // 近/远档分界距离随画质预设变化（与 marker/实例场同源）
        let near_sq = quality_params(self.quality).instance_lod_distance;
        let near_sq = near_sq * near_sq;
        let cam = glam::Vec3::new(cam_pos.x, cam_pos.y, cam_pos.z);
        let box_zone = Self::upload_zone_rel(&self.npc_box_parts, NPC_SLOT_BASE, near_sq, slot, stride, cam);
        let cyl_zone = Self::upload_zone_rel(&self.npc_cyl_parts, NPC_CYL_SLOT_BASE, near_sq, slot, stride, cam);
        let sph_zone = Self::upload_zone_rel(&self.npc_sph_parts, NPC_SPH_SLOT_BASE, near_sq, slot, stride, cam);
        (box_zone, cyl_zone, sph_zone)
    }
    /// 相对相机位置的分档上传（模型平移列 - 相机位置）
    pub(crate) fn upload_zone_rel(
        parts: &[InstanceData],
        base: u32,
        near_sq: f32,
        slot: *mut u8,
        stride: usize,
        cam: glam::Vec3,
    ) -> (u32, u32) {
        let mut near_count = 0u32;
        for inst in parts {
            let dx = inst.model[12] - cam.x;
            let dy = inst.model[13] - cam.y;
            let dz = inst.model[14] - cam.z;
            if dx * dx + dy * dy + dz * dz < near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((base + near_count) as usize) * stride),
                        stride,
                    );
                }
                near_count += 1;
            }
        }
        let mut far_count = 0u32;
        for inst in parts {
            let dx = inst.model[12] - cam.x;
            let dy = inst.model[13] - cam.y;
            let dz = inst.model[14] - cam.z;
            if dx * dx + dy * dy + dz * dz >= near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((base + near_count + far_count) as usize) * stride),
                        stride,
                    );
                }
                far_count += 1;
            }
        }
        (near_count, far_count)
    }
    /// 每帧视锥剔除 + 距离 LOD 分档（多核并行）：
    /// 可见实例按 [近档][远档] 连续压缩上传到当前帧 slot，返回 (near, far)。
    ///
    /// 并行结构（`cpu::scene_pool()`：AMD 绑首簇 CCD0、Intel 仅 P-core，与渲染主线程
    /// 同簇，杜绝跨 CCD 访问与 E-core 调度；渲染线程不固定 1-2 核——池与主线程同绑整簇
    /// 集合，由 OS 调度器把渲染侧工作分给集合内空闲率最高的核）：
    /// - 阶段 A：按段并行剔除（每段走 SIMD 选路，见 cull_spheres_dispatch），
    ///   可见全局索引写入 `culled_scratch` 对应段，同时统计各段近/远档计数；
    /// - 前缀和（串行，段数 ≤ 9，微秒级）：算出每段近/远档在 slot 中的写入起点；
    /// - 阶段 B：按段并行把 80B 实例拷贝到 slot 压缩偏移（段内近档在前、远档随后）。
    pub(crate) fn cull_and_upload(
        &mut self,
        view: glam::Mat4,
        proj: glam::Mat4,
        cam_pos: glam::Vec3,
    ) -> (u32, u32) {
        let planes = Self::extract_frustum_planes(view, proj);
        let near_sq = quality_params(self.quality).instance_lod_distance;
        let near_sq = near_sq * near_sq;
        let stride = std::mem::size_of::<InstanceData>();
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return (0, 0),
        };

        // 池大小（调用线程参与首段 → 并发 = workers+1）；段计数数组按需建一次
        let pool = crate::engine::cpu::scene_pool();
        // 段数硬夹 `CULL_MAX_SEGMENTS`（下面两张前缀和表是栈上定长数组，见该常量的注释）
        let nw = cull_segment_count(pool.workers());
        if self.seg_near_counts.len() != nw {
            self.seg_near_counts = (0..nw)
                .map(|_| std::sync::atomic::AtomicU32::new(0))
                .collect();
            self.seg_far_counts = (0..nw)
                .map(|_| std::sync::atomic::AtomicU32::new(0))
                .collect();
        }

        // 拆借引用（同一 self 的多字段并行借用；闭包 move 捕获各自引用）
        let cx = &self.instance_center_x;
        let cy = &self.instance_center_y;
        let cz = &self.instance_center_z;
        let radii = &self.instance_radii;
        let instances = &self.instances;
        let seg_near = &self.seg_near_counts;
        let seg_far = &self.seg_far_counts;
        let scratch = &mut self.culled_scratch;

        // ---- 阶段 A：并行剔除 + 近/远分档计数（段内 SIMD 选路，见 cull_spheres_dispatch）----
        pool.par_for_each_mut(scratch, move |seg, start, seg_slice| {
            let end = start + seg_slice.len();
            // 每段局部剔除结果（段实例数为容量上限，可见数通常远小于此）
            let mut local: Vec<u32> = Vec::with_capacity(seg_slice.len());
            Self::cull_spheres_dispatch(
                &cx[start..end],
                &cy[start..end],
                &cz[start..end],
                &radii[start..end],
                &planes,
                &mut local,
            );
            // 段暂存写入全局索引（段内偏移 + 段起点）
            for (k, &li) in local.iter().enumerate() {
                seg_slice[k] = (start + li as usize) as u32;
            }
            // 近/远档计数（与串行版同一距离² 判定，结果一致）
            let mut near = 0u32;
            let mut far = 0u32;
            for &gi in &seg_slice[..local.len()] {
                let inst = &instances[gi as usize];
                let dx = inst.model[12] - cam_pos.x;
                let dy = inst.model[13] - cam_pos.y;
                let dz = inst.model[14] - cam_pos.z;
                if dx * dx + dy * dy + dz * dz < near_sq {
                    near += 1;
                } else {
                    far += 1;
                }
            }
            seg_near[seg].store(near, std::sync::atomic::Ordering::Relaxed);
            seg_far[seg].store(far, std::sync::atomic::Ordering::Relaxed);
        });

        // ---- 前缀和（串行，段数 ≤ 9，微秒级）：每段近/远档写入起点 ----
        // 表长 = `CULL_MAX_SEGMENTS`，而 `nw` 已由 `cull_segment_count` 夹到同一上限
        // （原来这里是一句 `debug_assert!(nw <= 64)` —— release 里不存在，等于没写）
        let mut near_prefix = [0u32; CULL_MAX_SEGMENTS];
        let mut far_prefix = [0u32; CULL_MAX_SEGMENTS];
        let mut near_total = 0u32;
        let mut far_total = 0u32;
        for w in 0..nw {
            near_prefix[w] = near_total;
            near_total += seg_near[w].load(std::sync::atomic::Ordering::Relaxed);
            far_prefix[w] = far_total;
            far_total += seg_far[w].load(std::sync::atomic::Ordering::Relaxed);
        }

        // ---- 阶段 B：按段并行压缩上传（近档在前、远档随后；段间偏移由前缀和保证互不相交）----
        let slot_ptr = crate::engine::cpu::SendPtr(slot);
        pool.par_for_each_mut(scratch, move |seg, _start, seg_slice| {
            let count = (seg_near[seg].load(std::sync::atomic::Ordering::Relaxed)
                + seg_far[seg].load(std::sync::atomic::Ordering::Relaxed))
                as usize;
            let near_off = near_prefix[seg] as usize;
            let far_off = (near_total + far_prefix[seg]) as usize;
            let base = slot_ptr.get();
            let mut near_k = 0usize;
            let mut far_k = 0usize;
            for &gi in &seg_slice[..count] {
                let inst = &instances[gi as usize];
                let dx = inst.model[12] - cam_pos.x;
                let dy = inst.model[13] - cam_pos.y;
                let dz = inst.model[14] - cam_pos.z;
                // SAFETY: base 指向当前帧实例槽（映射内存），偏移由前缀和保证落在槽内
                let dst = unsafe {
                    if dx * dx + dy * dy + dz * dz < near_sq {
                        let d = base.add((near_off + near_k) * stride);
                        near_k += 1;
                        d
                    } else {
                        let d = base.add((far_off + far_k) * stride);
                        far_k += 1;
                        d
                    }
                };
                // SAFETY: 段内近/远档写入游标互不相交；段间偏移由前缀和保证互不相交；
                // par_for_each_mut join 后才返回，slot 在本次调用内不会再被触碰。
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        dst,
                        stride,
                    );
                }
            }
        });
        (near_total, far_total)
    }
    /// 按运行时指令集选路执行视锥剔除（各档路径与标量逐位一致，非 FMA）：
    /// x86_64：AVX-512（16 实例/批）> AVX2（8）> AVX（8）> SSE4.2（4）> 标量；
    /// aarch64：NEON（4）> 标量；其余平台：标量。
    /// ★ AVX-512 说明：本机 Zen4（8940HX，双 256 单元合并执行 512 位）实测走
    ///   16 实例/批路径；选路统一走 cpu::avx512_enabled()——硬件不支持、
    ///   RV3D_DISABLE_AVX512=1、Intel 11 代（能效差）与 12 代起（大小核）自动禁用回退。
    pub(crate) fn cull_spheres_dispatch(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        #[cfg(target_arch = "x86_64")]
        {
            // 基准用强制选路（RV3D_FORCE_SIMD，见 cpu::forced_simd_path）；仍要求硬件支持
            if let Some(forced) = crate::engine::cpu::forced_simd_path() {
                let supported = match forced {
                    "avx512" => std::is_x86_feature_detected!("avx512f"),
                    "avx2" => std::is_x86_feature_detected!("avx2"),
                    "avx" => std::is_x86_feature_detected!("avx"),
                    "sse4.2" => std::is_x86_feature_detected!("sse4.2"),
                    "scalar" => true,
                    _ => false,
                };
                if supported {
                    match forced {
                        "avx512" => {
                            // safety: 上面已确认 avx512f 硬件支持
                            unsafe {
                                Self::cull_spheres_avx512(cx, cy, cz, radii, planes, out);
                            }
                        }
                        "avx2" => {
                            // safety: 上面已确认 avx2 硬件支持
                            unsafe {
                                Self::cull_spheres_avx2(cx, cy, cz, radii, planes, out);
                            }
                        }
                        "avx" => {
                            // safety: 上面已确认 avx 硬件支持
                            unsafe {
                                Self::cull_spheres_avx(cx, cy, cz, radii, planes, out);
                            }
                        }
                        "sse4.2" => {
                            // safety: 上面已确认 sse4.2 硬件支持
                            unsafe {
                                Self::cull_spheres_sse(cx, cy, cz, radii, planes, out);
                            }
                        }
                        _ => Self::cull_spheres_scalar(cx, cy, cz, radii, planes, out),
                    }
                    return;
                }
                // 每帧调用（morph 每级 / 剔除每段）⇒ 走一次性告警，见 `simd::warn_forced_simd_unsupported`
                crate::engine::simd::warn_forced_simd_unsupported(forced);
            }
            if crate::engine::cpu::avx512_enabled() {
                // safety: 上面已运行时检测 AVX-512，CPU 支持才进入该分支
                unsafe {
                    Self::cull_spheres_avx512(cx, cy, cz, radii, planes, out);
                }
            } else if std::is_x86_feature_detected!("avx2") {
                // safety: 上面已运行时检测 AVX2
                unsafe {
                    Self::cull_spheres_avx2(cx, cy, cz, radii, planes, out);
                }
            } else if std::is_x86_feature_detected!("avx") {
                // safety: 上面已运行时检测 AVX
                unsafe {
                    Self::cull_spheres_avx(cx, cy, cz, radii, planes, out);
                }
            } else if std::is_x86_feature_detected!("sse4.2") {
                // safety: 上面已运行时检测 SSE4.2
                unsafe {
                    Self::cull_spheres_sse(cx, cy, cz, radii, planes, out);
                }
            } else {
                Self::cull_spheres_scalar(cx, cy, cz, radii, planes, out);
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            if std::arch::is_aarch64_feature_detected!("neon") {
                // safety: NEON 在 AArch64 是基线特性（此处仍运行时确认）
                unsafe {
                    Self::cull_spheres_neon(cx, cy, cz, radii, planes, out);
                }
            } else {
                Self::cull_spheres_scalar(cx, cy, cz, radii, planes, out);
            }
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self::cull_spheres_scalar(cx, cy, cz, radii, planes, out);
        }
    }
}
