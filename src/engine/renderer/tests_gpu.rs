// 由 renderer.rs 按主题拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 唯一改动是自扫源码那几处 include_str! 的相对路径（多一层目录）。
// 这些模块仍是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段与文件作用域私有项；
// `use super::*;` 把上一层的名字转发下来，子模块里的 `use super::*;` 因此仍解析到 renderer。
use super::*;

#[cfg(test)]
mod screenshot_pixel_tests {
    use super::*;

    #[test]
    fn pixel_order_supported_formats() {
        assert_eq!(
            pixel_order_for_format(vk::Format::B8G8R8A8_UNORM),
            Ok(PixelOrder::Bgra)
        );
        assert_eq!(
            pixel_order_for_format(vk::Format::B8G8R8A8_SRGB),
            Ok(PixelOrder::Bgra)
        );
        assert_eq!(
            pixel_order_for_format(vk::Format::R8G8B8A8_UNORM),
            Ok(PixelOrder::Rgba)
        );
        assert_eq!(
            pixel_order_for_format(vk::Format::R8G8B8A8_SRGB),
            Ok(PixelOrder::Rgba)
        );
        // 未知格式 → Err
        assert!(pixel_order_for_format(vk::Format::A2B10G10R10_UNORM_PACK32).is_err());
        assert!(pixel_order_for_format(vk::Format::UNDEFINED).is_err());
    }

    #[test]
    fn convert_bgra_pixels_to_rgba() {
        // BGRA 蓝色 [255,0,0,255] → RGBA 红色 [0,0,255,255]
        let src = [255u8, 0, 0, 255];
        let mut dst = [0u8; 4];
        convert_pixels_to_rgba(vk::Format::B8G8R8A8_UNORM, &src, &mut dst).unwrap();
        assert_eq!(dst, [0, 0, 255, 255]);
        // 多像素行：R/B 交换、G/A 保持
        let src2 = [10u8, 20, 30, 40, 1, 2, 3, 4];
        let mut dst2 = [0u8; 8];
        convert_pixels_to_rgba(vk::Format::B8G8R8A8_SRGB, &src2, &mut dst2).unwrap();
        assert_eq!(dst2, [30, 20, 10, 40, 3, 2, 1, 4]);
    }

    #[test]
    fn convert_rgba_pixels_passthrough() {
        let src = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut dst = [0u8; 8];
        convert_pixels_to_rgba(vk::Format::R8G8B8A8_SRGB, &src, &mut dst).unwrap();
        assert_eq!(dst, src);
    }

    #[test]
    fn convert_pixels_rejects_bad_length_or_format() {
        // 长度不匹配 → Err
        let src = [1u8, 2, 3];
        let mut dst = [0u8; 4];
        assert!(convert_pixels_to_rgba(vk::Format::R8G8B8A8_UNORM, &src, &mut dst).is_err());
        let src2 = [1u8, 2, 3, 4];
        let mut dst2 = [0u8; 3];
        assert!(convert_pixels_to_rgba(vk::Format::R8G8B8A8_UNORM, &src2, &mut dst2).is_err());
        // 未知格式 → Err
        let mut dst3 = [0u8; 4];
        assert!(convert_pixels_to_rgba(vk::Format::UNDEFINED, &src2, &mut dst3).is_err());
    }
}

#[cfg(test)]
mod simd_cull_tests {
    use super::*;

