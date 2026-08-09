//! 战斗特效 SIMD 加速模块：爆炸冲击波压力场等纯浮点批量计算。
//!
//! 主玩法当前没有爆炸/冲击波系统，本模块默认不参与游戏逻辑：
//! - `RV3D_EXPLOSION_SIM=1`：每帧推进一个 4096 采样点的小规模波前压力场
//!   （纯 CPU 压力模拟，不产生画面/伤害，安全、确定性），1Hz 输出各指令集
//!   路径耗时与对标量的加速比，用于实测 AVX-512/AVX2 在爆炸类浮点运算上的收益；
//! - 选路策略与 renderer 剔除一致：AVX-512 > AVX2 > AVX > SSE4.2 > NEON > 标量，
//!   各路径与标量逐位一致（非 FMA，运算顺序相同）。
//!
//! 简化物理模型（波前压力场）：
//!   p(x) = A / (1 + k·d²) × max(0, 1 − d/R)
//!   - A：爆源幅度；R：冲击波半径；k = 1/R²；d：采样点到爆心距离
//!   - 单调递减、有限输入下无 NaN/Inf（d ≥ 0 且 A/k/R 有限）
//!
//! 采样点以 `[f32; 3]`（AoS，12B 步长）传入：x86 用 gather 按分量取数、
//! ARM 用 `vld3q_f32` 结构步长加载，与标量逐位一致。

/// 冲击波压力场选路入口：对每个采样点计算压力写入 `out`，返回实际启用的指令集路径名。
/// x86_64：AVX-512（16 点/批）> AVX2（8）> AVX（8）> SSE4.2（4）> 标量；
/// aarch64：NEON（4）> 标量；其余平台：标量。
/// ★ AVX-512 说明：Zen4/Zen5（7000/9000 系）双 256 单元合并执行 512 位请求；
///   选路走 `cpu::avx512_enabled()`——硬件不支持、RV3D_DISABLE_AVX512=1、
///   Intel 11 代（能效差）与 12 代起（大小核）自动禁用回退。
pub fn shockwave_pressure(
    center: [f32; 3],
    radius: f32,
    amplitude: f32,
    points: &[[f32; 3]],
    out: &mut [f32],
) -> &'static str {
    debug_assert_eq!(points.len(), out.len());
    #[cfg(target_arch = "x86_64")]
    {
        if crate::engine::cpu::avx512_enabled() {
            // safety: 上面已运行时检测 AVX-512（含 Intel 11/12 代型号过滤）
            unsafe {
                shockwave_pressure_avx512(center, radius, amplitude, points, out);
            }
            return "avx512";
        }
        if std::is_x86_feature_detected!("avx2") {
            // safety: 上面已运行时检测 AVX2
            unsafe {
                shockwave_pressure_avx2(center, radius, amplitude, points, out);
            }
            return "avx2";
        }
        if std::is_x86_feature_detected!("avx") {
            // safety: 上面已运行时检测 AVX
            unsafe {
                shockwave_pressure_avx(center, radius, amplitude, points, out);
            }
            return "avx";
        }
        if std::is_x86_feature_detected!("sse4.2") {
            // safety: 上面已运行时检测 SSE4.2
            unsafe {
                shockwave_pressure_sse(center, radius, amplitude, points, out);
            }
            return "sse4.2";
        }
        shockwave_pressure_scalar(center, radius, amplitude, points, out);
        "scalar"
    }
    #[cfg(target_arch = "aarch64")]
    {
        if std::arch::is_aarch64_feature_detected!("neon") {
            // safety: NEON 在 AArch64 是基线特性（此处仍运行时确认）
            unsafe {
                shockwave_pressure_neon(center, radius, amplitude, points, out);
            }
            return "neon";
        }
        shockwave_pressure_scalar(center, radius, amplitude, points, out);
        "scalar"
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        shockwave_pressure_scalar(center, radius, amplitude, points, out);
        "scalar"
    }
}

