//! 程序化地面纹理 + 烘焙 AO / 静态天光（CPU 画像素，零第三方依赖）。
//!
//! 生成一张世界空间（覆盖 `2*WORLD_HALF` × `2*WORLD_HALF` 米）的地面材质纹理，
//! 每个 texel 合成：
//! 1. **材质基色**：草地 / 沙土 / 石板 / 道路 / 焦土弹坑（确定性值噪声分域，可辨识配色）；
//! 2. **烘焙 AO**：地形高度场凹度遮蔽（凹处暗、凸处亮，模拟光线遮挡）；
//! 3. **静态天光**：太阳方向漫反射 + 天光底（弱烘焙，与实时 Blinn-Phong 方向光/阴影不冲突）。
//!
//! 纹理与片元着色器的 world-space UV 严格对齐（见 `build.rs` FRAGMENT_SHADER_WGSL）：
//!   `uv = (world_pos.xz + WORLD_HALF) / (2 * WORLD_HALF)`
//! 即：texel 行 0（顶部）对应世界 `z = -WORLD_HALF`，texel 列 0 对应世界 `x = -WORLD_HALF`
//! （与 Vulkan 纹理坐标 uv.y=0 在图像顶部的约定一致，无额外翻转）。

/// 纹理覆盖的世界半宽（米）：纹理边长对应 `2*WORLD_HALF` 米的世界范围。
/// 必须与 `build.rs` 片元着色器里 world-space UV 的分母/偏移保持一致。
pub const WORLD_HALF: f32 = 256.0;

/// 默认纹理边长（覆盖 512×512 米，约 1 米/texel）。
pub const GROUND_TEXTURE_SIZE: u32 = 512;

/// 太阳方向（表面→光源），与 `game.rs::light_uniform` 的 `sun.direction` 一致。
const SUN_DIR: [f32; 3] = [-0.4, 0.9, -0.3];

/// 确定性种子（同种子恒同纹理，可测试）。
const DEFAULT_SEED: u32 = 0x5EED_FACE;

// ============================================================
// 确定性噪声（纯 u32 算术，跨平台逐位一致）
// ============================================================

/// 确定性整数哈希（格点噪声用；与 renderer 的 terrain_hash 同族但独立，避免耦合）
fn hash2(ix: i32, iz: i32, seed: u32) -> u32 {
    let mut h = (ix as u32)
        .wrapping_mul(0x1B873593)
        ^ (iz as u32).wrapping_mul(0xCC9E2D51)
        ^ seed.wrapping_mul(0x9E3779B9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846CA68B);
    h ^= h >> 16;
    h
}

/// Hermite 平滑阶跃
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// 平滑阶跃函数（edge0 < edge1）
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    smooth(t)
}

/// 格点伪随机高度：[-1, 1)
fn lattice(ix: i32, iz: i32, seed: u32) -> f32 {
    (hash2(ix, iz, seed) & 0xFFFF) as f32 / 32768.0 - 1.0
}