    /// 简单确定性伪随机（SplitMix64），纯逻辑测试不碰 GPU
    struct Rng(u64);
    impl Rng {
        fn next_f32(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 33) as u32 as f64 / (1u64 << 31) as f64) as f32
        }
    }

    #[test]
    fn simd_cull_matches_scalar() {
        let mut rng = Rng(0x5EED_2026);
        // 6 个随机朝向平面（法线随机、d 随机），覆盖可见/剔除/边界混合场景
        let mut planes = [[0f32; 4]; 6];
        for p in &mut planes {
            let (nx, ny, nz) = (rng.next_f32() * 2.0 - 1.0, rng.next_f32() * 2.0 - 1.0, rng.next_f32() * 2.0 - 1.0);
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            p[0] = nx / len;
            p[1] = ny / len;
            p[2] = nz / len;
            p[3] = rng.next_f32() * 4.0 - 2.0;
        }

        let n = 65536;
        let mut cx = Vec::with_capacity(n);
        let mut cy = Vec::with_capacity(n);
        let mut cz = Vec::with_capacity(n);
        let mut radii = Vec::with_capacity(n);
        for _ in 0..n {
            cx.push(rng.next_f32() * 200.0 - 100.0);
            cy.push(rng.next_f32() * 200.0 - 100.0);
            cz.push(rng.next_f32() * 200.0 - 100.0);
            radii.push(rng.next_f32() * 4.0);
        }

        let mut scalar_out = Vec::new();
        Renderer::cull_spheres_scalar(&cx, &cy, &cz, &radii, &planes, &mut scalar_out);

        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx2") {
            let mut avx_out = Vec::new();
            // safety: 已运行时检测 AVX2
            unsafe {
                Renderer::cull_spheres_avx2(&cx, &cy, &cz, &radii, &planes, &mut avx_out);
            }
            assert_eq!(avx_out, scalar_out, "AVX2 剔除结果与标量逐位不一致");
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx512f") {
            let mut avx512_out = Vec::new();
            // safety: 已运行时检测 AVX-512
            unsafe {
                Renderer::cull_spheres_avx512(&cx, &cy, &cz, &radii, &planes, &mut avx512_out);
            }
            assert_eq!(avx512_out, scalar_out, "AVX-512 剔除结果与标量逐位不一致");
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx") {
            let mut avx_out = Vec::new();
            // safety: 已运行时检测 AVX
            unsafe {
                Renderer::cull_spheres_avx(&cx, &cy, &cz, &radii, &planes, &mut avx_out);
            }
            assert_eq!(avx_out, scalar_out, "AVX 剔除结果与标量逐位不一致");
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("sse4.2") {
            let mut sse_out = Vec::new();
            // safety: 已运行时检测 SSE4.2
            unsafe {
                Renderer::cull_spheres_sse(&cx, &cy, &cz, &radii, &planes, &mut sse_out);
            }
            assert_eq!(sse_out, scalar_out, "SSE4.2 剔除结果与标量逐位不一致");
        }
        #[cfg(target_arch = "aarch64")]
        if std::arch::is_aarch64_feature_detected!("neon") {
            let mut neon_out = Vec::new();
            // safety: 已运行时检测 NEON（AArch64 基线特性）
            unsafe {
                Renderer::cull_spheres_neon(&cx, &cy, &cz, &radii, &planes, &mut neon_out);
            }
            assert_eq!(neon_out, scalar_out, "NEON 剔除结果与标量逐位不一致");
        }
        // 非 x86_64 或无双 AVX2：标量结果本身就是正确语义，无需对照
        assert!(!scalar_out.is_empty() || scalar_out.is_empty());
    }

    #[test]
    fn simd_cull_tail_batches_handled() {
        // 长度非 8 的倍数：覆盖尾部标量路径
        let mut planes = [[0f32; 4]; 6];
        for p in &mut planes {
            p[0] = 0.0;
            p[1] = 0.0;
            p[2] = 1.0;
            p[3] = 0.0;
        }
        let n = 13; // 1 个 AVX2 批 + 5 个尾部
        let mut cx = Vec::with_capacity(n);
        let mut cy = Vec::with_capacity(n);
        let mut cz = Vec::with_capacity(n);
        let mut radii = Vec::with_capacity(n);
        for i in 0..n {
            cx.push((i as f32) - 6.0);
            cy.push(0.0);
            cz.push((i as f32) - 6.0);
            radii.push(1.0);
        }
        let mut scalar_out = Vec::new();
        Renderer::cull_spheres_scalar(&cx, &cy, &cz, &radii, &planes, &mut scalar_out);

        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx2") {
            let mut avx_out = Vec::new();
            unsafe {
                Renderer::cull_spheres_avx2(&cx, &cy, &cz, &radii, &planes, &mut avx_out);
            }
            assert_eq!(avx_out, scalar_out);
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx512f") {
            let mut avx512_out = Vec::new();
            unsafe {
                Renderer::cull_spheres_avx512(&cx, &cy, &cz, &radii, &planes, &mut avx512_out);
            }
            assert_eq!(avx512_out, scalar_out);
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx") {
            let mut avx_out = Vec::new();
            unsafe {
                Renderer::cull_spheres_avx(&cx, &cy, &cz, &radii, &planes, &mut avx_out);
            }
            assert_eq!(avx_out, scalar_out);
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("sse4.2") {
            let mut sse_out = Vec::new();
            unsafe {
                Renderer::cull_spheres_sse(&cx, &cy, &cz, &radii, &planes, &mut sse_out);
            }
            assert_eq!(sse_out, scalar_out);
        }
        #[cfg(target_arch = "aarch64")]
        if std::arch::is_aarch64_feature_detected!("neon") {
            let mut neon_out = Vec::new();
            // safety: 已运行时检测 NEON（AArch64 基线特性）
            unsafe {
                Renderer::cull_spheres_neon(&cx, &cy, &cz, &radii, &planes, &mut neon_out);
            }
            assert_eq!(neon_out, scalar_out, "NEON 剔除结果与标量逐位不一致");
        }
    }

    #[test]
    fn simd_morph_matches_scalar() {
        // 覆盖各档批量倍数 + 尾部：33（AVX-512 两批+1 尾部）与 65536（整场地形网格）全量对比
        for n in [33usize, 65536usize] {
            let mut rng = Rng(0x51AD_0007 ^ n as u64);
            let mut base = Vec::with_capacity(n);
            let mut coarse = Vec::with_capacity(n);
            for _ in 0..n {
                base.push(rng.next_f32() * 40.0 - 20.0);
                coarse.push(rng.next_f32() * 40.0 - 20.0);
            }
            let blend = rng.next_f32();
            let mut scalar_out = vec![0.0f32; n];
            Renderer::morph_heights_scalar(&base, &coarse, blend, &mut scalar_out);

            let mut out = vec![0.0f32; n];
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx512f") && crate::engine::cpu::avx512_enabled() {
                // safety: 已运行时检测 AVX-512 且未被型号过滤
                unsafe {
                    Renderer::morph_heights_avx512(&base, &coarse, blend, &mut out);
                }
                assert_eq!(out, scalar_out, "AVX-512 morph 与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx2") {
                // safety: 已运行时检测 AVX2
                unsafe {
                    Renderer::morph_heights_avx2(&base, &coarse, blend, &mut out);
                }
                assert_eq!(out, scalar_out, "AVX2 morph 与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx") {
                // safety: 已运行时检测 AVX
                unsafe {
                    Renderer::morph_heights_avx(&base, &coarse, blend, &mut out);
                }
                assert_eq!(out, scalar_out, "AVX morph 与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("sse4.2") {
                // safety: 已运行时检测 SSE4.2
                unsafe {
                    Renderer::morph_heights_sse(&base, &coarse, blend, &mut out);
                }
                assert_eq!(out, scalar_out, "SSE4.2 morph 与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "aarch64")]
            if std::arch::is_aarch64_feature_detected!("neon") {
                // safety: NEON 在 AArch64 是基线特性（此处仍运行时确认）
                unsafe {
                    Renderer::morph_heights_neon(&base, &coarse, blend, &mut out);
                }
                assert_eq!(out, scalar_out, "NEON morph 与标量逐位不一致 (n={})", n);
            }
            // dispatch 选路（当前机器实际启用档位）也必须与标量逐位一致
            out.fill(0.0);
            Renderer::morph_heights_dispatch(&base, &coarse, blend, &mut out);
            assert_eq!(out, scalar_out, "dispatch morph 与标量逐位不一致 (n={})", n);
        }
    }

    /// 指令级微基准（隔离进程、无渲染并发）：65536 实例剔除 + 65536 点 morph，各档 vs 标量。
    /// 运行：`cargo test --release simd_cull_microbench -- --nocapture --test-threads=1`
    #[test]
    fn simd_cull_microbench() {
        let mut rng = Rng(0xC011_2026);
        let mut planes = [[0f32; 4]; 6];
        for p in &mut planes {
            let (nx, ny, nz) = (
                rng.next_f32() * 2.0 - 1.0,
                rng.next_f32() * 2.0 - 1.0,
                rng.next_f32() * 2.0 - 1.0,
            );
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            p[0] = nx / len;
            p[1] = ny / len;
            p[2] = nz / len;
            p[3] = rng.next_f32() * 4.0 - 2.0;
        }
        let n = 65536usize;
        let mut cx = Vec::with_capacity(n);
        let mut cy = Vec::with_capacity(n);
        let mut cz = Vec::with_capacity(n);
        let mut radii = Vec::with_capacity(n);
        for _ in 0..n {
            cx.push(rng.next_f32() * 200.0 - 100.0);
            cy.push(rng.next_f32() * 200.0 - 100.0);
            cz.push(rng.next_f32() * 200.0 - 100.0);
            radii.push(rng.next_f32() * 4.0);
        }
        let mut base = Vec::with_capacity(n);
        let mut coarse = Vec::with_capacity(n);
        for _ in 0..n {
            base.push(rng.next_f32() * 40.0 - 20.0);
            coarse.push(rng.next_f32() * 40.0 - 20.0);
        }
        let blend = 0.5f32;

        let mut cull_paths: Vec<(&'static str, Box<dyn Fn(&mut Vec<u32>)>)> = Vec::new();
        let mut morph_paths: Vec<(&'static str, Box<dyn Fn(&mut [f32])>)> = Vec::new();
        macro_rules! add_paths {
            ($name:expr, $cull:ident, $morph:ident) => {{
                let (cx, cy, cz, radii, planes, base, coarse) =
                    (cx.clone(), cy.clone(), cz.clone(), radii.clone(), planes, base.clone(), coarse.clone());
                cull_paths.push((
                    $name,
                    Box::new(move |out: &mut Vec<u32>| {
                        // safety: 已由调用方按硬件能力注册
                        unsafe {
                            Renderer::$cull(&cx, &cy, &cz, &radii, &planes, out);
                        }
                    }),
                ));
                morph_paths.push((
                    $name,
                    Box::new(move |out: &mut [f32]| {
                        // safety: 已由调用方按硬件能力注册
                        unsafe {
                            Renderer::$morph(&base, &coarse, blend, out);
                        }
                    }),
                ));
            }};
        }
        #[cfg(target_arch = "x86_64")]
        {
            if std::is_x86_feature_detected!("avx512f") {
                add_paths!("avx512", cull_spheres_avx512, morph_heights_avx512);
            }
            if std::is_x86_feature_detected!("avx2") {
                add_paths!("avx2", cull_spheres_avx2, morph_heights_avx2);
            }
            if std::is_x86_feature_detected!("avx") {
                add_paths!("avx", cull_spheres_avx, morph_heights_avx);
            }
            if std::is_x86_feature_detected!("sse4.2") {
                add_paths!("sse4.2", cull_spheres_sse, morph_heights_sse);
            }
        }
        {
            let (cx, cy, cz, radii, planes, base, coarse) =
                (cx.clone(), cy.clone(), cz.clone(), radii.clone(), planes, base.clone(), coarse.clone());
            cull_paths.push((
                "scalar",
                Box::new(move |out: &mut Vec<u32>| {
                    Renderer::cull_spheres_scalar(&cx, &cy, &cz, &radii, &planes, out);
                }),
            ));
            morph_paths.push((
                "scalar",
                Box::new(move |out: &mut [f32]| {
                    Renderer::morph_heights_scalar(&base, &coarse, blend, out);
                }),
            ));
        }

        let rounds = 200u32;
        let mut cull_out: Vec<Vec<u32>> = cull_paths.iter().map(|_| Vec::new()).collect();
        let mut morph_out: Vec<Vec<f32>> = morph_paths.iter().map(|_| vec![0.0f32; n]).collect();
        let bench = |paths: &[(&'static str, Box<dyn Fn(&mut Vec<u32>)>)],
                     outs: &mut [Vec<u32>]| -> Vec<u64> {
            let mut us = vec![0u64; paths.len()];
            for (i, (_, f)) in paths.iter().enumerate() {
                f(&mut outs[i]);
            }
            for (i, (_, f)) in paths.iter().enumerate() {
                let t0 = std::time::Instant::now();
                for _ in 0..rounds {
                    f(&mut outs[i]);
                    std::hint::black_box(&outs[i]);
                }
                us[i] = t0.elapsed().as_micros() as u64 / rounds as u64;
            }
            us
        };
        // 剔除基准
        let cull_us = bench(&cull_paths, &mut cull_out);
        let scalar_cull = *cull_us.last().unwrap();
        // morph 基准
        let mut morph_us = vec![0u64; morph_paths.len()];
        for (i, (_, f)) in morph_paths.iter().enumerate() {
            f(&mut morph_out[i]);
        }
        for (i, (_, f)) in morph_paths.iter().enumerate() {
            let t0 = std::time::Instant::now();
            for _ in 0..rounds {
                f(&mut morph_out[i]);
                std::hint::black_box(&morph_out[i]);
            }
            morph_us[i] = t0.elapsed().as_micros() as u64 / rounds as u64;
        }
        let scalar_morph = *morph_us.last().unwrap();
        // 逐位一致性（与各自标量对照）
        for (i, (name, _)) in cull_paths.iter().enumerate() {
            assert_eq!(&cull_out[i], cull_out.last().unwrap(), "{} 剔除与标量不一致", name);
        }
        for (i, (name, _)) in morph_paths.iter().enumerate() {
            assert_eq!(&morph_out[i], morph_out.last().unwrap(), "{} morph 与标量不一致", name);
        }
        println!("\n== cull SIMD 微基准（{} 实例 × {} 轮，release，单线程） ==", n, rounds);
        println!("{:<8}{:>14}{:>10}", "path", "us/round", "speedup");
        for (i, (name, _)) in cull_paths.iter().enumerate() {
            println!(
                "{:<8}{:>14}{:>9.2}x",
                name,
                cull_us[i],
                scalar_cull as f64 / cull_us[i].max(1) as f64
            );
        }
        println!("\n== morph SIMD 微基准（{} 点 × {} 轮，release，单线程） ==", n, rounds);
        println!("{:<8}{:>14}{:>10}", "path", "us/round", "speedup");
        for (i, (name, _)) in morph_paths.iter().enumerate() {
            println!(
                "{:<8}{:>14}{:>9.2}x",
                name,
                morph_us[i],
                scalar_morph as f64 / morph_us[i].max(1) as f64
            );
        }
    }
}

// ============================================================
// NPC 士兵可视化单元测试
// ============================================================

#[cfg(test)]
mod npc_visual_tests {
    use super::*;

    /// 读取列主序 model 数组的平移分量（model[12..15]）
    fn translation(m: &InstanceData) -> [f32; 3] {
        [m.model[12], m.model[13], m.model[14]]
    }

    /// 三几何分组：盒 9（脚×2/骨盆/胸廓/背心/头/头盔/枪身/枪托）+ 圆柱 8（四肢）= 17。
    /// **改 `soldier_part_matrices` 的身体计划就要同步改这里** —— 它是身体计划的守卫。
    #[test]
    fn soldier_parts_count_and_tint() {
        let tint = [0.2, 0.6, 0.9, 1.0];
        let (box_parts, cyl_parts, sph_parts) =
            Renderer::soldier_part_matrices([0.0, 0.0, 0.0], 0.0, tint, 0.0, false, false);
        assert_eq!(
            box_parts.len(),
            10,
            "盒体段应为 10（脚×2/骨盆/胸廓/背心/头/头盔/枪身/枪托/背包）"
        );
        assert_eq!(cyl_parts.len(), 8, "圆柱段应为 8（四肢×8）");
        assert_eq!(sph_parts.len(), 0, "不再有球体段：球头已被「方块头 + 头盔壳」取代");
        assert_eq!(box_parts.len() + cyl_parts.len() + sph_parts.len(), 18);
        // 段数预算守卫：压力模式 255 人 × 每组 3072 ⇒ 每组每人最多 12 段。
        // 没有这条断言，下次给士兵加装备就会在 255 人时静默截断（超出的段直接不画）。
        // 2026-09-12 加了背包（盒 9→10）：10×255 = 2550/3072 = 83%，仍留 17% 余量。
        // **上限 12 段**：再加盒段前先看这条断言会不会红。
        const NPC_HEADS: usize = 255;
        const PER_HEAD_MAX: usize = MAX_NPC_INSTANCES as usize / NPC_HEADS; // = 12
        assert_eq!(PER_HEAD_MAX, 12, "每组每人段数上限变了，下面的余量估算要重算");
        assert!(
            box_parts.len() <= PER_HEAD_MAX,
            "盒体段超预算：{} > {PER_HEAD_MAX}（{NPC_HEADS} 人时会静默截断）",
            box_parts.len()
        );
        assert!(
            cyl_parts.len() <= PER_HEAD_MAX,
            "圆柱段超预算：{} > {PER_HEAD_MAX}",
            cyl_parts.len()
        );
        // 逐段明暗（2026-09-12 第④条）：`tint` 不再是同一个值，而是**队色 × 本段系数**。
        // 旧断言是"所有段 tint == 队色"，它锁定的正是"整个人是一块均匀饱和色"那个缺陷
        // （实机放大图上就是一团橙色塑料）。改成锁四条不变量：
        //   ① alpha 不变；② 各通道不越界（∈ [0, 队色]）；
        //   ③ 色相比例不变（只许等比缩放，否则阵营色语义会漂移）；
        //   ④ 至少一段保持**完整队色**（远距离认阵营），且确实存在层次（不能全等）。
        let mut saw_full = false;
        let mut saw_variation = false;
        for p in box_parts.iter().chain(cyl_parts.iter()).chain(sph_parts.iter()) {
            assert_eq!(p.tint[3], tint[3], "段色 alpha 必须保持队色");
            for c in 0..3 {
                assert!(
                    p.tint[c] >= 0.0 && p.tint[c] <= tint[c] + 1e-6,
                    "段色越界：{:?} vs 队色 {:?}",
                    p.tint,
                    tint
                );
                // 比例不变：以通道 0 交叉相乘，避免除零
                let lhs = p.tint[c] * tint[0];
                let rhs = tint[c] * p.tint[0];
                assert!(
                    (lhs - rhs).abs() < 1e-5,
                    "段色改变了色相比例：{:?} vs 队色 {:?}",
                    p.tint,
                    tint
                );
            }
            if (p.tint[0] - tint[0]).abs() < 1e-6 {
                saw_full = true;
            } else {
                saw_variation = true;
            }
        }
        assert!(saw_full, "至少要有一段用完整队色，否则远距离认不出阵营");
        assert!(saw_variation, "逐段明暗必须有层次：全部相等就回到了'一块塑料'");
    }

    #[test]
    fn soldier_torso_height() {
        let (box_parts, _, _) =
            Renderer::soldier_part_matrices([0.0, 0.0, 0.0], 0.0, [1.0; 4], 0.0, false, false);
        // 盒体组第 4 段 = 胸廓（枢轴 y=1.26 + 段心 -0.01），yaw=0 时平移 y = 1.25
        let t = translation(&box_parts[3]);
        assert!(
            (t[1] - 1.25).abs() < 1e-3,
            "胸廓 y 应为 1.25，实际 {}",
            t[1]
        );
        assert!(t[0].abs() < 1e-3);
        assert!((t[2] + 0.01).abs() < 1e-3, "胸廓 z 应为 -0.01，实际 {}", t[2]);
    }

    /// 身体计划的**退化缩放守卫**：逐段检查实例矩阵三个基向量的长度。
    ///
    /// 存在的理由（2026-09-12 第 28 轮）：把游戏截图放大 3× 后发现士兵渲染成
    /// "一堆杂乱红方块 + 几条细长红色尖刺"。细到近乎 1px 的几何只可能来自**某一轴
    /// 缩放接近 0**。这条测试直接在矩阵上量，不必看图猜、也不必起游戏。
    ///
    /// `model` 列主序：列 i 占 `4i..4i+3`，所以三个基向量长度 = 三条轴的缩放。
    #[test]
    fn soldier_parts_have_no_degenerate_scale() {
        let (boxes, cyls, sphs) =
            Renderer::soldier_part_matrices([0.0; 3], 0.0, [1.0; 4], 0.0, false, false);
        let axis_len = |m: &[f32; 16], col: usize| -> f32 {
            let o = col * 4;
            (m[o] * m[o] + m[o + 1] * m[o + 1] + m[o + 2] * m[o + 2]).sqrt()
        };
        let mut bad: Vec<String> = Vec::new();
        for (i, p) in boxes.iter().enumerate() {
            let (sx, sy, sz) = (
                axis_len(&p.model, 0),
                axis_len(&p.model, 1),
                axis_len(&p.model, 2),
            );
            println!(
                "盒[{i}] scale=({sx:.3},{sy:.3},{sz:.3}) t=({:.2},{:.2},{:.2})",
                p.model[12], p.model[13], p.model[14]
            );
            if sx < 0.01 || sy < 0.01 || sz < 0.01 {
                bad.push(format!("盒[{i}]=({sx:.4},{sy:.4},{sz:.4})"));
            }
        }
        for (i, p) in cyls.iter().enumerate() {
            let (sx, sy, sz) = (
                axis_len(&p.model, 0),
                axis_len(&p.model, 1),
                axis_len(&p.model, 2),
            );
            println!(
                "柱[{i}] scale=({sx:.3},{sy:.3},{sz:.3}) t=({:.2},{:.2},{:.2})",
                p.model[12], p.model[13], p.model[14]
            );
            if sx < 0.01 || sy < 0.01 || sz < 0.01 {
                bad.push(format!("柱[{i}]=({sx:.4},{sy:.4},{sz:.4})"));
            }
        }
        for p in sphs.iter() {
            let _ = p;
        }
        assert!(
            bad.is_empty(),
            "存在退化缩放（某轴 ≈ 0，渲染出来就是尖刺）：{}",
            bad.join(", ")
        );
    }

    #[test]
    fn soldier_gun_rotates_with_yaw() {
        let (base, _, _) =
            Renderer::soldier_part_matrices([0.0, 0.0, 0.0], 0.0, [1.0; 4], 0.0, false, false);
        let (turned, _, _) = Renderer::soldier_part_matrices(
            [0.0, 0.0, 0.0],
            std::f32::consts::FRAC_PI_2,
            [1.0; 4],
            0.0,
            false,
            false,
        );
        // 盒体组第 8 段 = 枪身，局部 (x=+0.16, y=+1.18, z=+0.36)。
        // **不变式：枪指 +Z、并随 yaw 一起转。** 带横向偏移后，90° 把局部 (+0.16,+0.36)
        // 映到 (+0.36,-0.16) —— 所以 z 不再归零；旧表枪在 x=0，"z 应归零"当时成立纯属巧合，
        // 拿它当判据会把一个正确的旋转判错。
        let g0 = translation(&base[7]);
        let g90 = translation(&turned[7]);
        assert!((g0[0] - 0.16).abs() < 1e-3, "yaw=0 枪 x={}", g0[0]);
        assert!(
            (g0[2] - 0.36).abs() < 1e-3,
            "yaw=0 枪应伸向 +Z，z={}",
            g0[2]
        );
        assert!(
            (g90[0] - 0.36).abs() < 1e-3,
            "yaw=90° 枪应转到 +X，x={}",
            g90[0]
        );
        assert!(
            (g90[2] + 0.16).abs() < 1e-3,
            "yaw=90° 枪 z 应为 -0.16，z={}",
            g90[2]
        );
    }

    #[test]
    fn soldier_pos_translation_applies() {
        let (box_parts, _, _) = Renderer::soldier_part_matrices(
            [10.0, 2.0, -3.0],
            0.0,
            [1.0; 4],
            0.0,
            false,
            false,
        );
        // 盒体组第 8 段 = 枪身：平移 = pos + 局部 (0.16, 1.18, 0.36)
        let t = translation(&box_parts[7]);
        assert!((t[0] - 10.16).abs() < 1e-3, "x={}", t[0]);
        // 胸廓（盒体组第 4 段）：平移 = pos + (0, 1.25, -0.01)
        let tc = translation(&box_parts[3]);
        assert!((tc[1] - 3.25).abs() < 1e-3, "胸廓 y={}", tc[1]);
    }

    /// 尸体 15 段（14 段人体 + 横置枪），三几何分组，tint 保留
    #[test]
    fn dead_body_15_parts_with_tint() {
        let (box_parts, cyl_parts, sph_parts) =
            Renderer::dead_part_matrices([1.0, 0.0, 2.0], 0.0, [0.9, 0.1, 0.1, 1.0]);
        assert_eq!(box_parts.len(), 4, "尸体盒体段应为 4（脚×2/颈/枪）");
        assert_eq!(cyl_parts.len(), 10, "尸体圆柱段应为 10（四肢×8/骨盆/胸）");
        assert_eq!(sph_parts.len(), 1, "尸体球体段应为 1");
        assert_eq!(box_parts.len() + cyl_parts.len() + sph_parts.len(), 15);
        for p in box_parts.iter().chain(cyl_parts.iter()).chain(sph_parts.iter()) {
            assert_eq!(p.tint, [0.9, 0.1, 0.1, 1.0]);
        }
    }
}

/// `assets/mesh.spv` 的 **Workgroup 显式布局**回归守卫（2026-09-15）。
///
/// ## 守的是什么
///
/// `MeshShadingEXT` 要求 SPIR-V >= 1.4，而 `Offset` / `ArrayStride` 这类**显式布局装饰**
/// 在 SPIR-V ≤ 1.3 允许、**1.4 起对非 `Block` 类型禁止**。naga 30 的 SPIR-V 写入器
/// （`src/back/spv/writer.rs` 的 `decorate_struct_member`）**无条件**写 `Offset`，
/// 于是网格着色器一度是 7 个 `.spv` 里**唯一**过不了严格校验的那个：
///
/// ```text
/// spirv-val --target-env vulkan1.3 assets/mesh.spv
/// [VUID-StandaloneSpirv-None-10684] the Workgroup storage class has a explicit layout
/// from the Offset decoration
/// ```
///
/// `build.rs::strip_workgroup_explicit_layout` 在写出前把它去掉。这条测试**独立复算**
/// 一遍"从 Workgroup 变量出发的类型可达闭包"，所以下面三种情况都会让它变红：
/// ① 有人把 build.rs 里那次调用删了；② naga 升级后又多写了别的显式布局装饰；
/// ③ 网格着色器 WGSL 改出新形态的 Workgroup 类型。
///
/// ## 为什么"去掉"对 Workgroup 是无损的
///
/// Workgroup 内存**主机侧永远不碰**（`renderer.rs` 一级的实例/光照 buffer 都是
/// Uniform/StorageBuffer，那些带 `Block`、装饰**必须原样保留**），着色器访问一律走
/// `OpAccessChain` 的**成员索引**，字节偏移由驱动按 std430 自行推导 —— 只要同一个
/// 着色器内部一致，偏移取多少都不影响结果。反过来，动到带 `Block` 的类型上就是
/// **缓冲布局错位**，所以这条测试也顺带断言了那几个 `Block` 类型仍有 `Offset`。
#[cfg(test)]
mod workgroup_layout_tests {
    use std::collections::{HashMap, HashSet};

    const OP_TYPE_ARRAY: u32 = 28;
    const OP_TYPE_RUNTIME_ARRAY: u32 = 29;
    const OP_TYPE_STRUCT: u32 = 30;
    const OP_TYPE_POINTER: u32 = 32;
    const OP_VARIABLE: u32 = 59;
    const OP_DECORATE: u32 = 71;
    const OP_MEMBER_DECORATE: u32 = 72;
    const DECORATION_BLOCK: u32 = 2;
    const DECORATION_OFFSET: u32 = 35;
    const STORAGE_WORKGROUP: u32 = 4;
    /// `RowMajor` / `ColMajor` / `ArrayStride` / `MatrixStride` / `Offset`
    const EXPLICIT_LAYOUT: [u32; 5] = [4, 5, 6, 7, 35];

    fn words_of_mesh_spv() -> Vec<u32> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mesh.spv");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("读 {} 失败: {e}", path.display()));
        assert_eq!(bytes.len() % 4, 0, "SPIR-V 字节数必须是 4 的倍数");
        bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    struct Scan {
        /// 从 Workgroup 变量出发可达的类型
        workgroup_reachable: HashSet<u32>,
        /// 可达类型上的显式布局装饰（`(类型, 装饰)`）
        workgroup_layout: Vec<(u32, u32)>,
        /// 带 `Block` 装饰的类型
        block_types: HashSet<u32>,
        /// 带 `Block` 的类型的 `Offset` 装饰条数
        block_offsets: usize,
    }

    fn scan(words: &[u32]) -> Scan {
        assert_eq!(words[0], 0x0723_0203, "不是 SPIR-V 字流（魔数不符）");
        let mut structs: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut arrays: HashMap<u32, u32> = HashMap::new();
        let mut pointers: HashMap<u32, (u32, u32)> = HashMap::new();
        let mut seeds: Vec<u32> = Vec::new();
        let mut block_types: HashSet<u32> = HashSet::new();
        let mut decorations: Vec<(u32, u32, Option<u32>)> = Vec::new(); // (目标, 装饰, 成员)

        let mut i = 5usize;
        while i < words.len() {
            let count = (words[i] >> 16) as usize;
            let op = words[i] & 0xFFFF;
            let w = &words[i..i + count];
            match op {
                OP_TYPE_STRUCT => {
                    structs.insert(w[1], w[2..].to_vec());
                }
                OP_TYPE_ARRAY | OP_TYPE_RUNTIME_ARRAY => {
                    arrays.insert(w[1], w[2]);
                }
                OP_TYPE_POINTER => {
                    pointers.insert(w[1], (w[2], w[3]));
                }
                OP_VARIABLE if w[3] == STORAGE_WORKGROUP => {
                    if let Some(&(storage, pointee)) = pointers.get(&w[1]) {
                        if storage == STORAGE_WORKGROUP {
                            seeds.push(pointee);
                        }
                    }
                }
                OP_DECORATE => {
                    if w[2] == DECORATION_BLOCK {
                        block_types.insert(w[1]);
                    }
                    decorations.push((w[1], w[2], None));
                }
                OP_MEMBER_DECORATE => decorations.push((w[1], w[3], Some(w[2]))),
                _ => {}
            }
            i += count;
        }

        let mut reachable: HashSet<u32> = HashSet::new();
        let mut stack = seeds;
        while let Some(ty) = stack.pop() {
            if !reachable.insert(ty) {
                continue;
            }
            if let Some(members) = structs.get(&ty) {
                stack.extend(members.iter().copied());
            }
            if let Some(&elem) = arrays.get(&ty) {
                stack.push(elem);
            }
        }

        let mut workgroup_layout = Vec::new();
        let mut block_offsets = 0usize;
        for (target, decoration, _member) in decorations {
            if reachable.contains(&target) && EXPLICIT_LAYOUT.contains(&decoration) {
                workgroup_layout.push((target, decoration));
            }
            if block_types.contains(&target) && decoration == DECORATION_OFFSET {
                block_offsets += 1;
            }
        }
        Scan { workgroup_reachable: reachable, workgroup_layout, block_types, block_offsets }
    }

    /// 主语：网格着色器不得带任何 Workgroup 显式布局装饰（否则严格 `spirv-val` 拒载）。
    #[test]
    fn mesh_spirv_has_no_workgroup_explicit_layout() {
        let words = words_of_mesh_spv();
        let s = scan(&words);
        assert!(
            !s.workgroup_reachable.is_empty(),
            "mesh.spv 里找不到 Workgroup 变量 —— 网格着色器结构变了，这条守卫已失效，必须重写"
        );
        assert!(
            s.workgroup_layout.is_empty(),
            "mesh.spv 的 Workgroup 类型上仍有 {} 条显式布局装饰（类型/装饰：{:?}）。\n\
             修法：确认 build.rs::compile_wgsl_mesh 仍调用 strip_workgroup_explicit_layout；\n\
             若是 naga 新增了别的装饰种类，把它加进 build.rs 的 EXPLICIT_LAYOUT。\n\
             验证命令：spirv-val --target-env vulkan1.3 assets/mesh.spv",
            s.workgroup_layout.len(),
            s.workgroup_layout
        );
    }

    /// 反面：去掉装饰**只**能碰 Workgroup 可达类型。带 `Block` 的 Uniform / StorageBuffer /
    /// PushConstant 一旦被动，主机侧按同一份偏移写入的数据全部错位（静默、不报 VUID）。
    #[test]
    fn block_types_keep_their_offsets() {
        let words = words_of_mesh_spv();
        let s = scan(&words);
        assert!(
            !s.block_types.is_empty(),
            "mesh.spv 里找不到 Block 类型 —— 这条反向守卫已失效"
        );
        assert!(
            s.block_offsets > 0,
            "带 Block 的类型（{:?}）的 Offset 装饰被去掉了 —— 缓冲布局会错位",
            s.block_types
        );
        for ty in &s.block_types {
            assert!(
                !s.workgroup_reachable.contains(ty),
                "Block 类型 {ty} 同时是 Workgroup 可达的 —— 去掉规则会误伤它，必须先改 build.rs 的取舍"
            );
        }
    }
}