/// 标量冲击波压力（基准语义，供指令集路径逐位对照与加速比测量）：
/// p = A/(1+k·d²) × max(0, 1−d/R)；d² = ((dx²+dy²)+dz²)，运算顺序固定。
pub fn shockwave_pressure_scalar(
    center: [f32; 3],
    radius: f32,
    amplitude: f32,
    points: &[[f32; 3]],
    out: &mut [f32],
) {
    let k = 1.0 / (radius * radius);
    let a = amplitude;
    let r = radius;
    for i in 0..points.len() {
        let dx = points[i][0] - center[0];
        let dy = points[i][1] - center[1];
        let dz = points[i][2] - center[2];
        let d2 = (dx * dx + dy * dy) + dz * dz;
        let d = d2.sqrt();
        let falloff = if d < r { 1.0 - d / r } else { 0.0 };
        out[i] = a / (1.0 + k * d2) * falloff;
    }
}

/// AVX-512 冲击波压力：16 点/批。采样点 [f32;3] 步长 12B，gather（索引单位=4B）
/// 按 x/y/z 分量取数；运算顺序与标量逐位一致（无 FMA；sqrt/div 均 IEEE 正确舍入）。
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn shockwave_pressure_avx512(
    center: [f32; 3],
    radius: f32,
    amplitude: f32,
    points: &[[f32; 3]],
    out: &mut [f32],
) {
    use std::arch::x86_64::*;
    let c0 = _mm512_set1_ps(center[0]);
    let c1 = _mm512_set1_ps(center[1]);
    let c2 = _mm512_set1_ps(center[2]);
    let rv = _mm512_set1_ps(radius);
    let av = _mm512_set1_ps(amplitude);
    let kv = _mm512_set1_ps(1.0 / (radius * radius));
    let one = _mm512_set1_ps(1.0);
    let base = points.as_ptr() as *const f32;
    let ix0 = _mm512_setr_epi32(0, 3, 6, 9, 12, 15, 18, 21, 24, 27, 30, 33, 36, 39, 42, 45);
    let iy0 = _mm512_add_epi32(ix0, _mm512_set1_epi32(1));
    let iz0 = _mm512_add_epi32(ix0, _mm512_set1_epi32(2));
    let mut i = 0usize;
    while i + 16 <= points.len() {
        let p = base.add(i * 3);
        let px = _mm512_i32gather_ps::<4>(ix0, p);
        let py = _mm512_i32gather_ps::<4>(iy0, p);
        let pz = _mm512_i32gather_ps::<4>(iz0, p);
        let dx = _mm512_sub_ps(px, c0);
        let dy = _mm512_sub_ps(py, c1);
        let dz = _mm512_sub_ps(pz, c2);
        let d2 = _mm512_add_ps(
            _mm512_add_ps(_mm512_mul_ps(dx, dx), _mm512_mul_ps(dy, dy)),
            _mm512_mul_ps(dz, dz),
        );
        let d = _mm512_sqrt_ps(d2);
        // falloff = d < r ? 1 - d/r : 0（NaN 时比较为 false → 0，与标量一致）
        let inside = _mm512_cmp_ps_mask(d, rv, _CMP_LT_OQ);
        let falloff = _mm512_mask_blend_ps(
            inside,
            _mm512_setzero_ps(),
            _mm512_sub_ps(one, _mm512_div_ps(d, rv)),
        );
        // p = a / (1 + k·d2) × falloff
        let p = _mm512_mul_ps(
            _mm512_div_ps(av, _mm512_add_ps(one, _mm512_mul_ps(kv, d2))),
            falloff,
        );
        _mm512_storeu_ps(out.as_mut_ptr().add(i), p);
        i += 16;
    }
    // 尾部不足 16 个走标量
    for j in i..points.len() {
        let dx = points[j][0] - center[0];
        let dy = points[j][1] - center[1];
        let dz = points[j][2] - center[2];
        let d2 = (dx * dx + dy * dy) + dz * dz;
        let d = d2.sqrt();
        let falloff = if d < radius { 1.0 - d / radius } else { 0.0 };
        out[j] = amplitude / (1.0 + (1.0 / (radius * radius)) * d2) * falloff;
    }
}