/// [0, 1) 确定性随机（取哈希高 24 位，避免低位周期过短）
fn unit_from_hash(h: u32) -> f32 {
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// 双线性 smoothstep 值噪声（确定性、低频平缓）
fn value_noise(x: f32, z: f32, cell: f32, seed: u32) -> f32 {
    let fx = x / cell;
    let fz = z / cell;
    let ix = fx.floor() as i32;
    let iz = fz.floor() as i32;
    let tx = smooth(fx - ix as f32);
    let tz = smooth(fz - iz as f32);
    let h00 = lattice(ix, iz, seed);
    let h10 = lattice(ix + 1, iz, seed);
    let h01 = lattice(ix, iz + 1, seed);
    let h11 = lattice(ix + 1, iz + 1, seed);
    let a = h00 + (h10 - h00) * tx;
    let b = h01 + (h11 - h01) * tx;
    a + (b - a) * tz
}

// ============================================================
// 颜色辅助（纯 [f32;3]，无第三方依赖）
// ============================================================

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len <= 1e-6 {
        [0.0, 1.0, 0.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

/// 线性 RGB → sRGB 编码（纹理为 R8G8B8A8_SRGB，硬件采样会做 sRGB→线性 decode，
/// 因此写入前必须编码，否则颜色被二次压暗、色调丢失）。
fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

// ============================================================
// 材质基色
// ============================================================

/// 弹坑焦土遮罩：24m 格点上确定性散布的圆形焦土斑（模拟炮击/爆炸痕迹）
fn crater_mask(x: f32, z: f32, seed: u32) -> f32 {
    let cell = 24.0;
    let ix = (x / cell).floor() as i32;
    let iz = (z / cell).floor() as i32;
    // 中心附近取相邻 4 格，避免格点边界处弹坑被硬切
    let mut m: f32 = 0.0;
    for dz in 0..2 {
        for dx in 0..2 {
            let gx = ix + dx;
            let gz = iz + dz;
            let h0 = hash2(gx, gz, seed);
            if unit_from_hash(h0) < 0.35 {
                let cx = gx as f32 * cell
                    + cell * 0.5
                    + lattice(gx, gz, seed.wrapping_add(1)) * cell * 0.3;
                let cz = gz as f32 * cell
                    + cell * 0.5
                    + lattice(gx, gz, seed.wrapping_add(2)) * cell * 0.3;
                let radius = 3.0 + unit_from_hash(hash2(gx, gz, seed.wrapping_add(3))) * 4.0;
                let dx = x - cx;
                let dz = z - cz;
                let d = (dx * dx + dz * dz).sqrt();
                let this = 1.0 - smoothstep(radius * 0.65, radius, d);
                m = m.max(this);
            }
        }
    }
    m
}

/// 地面材质基色（RGB，线性空间近似，写入 sRGB 纹理由硬件解释）
fn ground_material(x: f32, z: f32, seed: u32) -> [f32; 3] {
    // 大尺度地貌分域（~120m 尺度，地图 512m 内形成约 4 个草地/沙土/石板大区）
    let biome = value_noise(x, z, 120.0, seed.wrapping_add(20)) * 0.7
        + value_noise(x, z, 60.0, seed.wrapping_add(21)) * 0.3;
    // 中频斑驳（~10m 尺度，材质明暗变化）
    let detail = value_noise(x, z, 12.0, seed.wrapping_add(27)) * 0.6
        + value_noise(x, z, 6.0, seed.wrapping_add(28)) * 0.4;

    // 道路：一条沿 x 轴的土路 + 一条斜向土路
    let road_main = 1.0 - smoothstep(4.0, 6.5, z.abs());
    let road_diag = 1.0 - smoothstep(4.0, 6.5, (z * 0.6 + x * 0.8).abs());
    let road = road_main.max(road_diag);

    // 弹坑焦土
    let crater = crater_mask(x, z, seed.wrapping_add(33));

    // 明确分区配色（高饱和，保证可辨识）：草地绿 / 沙地黄 / 石板灰
    let grass: [f32; 3] = [0.20, 0.50, 0.15];
    let sand: [f32; 3] = [0.68, 0.54, 0.28];
    let stone: [f32; 3] = [0.50, 0.49, 0.56];
    let road_col: [f32; 3] = [0.42, 0.33, 0.23];
    let crater_col: [f32; 3] = [0.09, 0.08, 0.07];

    // 明确分区（smoothstep 阈值，避免连续抹平）：biome 低→草地、中→沙土、高→石板
    let grass_w = 1.0 - smoothstep(-0.18, -0.06, biome);
    let stone_w = smoothstep(0.06, 0.18, biome);
    let sand_w = (1.0 - grass_w - stone_w).clamp(0.0, 1.0);
    let mut base = [
        grass[0] * grass_w + sand[0] * sand_w + stone[0] * stone_w,
        grass[1] * grass_w + sand[1] * sand_w + stone[1] * stone_w,
        grass[2] * grass_w + sand[2] * sand_w + stone[2] * stone_w,
    ];

    // 道路覆盖（土路压过植被）
    base = lerp3(base, road_col, road * 0.9);
    // 弹坑焦土覆盖
    base = lerp3(base, crater_col, crater * 0.92);

    // 细节明暗抖动（±10%）
    let dither = 1.0 + detail * 0.10;
    [base[0] * dither, base[1] * dither, base[2] * dither]
}

// ============================================================
// 烘焙 AO + 静态天光
// ============================================================

/// 高度场凹度 AO：周围 8 方向采样的平均仰角越大（被丘陵环绕）→ 遮蔽越强 → 越暗。
/// 中央平坦区（高度恒 0）仰角为 0，AO = 1（无遮蔽）。
fn heightfield_ao(x: f32, z: f32, height_at: &dyn Fn(f32, f32) -> f32) -> f32 {
    let h0 = height_at(x, z);
    let r = 6.0;
    const N: usize = 8;
    let mut occ = 0.0;
    for i in 0..N {
        let a = (i as f32 / N as f32) * std::f32::consts::TAU;
        let (sx, sz) = (a.cos(), a.sin());
        let elev = (height_at(x + r * sx, z + r * sz) - h0) / r;
        occ += elev.max(0.0);
    }
    (1.0 - occ / N as f32 * 1.4).clamp(0.55, 1.0)
}

/// 静态天光：高度梯度求法线 → 太阳方向漫反射 + 天光底（弱烘焙）。
/// 保持较弱，避免与实时 Blinn-Phong 方向光/阴影叠加后过曝。
fn baked_light(x: f32, z: f32, height_at: &dyn Fn(f32, f32) -> f32) -> f32 {
    let eps = 2.0;
    let dx = (height_at(x + eps, z) - height_at(x - eps, z)) / (2.0 * eps);
    let dz = (height_at(x, z + eps) - height_at(x, z - eps)) / (2.0 * eps);
    let n = normalize3([-dx, 1.0, -dz]);
    let sun = normalize3(SUN_DIR);
    let diff = (n[0] * sun[0] + n[1] * sun[1] + n[2] * sun[2]).max(0.0);
    0.78 + 0.22 * diff
}

// ============================================================
// 主入口
// ============================================================

/// 生成世界空间地面纹理（RGBA8，`size * size * 4` 字节）。
///
/// `height_at` 为地形高度采样器（renderer 传入 `terrain_height`），用于烘焙 AO 与天光。
/// 结果确定性：相同 `size`/`seed`/`height_at` 恒同输出。
pub fn generate_ground_texture(
    size: u32,
    height_at: &dyn Fn(f32, f32) -> f32,
    seed: u32,
) -> Vec<u8> {
    let mut out = vec![0u8; (size as usize) * (size as usize) * 4];
    let scale = 2.0 * WORLD_HALF / size as f32;

    for py in 0..size {
        // py=0（顶部行）对应世界 z = -WORLD_HALF，与 shader uv.y 对齐（见模块头注释）
        let z = -WORLD_HALF + (py as f32 + 0.5) * scale;
        for px in 0..size {
            let x = -WORLD_HALF + (px as f32 + 0.5) * scale;

            let mat = ground_material(x, z, seed);
            let ao = heightfield_ao(x, z, height_at);
            let light = baked_light(x, z, height_at);
            let shade = ao * light;

            let idx = ((py * size + px) * 4) as usize;
            // 材质颜色为线性 RGB，写入 sRGB 纹理前编码（硬件采样时 decode 回线性）
            out[idx] = (linear_to_srgb(mat[0] * shade) * 255.0).round() as u8;
            out[idx + 1] = (linear_to_srgb(mat[1] * shade) * 255.0).round() as u8;
            out[idx + 2] = (linear_to_srgb(mat[2] * shade) * 255.0).round() as u8;
            out[idx + 3] = 255;
        }
    }
    out
}

/// 默认参数生成（测试与 renderer 共用）。
pub fn generate_default_ground_texture(
    height_at: &dyn Fn(f32, f32) -> f32,
) -> Vec<u8> {
    generate_ground_texture(GROUND_TEXTURE_SIZE, height_at, DEFAULT_SEED)
}

// ============================================================
// 单元测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 全平地形（用于验证平坦区 AO=1、天光恒定）
    fn flat_height(_x: f32, _z: f32) -> f32 {
        0.0
    }

    #[test]
    fn generate_is_deterministic_and_sized() {
        let a = generate_ground_texture(64, &flat_height, 123);
        let b = generate_ground_texture(64, &flat_height, 123);
        assert_eq!(a.len(), 64 * 64 * 4);
        assert_eq!(a, b, "同种子同输入必须逐字节一致");
        // alpha 恒不透明
        assert!(a.chunks_exact(4).all(|px| px[3] == 255));
    }

    #[test]
    fn different_seed_differs() {
        let a = generate_ground_texture(64, &flat_height, 1);
        let b = generate_ground_texture(64, &flat_height, 2);
        assert_ne!(a, b, "不同种子应产出不同纹理");
    }

    #[test]
    fn flat_ground_has_no_ao_occlusion() {
        // 全平地形：周围无高差 → 仰角 0 → AO = 1
        let ao = heightfield_ao(10.0, 20.0, &flat_height);
        assert!((ao - 1.0).abs() < 1e-6);
    }

    #[test]
    fn concave_ground_is_darker_than_flat() {
        // 中心凹坑（周围高）：中心点的 AO 应 < 1（被遮蔽）
        let bowl = |x: f32, z: f32| -> f32 { (x * x + z * z) * 0.01 };
        let ao_center = heightfield_ao(0.0, 0.0, &bowl);
        assert!(ao_center < 1.0, "凹处中心应有遮蔽，实际 {ao_center}");
        // 平坦远处 AO = 1
        let ao_flat = heightfield_ao(100.0, 100.0, &flat_height);
        assert!((ao_flat - 1.0).abs() < 1e-6);
    }

    #[test]
    fn baked_light_faces_sun_side_brighter() {
        // 朝太阳倾斜（法线指向太阳方向的分量更大）→ 更亮
        let sun = normalize3(SUN_DIR);
        // 构造一个法线朝太阳的坡：让 -dh/dx ≈ sun.x / sun.y（近似，仅验单调性）
        let slope_up = |x: f32, _z: f32| -> f32 { -sun[0] / sun[1].abs() * x };
        let light_sun = baked_light(0.0, 0.0, &slope_up);
        // 背对太阳的坡更暗
        let slope_down = |x: f32, _z: f32| -> f32 { sun[0] / sun[1].abs() * x };
        let light_away = baked_light(0.0, 0.0, &slope_down);
        assert!(light_sun > light_away, "朝阳坡应比背阳坡亮");
    }

    #[test]
    fn baked_light_in_unit_range() {
        let hilly = |x: f32, z: f32| -> f32 { (x * 0.05).sin() + (z * 0.05).cos() };
        for i in 0..64 {
            let x = (i as f32 - 32.0) * 2.0;
            let l = baked_light(x, x * 0.5, &hilly);
            assert!(l > 0.5 && l <= 1.0, "天光应在合理范围，实际 {l}");
        }
    }

    #[test]
    fn crater_mask_is_localized() {
        // 在某个已知弹坑格点中心应 > 0，远离格点应为 0（非确定但区域化）
        // 仅验证输出范围合法
        for i in 0..32 {
            let x = i as f32 * 5.0;
            let m = crater_mask(x, x, 7);
            assert!((0.0..=1.0).contains(&m));
        }
    }
}