/// AVX2 冲击波压力：8 点/批（gather 按 [f32;3] 步长取数；与标量逐位一致，无 FMA）
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn shockwave_pressure_avx2(
    center: [f32; 3],
    radius: f32,
    amplitude: f32,
    points: &[[f32; 3]],
    out: &mut [f32],
) {
    use std::arch::x86_64::*;
    let c0 = _mm256_set1_ps(center[0]);
    let c1 = _mm256_set1_ps(center[1]);
    let c2 = _mm256_set1_ps(center[2]);
    let rv = _mm256_set1_ps(radius);
    let av = _mm256_set1_ps(amplitude);
    let kv = _mm256_set1_ps(1.0 / (radius * radius));
    let one = _mm256_set1_ps(1.0);
    let base = points.as_ptr() as *const f32;
    let ix0 = _mm256_setr_epi32(0, 3, 6, 9, 12, 15, 18, 21);
    let iy0 = _mm256_add_epi32(ix0, _mm256_set1_epi32(1));
    let iz0 = _mm256_add_epi32(ix0, _mm256_set1_epi32(2));
    let mut i = 0usize;
    while i + 8 <= points.len() {
        let p = base.add(i * 3);
        let px = _mm256_i32gather_ps::<4>(p, ix0);
        let py = _mm256_i32gather_ps::<4>(p, iy0);
        let pz = _mm256_i32gather_ps::<4>(p, iz0);
        let dx = _mm256_sub_ps(px, c0);
        let dy = _mm256_sub_ps(py, c1);
        let dz = _mm256_sub_ps(pz, c2);
        let d2 = _mm256_add_ps(
            _mm256_add_ps(_mm256_mul_ps(dx, dx), _mm256_mul_ps(dy, dy)),
            _mm256_mul_ps(dz, dz),
        );
        let d = _mm256_sqrt_ps(d2);
        let inside = _mm256_cmp_ps(d, rv, _CMP_LT_OQ);
        let falloff = _mm256_blendv_ps(
            _mm256_setzero_ps(),
            _mm256_sub_ps(one, _mm256_div_ps(d, rv)),
            inside,
        );
        let p = _mm256_mul_ps(
            _mm256_div_ps(av, _mm256_add_ps(one, _mm256_mul_ps(kv, d2))),
            falloff,
        );
        _mm256_storeu_ps(out.as_mut_ptr().add(i), p);
        i += 8;
    }
    for j in i..points.len() {
        let dx = points[j][0] - center[0];
        let dy = points[j][1] - center[1];
        let dz = points[j][2] - center[2];
        let d2 = (dx * dx + dy * dy) + dz * dz;
        let d = d2.sqrt();
        let falloff = if d < radius { 1.0 - d / radius } else { 0.0 };
        out[j] = amplitude / (1.0 + (1.0 / (radius * radius)) * d2) * falloff;
    }
}

/// AVX（非 AVX2，3/4 代酷睿与初代锐龙）冲击波压力：8 点/批。
/// AVX 无 gather：标量按 [f32;3] 步长取 8 点后再向量化运算（与标量逐位一致）。
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn shockwave_pressure_avx(
    center: [f32; 3],
    radius: f32,
    amplitude: f32,
    points: &[[f32; 3]],
    out: &mut [f32],
) {
    use std::arch::x86_64::*;
    let c0 = _mm256_set1_ps(center[0]);
    let c1 = _mm256_set1_ps(center[1]);
    let c2 = _mm256_set1_ps(center[2]);
    let rv = _mm256_set1_ps(radius);
    let av = _mm256_set1_ps(amplitude);
    let kv = _mm256_set1_ps(1.0 / (radius * radius));
    let one = _mm256_set1_ps(1.0);
    let base = points.as_ptr() as *const f32;
    let mut i = 0usize;
    while i + 8 <= points.len() {
        let p = base.add(i * 3);
        let mut pxv = [0f32; 8];
        let mut pyv = [0f32; 8];
        let mut pzv = [0f32; 8];
        for k in 0..8 {
            pxv[k] = *p.add(k * 3);
            pyv[k] = *p.add(k * 3 + 1);
            pzv[k] = *p.add(k * 3 + 2);
        }
        let px = _mm256_loadu_ps(pxv.as_ptr());
        let py = _mm256_loadu_ps(pyv.as_ptr());
        let pz = _mm256_loadu_ps(pzv.as_ptr());
        let dx = _mm256_sub_ps(px, c0);
        let dy = _mm256_sub_ps(py, c1);
        let dz = _mm256_sub_ps(pz, c2);
        let d2 = _mm256_add_ps(
            _mm256_add_ps(_mm256_mul_ps(dx, dx), _mm256_mul_ps(dy, dy)),
            _mm256_mul_ps(dz, dz),
        );
        let d = _mm256_sqrt_ps(d2);
        let inside = _mm256_cmp_ps(d, rv, _CMP_LT_OQ);
        let falloff = _mm256_blendv_ps(
            _mm256_setzero_ps(),
            _mm256_sub_ps(one, _mm256_div_ps(d, rv)),
            inside,
        );
        let p = _mm256_mul_ps(
            _mm256_div_ps(av, _mm256_add_ps(one, _mm256_mul_ps(kv, d2))),
            falloff,
        );
        _mm256_storeu_ps(out.as_mut_ptr().add(i), p);
        i += 8;
    }
    for j in i..points.len() {
        let dx = points[j][0] - center[0];
        let dy = points[j][1] - center[1];
        let dz = points[j][2] - center[2];
        let d2 = (dx * dx + dy * dy) + dz * dz;
        let d = d2.sqrt();
        let falloff = if d < radius { 1.0 - d / radius } else { 0.0 };
        out[j] = amplitude / (1.0 + (1.0 / (radius * radius)) * d2) * falloff;
    }
}

/// SSE4.2 冲击波压力：4 点/批（2008 年后所有 Intel/AMD 消费级）。
/// SSE 无 gather：标量按 [f32;3] 步长取 4 点后再向量化运算。
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
unsafe fn shockwave_pressure_sse(
    center: [f32; 3],
    radius: f32,
    amplitude: f32,
    points: &[[f32; 3]],
    out: &mut [f32],
) {
    use std::arch::x86_64::*;
    let c0 = _mm_set1_ps(center[0]);
    let c1 = _mm_set1_ps(center[1]);
    let c2 = _mm_set1_ps(center[2]);
    let rv = _mm_set1_ps(radius);
    let av = _mm_set1_ps(amplitude);
    let kv = _mm_set1_ps(1.0 / (radius * radius));
    let one = _mm_set1_ps(1.0);
    let base = points.as_ptr() as *const f32;
    let mut i = 0usize;
    while i + 4 <= points.len() {
        let p = base.add(i * 3);
        let mut pxv = [0f32; 4];
        let mut pyv = [0f32; 4];
        let mut pzv = [0f32; 4];
        for k in 0..4 {
            pxv[k] = *p.add(k * 3);
            pyv[k] = *p.add(k * 3 + 1);
            pzv[k] = *p.add(k * 3 + 2);
        }
        let px = _mm_loadu_ps(pxv.as_ptr());
        let py = _mm_loadu_ps(pyv.as_ptr());
        let pz = _mm_loadu_ps(pzv.as_ptr());
        let dx = _mm_sub_ps(px, c0);
        let dy = _mm_sub_ps(py, c1);
        let dz = _mm_sub_ps(pz, c2);
        let d2 = _mm_add_ps(
            _mm_add_ps(_mm_mul_ps(dx, dx), _mm_mul_ps(dy, dy)),
            _mm_mul_ps(dz, dz),
        );
        let d = _mm_sqrt_ps(d2);
        let inside = _mm_cmp_ps(d, rv, _CMP_LT_OQ);
        let falloff = _mm_blendv_ps(
            _mm_setzero_ps(),
            _mm_sub_ps(one, _mm_div_ps(d, rv)),
            inside,
        );
        let p = _mm_mul_ps(
            _mm_div_ps(av, _mm_add_ps(one, _mm_mul_ps(kv, d2))),
            falloff,
        );
        _mm_storeu_ps(out.as_mut_ptr().add(i), p);
        i += 4;
    }
    for j in i..points.len() {
        let dx = points[j][0] - center[0];
        let dy = points[j][1] - center[1];
        let dz = points[j][2] - center[2];
        let d2 = (dx * dx + dy * dy) + dz * dz;
        let d = d2.sqrt();
        let falloff = if d < radius { 1.0 - d / radius } else { 0.0 };
        out[j] = amplitude / (1.0 + (1.0 / (radius * radius)) * d2) * falloff;
    }
}

/// NEON（AArch64，Apple Silicon/Android/高通 X Elite）冲击波压力：4 点/批。
/// `vld3q_f32` 按 [f32;3] 结构步长一次取 4 点的 x/y/z 分量，与标量加载逐位一致。
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn shockwave_pressure_neon(
    center: [f32; 3],
    radius: f32,
    amplitude: f32,
    points: &[[f32; 3]],
    out: &mut [f32],
) {
    use std::arch::aarch64::*;
    let c0 = vdupq_n_f32(center[0]);
    let c1 = vdupq_n_f32(center[1]);
    let c2 = vdupq_n_f32(center[2]);
    let rv = vdupq_n_f32(radius);
    let av = vdupq_n_f32(amplitude);
    let kv = vdupq_n_f32(1.0 / (radius * radius));
    let one = vdupq_n_f32(1.0);
    let zero = vdupq_n_f32(0.0);
    let mut i = 0usize;
    while i + 4 <= points.len() {
        let s = vld3q_f32(points.as_ptr().add(i));
        let px = s.0;
        let py = s.1;
        let pz = s.2;
        let dx = vsubq_f32(px, c0);
        let dy = vsubq_f32(py, c1);
        let dz = vsubq_f32(pz, c2);
        let d2 = vaddq_f32(
            vaddq_f32(vmulq_f32(dx, dx), vmulq_f32(dy, dy)),
            vmulq_f32(dz, dz),
        );
        let d = vsqrtq_f32(d2);
        let inside = vcltq_f32(d, rv);
        let falloff = vbslq_f32(inside, vsubq_f32(one, vdivq_f32(d, rv)), zero);
        let p = vmulq_f32(vdivq_f32(av, vaddq_f32(one, vmulq_f32(kv, d2))), falloff);
        vst1q_f32(out.as_mut_ptr().add(i), p);
        i += 4;
    }
    for j in i..points.len() {
        let dx = points[j][0] - center[0];
        let dy = points[j][1] - center[1];
        let dz = points[j][2] - center[2];
        let d2 = (dx * dx + dy * dy) + dz * dz;
        let d = d2.sqrt();
        let falloff = if d < radius { 1.0 - d / radius } else { 0.0 };
        out[j] = amplitude / (1.0 + (1.0 / (radius * radius)) * d2) * falloff;
    }
}

#[cfg(test)]
mod simd_shockwave_tests {
    use super::*;

    /// 简单确定性伪随机（SplitMix64），纯逻辑测试不碰 GPU
    struct Rng(u64);
    impl Rng {
        fn next_f32(&mut self) -> f32 {
            self.0 = self.0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as u32 as f64 / (1u64 << 31) as f64) as f32
        }
    }

    fn make_points(rng: &mut Rng, n: usize) -> Vec<[f32; 3]> {
        (0..n)
            .map(|_| {
                [
                    rng.next_f32() * 512.0 - 256.0,
                    rng.next_f32() * 40.0 - 20.0,
                    rng.next_f32() * 512.0 - 256.0,
                ]
            })
            .collect()
    }

    #[test]
    fn shockwave_all_paths_match_scalar() {
        // 覆盖各档批量倍数 + 尾部（33）+ 4096 点全场
        for n in [33usize, 4096usize] {
            let mut rng = Rng(0x5B0C_00 as u64 ^ n as u64);
            let points = make_points(&mut rng, n);
            let center = [rng.next_f32() * 60.0 - 30.0, 2.0, rng.next_f32() * 60.0 - 30.0];
            let radius = 40.0 + rng.next_f32() * 80.0;
            let amplitude = 500.0 + rng.next_f32() * 1500.0;

            let mut scalar_out = vec![0.0f32; n];
            shockwave_pressure_scalar(center, radius, amplitude, &points, &mut scalar_out);

            let mut out = vec![0.0f32; n];
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx512f") && crate::engine::cpu::avx512_enabled() {
                // safety: 已运行时检测 AVX-512 且未被型号过滤
                unsafe {
                    shockwave_pressure_avx512(center, radius, amplitude, &points, &mut out);
                }
                assert_eq!(out, scalar_out, "AVX-512 冲击波与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx2") {
                // safety: 已运行时检测 AVX2
                unsafe {
                    shockwave_pressure_avx2(center, radius, amplitude, &points, &mut out);
                }
                assert_eq!(out, scalar_out, "AVX2 冲击波与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx") {
                // safety: 已运行时检测 AVX
                unsafe {
                    shockwave_pressure_avx(center, radius, amplitude, &points, &mut out);
                }
                assert_eq!(out, scalar_out, "AVX 冲击波与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("sse4.2") {
                // safety: 已运行时检测 SSE4.2
                unsafe {
                    shockwave_pressure_sse(center, radius, amplitude, &points, &mut out);
                }
                assert_eq!(out, scalar_out, "SSE4.2 冲击波与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "aarch64")]
            if std::arch::is_aarch64_feature_detected!("neon") {
                // safety: NEON 在 AArch64 是基线特性（此处仍运行时确认）
                unsafe {
                    shockwave_pressure_neon(center, radius, amplitude, &points, &mut out);
                }
                assert_eq!(out, scalar_out, "NEON 冲击波与标量逐位不一致 (n={})", n);
            }
            // dispatch 选路（当前机器实际启用档位）也必须与标量逐位一致
            out.fill(0.0);
            let path = shockwave_pressure(center, radius, amplitude, &points, &mut out);
            assert_eq!(out, scalar_out, "dispatch({}) 冲击波与标量逐位不一致 (n={})", path, n);
        }
    }

    #[test]
    fn shockwave_physical_properties() {
        // 单调性/边界：爆心压力最大；R 外压力为 0；无 NaN/Inf
        let n = 64;
        let mut rng = Rng(0x5A17E_11);
        let points = make_points(&mut rng, n);
        let center = [0.0, 0.0, 0.0];
        let radius = 50.0;
        let amplitude = 1000.0;
        let mut out = vec![0.0f32; n];
        let _ = shockwave_pressure(center, radius, amplitude, &points, &mut out);
        for (p, v) in points.iter().zip(out.iter()) {
            let d = ((p[0] * p[0] + p[1] * p[1]) + p[2] * p[2]).sqrt();
            assert!(v.is_finite(), "压力必须是有限数: {}", v);
            assert!(*v >= 0.0 && *v <= amplitude, "压力应在 [0, A] 内: {}", v);
            if d >= radius {
                assert_eq!(*v, 0.0, "半径外压力应为 0 (d={})", d);
            }
        }
        // 中心点压力 = A（d=0 → p = A/(1+0) × 1）
        let mut c = vec![0.0f32; 1];
        let _ = shockwave_pressure(center, radius, amplitude, &[[0.0, 0.0, 0.0]], &mut c);
        assert!((c[0] - amplitude).abs() < 1e-3, "中心压力应等于幅度: {}", c[0]);
    }
}
