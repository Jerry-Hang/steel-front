//! 光照与阴影模块（Blinn-Phong + Shadow Mapping 基础实现）
//!
//! 纯 CPU 侧光照数学：既作为单元测试的验证对象，也作为 build.rs 内
//! `FRAGMENT_SHADER_WGSL` 光照计算的参考实现（函数/常量与 WGSL 一一对应）。
//!
//! 内容：
//! - `DirectionalLight` / `PointLight`：场景光照类型（方向、颜色、强度、衰减）
//! - Blinn-Phong：漫反射 + 高光（半程向量）+ 点光源衰减
//! - Shadow Mapping：shadow map 尺寸/格式常量、光空间正交 view-proj、
//!   深度 bias、采样坐标/深度投影与比较逻辑
//!
//! `LightUniform` 的布局与 build.rs 的 WGSL `LightUniform` 保持一致
//! （vec4 对齐，共 352 字节）；默认全零 = 光照关闭（向后兼容）。

use glam::{Mat4, Vec2, Vec3, Vec4};

/// 单帧光照 Uniform 中最多支持的点光源数量（与 WGSL `array<PointLight, 4>` 一致）
pub const MAX_POINT_LIGHTS: usize = 4;
/// 光照 Uniform 的 descriptor binding（与 WGSL `@binding(4)` 一致）
pub const LIGHT_UBO_BINDING: u32 = 4;
/// 光照 Uniform 总大小（字节）：
/// flags 16 + ambient 16 + directional 32 + 4×PointLight 48 + shadow(mat4 64 + bias 16 + config 16) = 352
pub const LIGHT_UBO_SIZE: usize = 352;

/// Shadow map 默认尺寸（基础实现：2048×2048）
///
/// 🔴 2026-09-14：`#[allow(dead_code)]` **已删** —— 它压制的是一个**活着**的常量
/// （`ShadowConfig::new` 与 `ShadowUniform::pack` 都在用，实测 17 处引用）。
/// 按铁律 F："看到'规划中'的 dead code，必须回答'那它为什么没被接线'，
/// **不许加 `#[allow]` 了事**" —— 而这几个 `allow` 连"没接线"这个前提都不成立，
/// 是纯粹的陈旧压制，只会掩盖真正新出现的 dead code。
/// 默认阴影贴图边长（历史默认值；实际生效值见 [`shadow_map_size`]）。
pub const SHADOW_MAP_SIZE: u32 = 2048;

/// 有效阴影贴图边长：`RV3D_SHADOW_MAP`（**1024..=2048**，默认 [`SHADOW_MAP_SIZE`]）。
///
/// ⚠️ 上限 2048 不是随意定的：2026-10-22 实测 4096² D32 在 RTX 5060 + Wayland/PRIME 上
/// **首帧就 device lost**（fence 5s 超时、日志无 VUID）⇒ 提高分辨率这条路本机走不通，
/// 阴影锐度只能靠"缩小覆盖范围/级联"解决。1024 保留给低配/性能对照。
///
/// 为什么必须**只通过这里**取：着色器用 `shadow.config.x` 推纹素尺寸、Rust 侧用同一个数
/// 建图/设视口 —— 两边不同就是"阴影整体偏移/错位"的静默错误（本仓惯犯）。
/// 一次性缓存：env 是进程级常量，跑起来后再改没有意义（测试里也不许改 env）。
pub fn shadow_map_size() -> u32 {
    static SIZE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| {
        crate::syscfg::parse::<u32>("RV3D_SHADOW_MAP")
            .map(|v| v.clamp(1024, 2048))
            .unwrap_or(SHADOW_MAP_SIZE)
    })
}
// 2026-09-14 **删除** `SHADOW_MAP_FORMAT: u32 = 126`：
// 它唯一一次出现就是它自己的定义（全仓仅 1 处引用），是 `vk::Format::D32_SFLOAT`
// 的手抄副本 —— 渲染器直接用 ash 枚举，这个数字常量既没有消费者、也可能与 ash 分叉。
// 留一个"注释说它对应什么"的常量，比删掉它更危险。
/// 默认阴影深度 bias（缓解 shadow acne）
pub const DEFAULT_SHADOW_DEPTH_BIAS: f32 = 0.005;
/// 默认阴影法线 bias（沿法线外推的基数，WGSL `light_data.shadow.bias.y`）
pub const DEFAULT_SHADOW_NORMAL_BIAS: f32 = 0.02;
/// 默认高光指数（与 WGSL 一致）
#[allow(dead_code)]
pub const DEFAULT_SHININESS: f32 = 32.0;
/// 高光强度系数（与 WGSL 一致）
#[allow(dead_code)]
pub const SPECULAR_STRENGTH: f32 = 0.4;

/// 手电亮度随电量：返回 (强度系数 0.25..1.0, 作用半径 8..14 m)。
///
/// 电量低时**变暗而不是突然熄灭**（玩家有预警）；`sqrt` 让前 75% 电量几乎不变化。
pub fn flashlight_dim(battery: f32) -> (f32, f32) {
    let b = battery.clamp(0.0, 1.0);
    (0.25 + 0.75 * b.sqrt(), 8.0 + 6.0 * b)
}

/// AI 视距的总系数：夜间衰减 × 玩家是否亮着（手电 = 灯塔，看得到更远）。
///
/// `night` 来自 `vision_factor_for_sun`（1.0 白天 / 0.55 夜里）；`player_lit` 只在夜间有效。
pub fn ai_sight_factor(night: f32, player_lit: bool) -> f32 {
    let lit = if player_lit { 1.25 } else { 1.0 };
    night.clamp(0.0, 1.0) * lit
}

/// 手电电量推进一步（0..1）。开灯耗电（`on_secs` 秒用光），关灯缓慢回充（`off_secs` 秒充满）。
///
/// 纯函数：`Game` 每帧调用，单测直接验（时间基不依赖真实时钟）。
pub fn battery_step(battery: f32, on: bool, dt: f32, on_secs: f32, off_secs: f32) -> f32 {
    let b = battery.clamp(0.0, 1.0);
    if on {
        (b - dt / on_secs.max(1.0)).max(0.0)
    } else {
        (b + dt / off_secs.max(1.0)).min(1.0)
    }
}

/// 手电位置：相机前方 1.5 m、比眼位略低 0.15 m（照在地面上更明显）。
pub fn flashlight_position(cam: [f32; 3], yaw: f32) -> [f32; 3] {
    let fwd = [-yaw.sin(), 0.0, -yaw.cos()];
    [cam[0] + fwd[0] * 1.5, cam[1] - 0.15, cam[2] + fwd[2] * 1.5]
}

/// 夜间视距系数：太阳越低，NPC 看得越近（玩法：夜里更容易靠近）。
///
/// 1.0 = 白天满视距；0.55 = 夜里只剩 55%。用 smoothstep 在 0.02..0.42 之间过渡，
/// 与天空 `dayk`/雾色同一条太阳高度曲线（三处必须共用，否则"天黑了 AI 还看得老远"）。
pub fn vision_factor_for_sun(sun_y: f32) -> f32 {
    let t = ((sun_y - 0.02) / 0.40).clamp(0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);
    0.55 + 0.45 * s
}

/// 取离相机最近的 `n` 个路灯（按 xz 距离，距离相同按原顺序 —— 确定性）。
///
/// 引擎只有 4 个点光槽位、实际用 2 个；低太阳时把它们挪到玩家身边最近的两个，
/// 才能看见"脚下的光池"，而不是远处固定的两个装饰灯。
pub fn pick_nearest_lamps(lamps: &[[f32; 3]], cam_xz: [f32; 2], n: usize) -> Vec<[f32; 3]> {
    let mut idx: Vec<usize> = (0..lamps.len()).collect();
    idx.sort_by(|&a, &b| {
        let da = (lamps[a][0] - cam_xz[0]).powi(2) + (lamps[a][2] - cam_xz[1]).powi(2);
        let db = (lamps[b][0] - cam_xz[0]).powi(2) + (lamps[b][2] - cam_xz[1]).powi(2);
        da.partial_cmp(&db)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    idx.truncate(n);
    idx.into_iter().map(|i| lamps[i]).collect()
}

/// 太阳方向（表面→光源）：`elev_deg` / `azim_deg` 为角度制。
///
/// 与 `RV3D_SUN_ELEV` / `RV3D_SUN_AZIM` 配套：不设时沿用 `view.rs` 的默认向量
/// （-0.4, 0.9, -0.3，等于 elev≈61°、azim≈233°）。
pub fn sun_direction(elev_deg: f32, azim_deg: f32) -> Vec3 {
    let e = elev_deg.to_radians();
    let a = azim_deg.to_radians();
    Vec3::new(a.cos() * e.cos(), e.sin(), a.sin() * e.cos()).normalize()
}

/// 低太阳 → 更暖、更弱：返回 (颜色, 强度系数)。
///
/// 60° 以上与现有配平一致（1.0,0.95,0.85）/1.0；10° 附近偏橙红且降到 0.74 强度
/// （低角度穿过更厚大气，直射能量更低）。只影响方向光，环境光不动。
pub fn sun_tint(elev_deg: f32) -> (Vec3, f32) {
    let t = (elev_deg.clamp(10.0, 60.0) - 10.0) / 50.0;
    let warm = Vec3::new(1.0, 0.62, 0.40);
    let noon = Vec3::new(1.0, 0.95, 0.85);
    (warm.lerp(noon, t), 0.62 + 0.38 * t)
}

/// 从环境变量解析太阳角度；未设置/非法返回 None（沿用默认）。
///
/// `RV3D_SUN_ELEV` 建议 8..80；`RV3D_SUN_AZIM` 0..360（0 = +X 方向起算）。
pub fn sun_from_env() -> Option<(Vec3, Vec3, f32)> {
    // 必须走 crate::syscfg（Android 上没有进程环境变量；直读 std::env 会静默失效）。
    let elev = crate::syscfg::parse::<f32>("RV3D_SUN_ELEV")?;
    let azim = crate::syscfg::parse::<f32>("RV3D_SUN_AZIM").unwrap_or(233.0);
    let (color, scale) = sun_tint(elev);
    Some((sun_direction(elev, azim), color, scale))
}

/// 方向光：`direction` 为从表面指向光源的方向
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalLight {
    pub direction: Vec3,
    pub color: Vec3,
    pub intensity: f32,
}

impl DirectionalLight {
    pub fn new(direction: Vec3, color: Vec3, intensity: f32) -> Self {
        Self {
            direction,
            color,
            intensity,
        }
    }
}

/// 点光源：带常量/线性/二次衰减与有效作用半径
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointLight {
    pub position: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    /// 常量衰减系数
    pub constant: f32,
    /// 线性衰减系数
    pub linear: f32,
    /// 二次衰减系数
    pub quadratic: f32,
    /// 有效作用半径（0 = 不限距离）
    pub range: f32,
}

impl PointLight {
    pub fn new(position: Vec3, color: Vec3, intensity: f32) -> Self {
        Self {
            position,
            color,
            intensity,
            constant: 1.0,
            linear: 0.09,
            quadratic: 0.032,
            range: 0.0,
        }
    }
}

/// Shadow map 配置（方向光正交光空间）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowConfig {
    pub map_size: u32,
    pub depth_bias: f32,
    pub normal_bias: f32,
    /// 表面→光源方向（如正午太阳光为 +Y；实现中视线沿 -light_dir 观察场景）
    pub light_dir: Vec3,
    /// 阴影覆盖区域中心（通常为相机注视点）
    pub target: Vec3,
    /// 正交投影半宽/半高（世界单位）
    pub extent: f32,
    pub near: f32,
    pub far: f32,
}

impl ShadowConfig {
    pub fn new(light_dir: Vec3, target: Vec3, extent: f32, near: f32, far: f32) -> Self {
        Self {
            map_size: shadow_map_size(),
            depth_bias: DEFAULT_SHADOW_DEPTH_BIAS,
            normal_bias: DEFAULT_SHADOW_NORMAL_BIAS,
            light_dir,
            target,
            extent,
            near,
            far,
        }
    }

    /// 光空间 view-proj 矩阵（世界空间 → 光裁剪空间）
    pub fn view_proj(&self) -> Mat4 {
        shadow_view_proj(self.light_dir, self.target, self.extent, self.near, self.far)
    }
}

// ============================================================
// WGSL `LightUniform` 布局镜像（vec4 对齐，共 352 字节）
// ============================================================

/// 方向光 Uniform（与 WGSL `DirectionalLight` 一致）
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalLightUniform {
    /// xyz = 表面→光源方向，w = enabled(1.0/0.0)
    pub direction: Vec4,
    /// rgb = 颜色，w = 强度
    pub color_intensity: Vec4,
}

/// 点光源 Uniform（与 WGSL `PointLight` 一致）
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointLightUniform {
    /// xyz = 世界位置，w = enabled(1.0/0.0)
    pub position: Vec4,
    /// rgb = 颜色，w = 强度
    pub color_intensity: Vec4,
    /// x = constant, y = linear, z = quadratic, w = range
    pub attenuation: Vec4,
}

/// 阴影 Uniform（与 WGSL `ShadowInfo` 一致）
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowUniform {
    /// 光空间 view-proj（世界空间 → 光裁剪空间）
    pub light_view_proj: Mat4,
    /// x = depth_bias, y = normal_bias, z = enabled, w = 0
    pub bias: Vec4,
    /// x = shadow map 尺寸, y/z/w = 0
    pub config: Vec4,
}

/// 光照 Uniform（与 WGSL `LightUniform` 一致，默认全零 = 关闭）
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightUniform {
    /// x = lighting enabled, y = shadow enabled, z/w = 0
    pub flags: Vec4,
    /// rgb = 环境色, w = 环境强度
    pub ambient: Vec4,
    pub directional: DirectionalLightUniform,
    pub points: [PointLightUniform; MAX_POINT_LIGHTS],
    pub shadow: ShadowUniform,
}

impl Default for LightUniform {
    fn default() -> Self {
        Self {
            flags: Vec4::ZERO,
            ambient: Vec4::ZERO,
            directional: DirectionalLightUniform {
                direction: Vec4::ZERO,
                color_intensity: Vec4::ZERO,
            },
            points: [PointLightUniform {
                position: Vec4::ZERO,
                color_intensity: Vec4::ZERO,
                attenuation: Vec4::ZERO,
            }; MAX_POINT_LIGHTS],
            shadow: ShadowUniform {
                light_view_proj: Mat4::ZERO,
                bias: Vec4::ZERO,
                config: Vec4::ZERO,
            },
        }
    }
}

/// 编译期校验：布局必须与 WGSL `LightUniform` 一致（std140，352 字节）
const _: () = assert!(std::mem::size_of::<LightUniform>() == LIGHT_UBO_SIZE);
const _: () = assert!(std::mem::size_of::<PointLightUniform>() == 48);
const _: () = assert!(std::mem::size_of::<ShadowUniform>() == 96);
const _: () = assert!(std::mem::align_of::<LightUniform>() <= 16);

impl LightUniform {
    /// 构建光照 Uniform；`directional` 与 `points` 均为空时保持全零（光照关闭）。
    pub fn build(
        directional: Option<&DirectionalLight>,
        points: &[PointLight],
        ambient_color: Vec3,
        ambient_intensity: f32,
        shadow: Option<&ShadowConfig>,
    ) -> Self {
        let mut u = Self::default();
        if directional.is_some() || !points.is_empty() {
            u.flags.x = 1.0;
        }
        u.ambient = ambient_color.extend(ambient_intensity);
        if let Some(d) = directional {
            u.directional.direction = d.direction.extend(1.0);
            u.directional.color_intensity = d.color.extend(d.intensity);
        }
        for (i, p) in points.iter().take(MAX_POINT_LIGHTS).enumerate() {
            u.points[i].position = p.position.extend(1.0);
            u.points[i].color_intensity = p.color.extend(p.intensity);
            u.points[i].attenuation = Vec4::new(p.constant, p.linear, p.quadratic, p.range);
        }
        if let Some(s) = shadow {
            // 性能诊断（2026-10-09）：RV3D_NO_SHADOW=1 关掉逐像素阴影 PCF，
            // 用来量阴影在片元成本里占多少。实机：setprop debug.sf.no_shadow 1
            if crate::syscfg::flag("RV3D_NO_SHADOW") {
                return u;
            }
            u.flags.y = 1.0;
            u.shadow.light_view_proj = s.view_proj();
            u.shadow.bias = Vec4::new(s.depth_bias, s.normal_bias, 1.0, 0.0);
            u.shadow.config = Vec4::new(s.map_size as f32, 0.0, 0.0, 0.0);
        }
        u
    }
}

// ============================================================
// Blinn-Phong 光照数学（与 WGSL 一致）
// ============================================================

/// Blinn-Phong 漫反射系数：`max(dot(n, l), 0)`
#[allow(dead_code)] // Blinn-Phong/阴影数学参考实现：供测试与 WGSL 着色器对照（见模块头注释）
pub fn blinn_phong_diffuse(normal: Vec3, light_dir: Vec3) -> f32 {
    normal.dot(light_dir).max(0.0)
}

/// Blinn-Phong 高光系数：`pow(max(dot(n, h), 0), shininess)`，`h = normalize(l + v)`
#[allow(dead_code)]
pub fn blinn_phong_specular(normal: Vec3, light_dir: Vec3, view_dir: Vec3, shininess: f32) -> f32 {
    // 🔴 **背光面不许出高光**（2026-10-19 外部复核第 9 条，已确认）：
    // 没有这层门控时，`n·l <= 0` 的暗面在掠射观察角下仍能算出 `n·h > 0` ⇒ 背光漏光。
    // 注意：本文件是 CPU 侧参考实现（渲染用的是 `build.rs` 里的 WGSL，两处要同步看）。
    if normal.dot(light_dir) <= 0.0 {
        return 0.0;
    }
    let half = (light_dir + view_dir).normalize_or_zero();
    normal.dot(half).max(0.0).powf(shininess)
}

/// 点光源衰减：`1 / (c + l*d + q*d²)`，clamp 到 [0, 1]
#[allow(dead_code)]
pub fn point_attenuation(distance: f32, constant: f32, linear: f32, quadratic: f32) -> f32 {
    let denom = constant + linear * distance + quadratic * distance * distance;
    if denom <= 0.0 {
        1.0
    } else {
        (1.0 / denom).min(1.0)
    }
}

/// 方向光辐射度（与 WGSL `evaluate_directional` 一致）
#[allow(dead_code)]
pub fn directional_radiance(
    light: &DirectionalLight,
    normal: Vec3,
    view_dir: Vec3,
    shininess: f32,
) -> Vec3 {
    let light_dir = light.direction.normalize_or_zero();
    let diffuse = blinn_phong_diffuse(normal, light_dir);
    let spec = blinn_phong_specular(normal, light_dir, view_dir, shininess);
    light.color * light.intensity * (diffuse + SPECULAR_STRENGTH * spec)
}

/// 点光源辐射度（与 WGSL `evaluate_point` 一致；`range = 0` 时不限距离）
#[allow(dead_code)]
pub fn point_radiance(
    light: &PointLight,
    world_pos: Vec3,
    normal: Vec3,
    view_dir: Vec3,
    shininess: f32,
) -> Vec3 {
    let to_light = light.position - world_pos;
    let dist = to_light.length();
    if light.range > 0.0 && dist > light.range {
        return Vec3::ZERO;
    }
    let light_dir = if dist > 1e-6 {
        to_light / dist
    } else {
        Vec3::ZERO
    };
    let diffuse = blinn_phong_diffuse(normal, light_dir);
    let spec = blinn_phong_specular(normal, light_dir, view_dir, shininess);
    let atten = point_attenuation(dist, light.constant, light.linear, light.quadratic);
    light.color * light.intensity * atten * (diffuse + SPECULAR_STRENGTH * spec)
}

// ============================================================
// Shadow Mapping（基础实现：方向光正交 shadow map）
// ============================================================

/// 光空间正交 view-proj：世界空间 → 光裁剪空间（RH，深度 0..1）。
///
/// `light_dir` 为表面→光源方向；以 `target` 为中心、`extent` 为半宽/半高，
/// 视线沿 -light_dir 观察场景（光源在场景上方时自然俯视），把场景中心置于光视锥中段。
pub fn shadow_view_proj(light_dir: Vec3, target: Vec3, extent: f32, near: f32, far: f32) -> Mat4 {
    let fwd = (-light_dir).normalize_or_zero();
    // 光线方向与 Y 平行时退化为用 X 轴做参考上方向，避免零向量叉积
    let up_ref = if fwd.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    let right = fwd.cross(up_ref).normalize_or_zero();
    let up = right.cross(fwd).normalize_or_zero();
    let eye = target - fwd * far * 0.5;
    let view = Mat4::look_at_rh(eye, target, up);
    let proj = Mat4::orthographic_rh(-extent, extent, -extent, extent, near, far);
    proj * view
}

/// 把世界坐标投影到光空间：返回 (shadow uv, 光空间深度)。
/// 与片元着色器采样一致：uv.y 镜像（depth-only pass 顶点输出经 ADJUST_COORDINATE_SPACE
/// Y 翻转）；深度为 clip.z 原值直传（glam ortho_rh 本版本产出 [0,1] 深度，与 GPU
/// 阴影图写入同基准；旧 *0.5+0.5 二重映射致 +0.25 偏移全场误判阴影，已根治）。
#[allow(dead_code)]
pub fn world_to_shadow_uv(world_pos: Vec3, shadow_vp: Mat4) -> (Vec2, f32) {
    let p = shadow_vp * world_pos.extend(1.0);
    let uv = Vec2::new(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    (uv, p.z)
}

/// 阴影深度比较（bias 缓解 acne）：`fragment_depth - bias > shadow_depth` 判定为阴影
#[allow(dead_code)]
pub fn shadow_depth_test(shadow_depth: f32, fragment_depth: f32, depth_bias: f32) -> bool {
    fragment_depth - depth_bias > shadow_depth
}

/// 阴影可见性：0.0 = 阴影，1.0 = 照亮（与片元着色器 `1.0 - shadow_test(...)` 语义一致）
#[allow(dead_code)]
pub fn shadow_visibility(shadow_depth: f32, fragment_depth: f32, depth_bias: f32) -> f32 {
    if shadow_depth_test(shadow_depth, fragment_depth, depth_bias) {
        0.0
    } else {
        1.0
    }
}

// ============================================================
// 单元测试：覆盖光照数学（高光、衰减、shadow bias 比较等）
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    #[test]
    fn diffuse_clamps_and_peaks() {
        let n = Vec3::Y;
        assert!((blinn_phong_diffuse(n, Vec3::Y) - 1.0).abs() < EPS);
        assert!(blinn_phong_diffuse(n, Vec3::X).abs() < EPS);
        // 背光面为 0（clamp 而非负值）
        assert_eq!(blinn_phong_diffuse(n, -Vec3::Y), 0.0);
    }

    #[test]
    fn specular_peaks_when_half_vector_aligns_with_normal() {
        let n = Vec3::Y;
        // 视线与光源关于法线对称 → 半程向量 == 法线 → 高光 = 1
        let view = Vec3::new(0.0, 1.0, 1.0).normalize();
        let light = Vec3::new(0.0, 1.0, -1.0).normalize();
        let spec = blinn_phong_specular(n, light, view, DEFAULT_SHININESS);
        assert!((spec - 1.0).abs() < EPS);

        // 光源偏离半程向量后高光下降，且指数越大下降越快
        let off = Vec3::new(1.0, 1.0, 0.0).normalize();
        let spec_high = blinn_phong_specular(n, off, view, DEFAULT_SHININESS);
        let spec_low = blinn_phong_specular(n, off, view, 4.0);
        assert!(spec_high > 0.0 && spec_high < spec);
        assert!(spec_high < spec_low);
    }

    #[test]
    fn attenuation_falls_off_with_distance() {
        assert!((point_attenuation(0.0, 1.0, 0.0, 0.0) - 1.0).abs() < EPS);
        let a1 = point_attenuation(10.0, 1.0, 0.09, 0.032);
        let a2 = point_attenuation(50.0, 1.0, 0.09, 0.032);
        assert!(a1 > a2 && a2 > 0.0);
        // 二次项主导：距离翻倍后衰减显著变小
        let d = 20.0;
        assert!(
            point_attenuation(2.0 * d, 1.0, 0.09, 0.032)
                < point_attenuation(d, 1.0, 0.09, 0.032)
        );
        // 非法分母（<= 0）按 1.0 处理
        assert_eq!(point_attenuation(1.0, -1.0, 0.0, 0.0), 1.0);
    }

    /// 🔴 **背光面不许出高光**（2026-10-19 外部复核第 9 条）。
    /// 反向验证：删掉 `blinn_phong_specular` 开头的 `n·l <= 0` 门控 ⇒ 本判据红。
    #[test]
    fn backface_has_no_specular() {
        let light = DirectionalLight::new(Vec3::new(0.0, -1.0, -1.0).normalize(), Vec3::ONE, 1.0);
        let view = Vec3::new(0.0, 0.9, 0.4).normalize();
        let r = directional_radiance(&light, Vec3::Y, view, DEFAULT_SHININESS);
        assert_eq!(
            r,
            Vec3::ZERO,
            "背光面（n·l < 0）仍有高光：{r:?}（暗面漏光）"
        );
    }

    #[test]
    fn directional_light_faces_surface() {
        let light = DirectionalLight::new(Vec3::Y, Vec3::ONE, 1.0);
        let view = Vec3::Y;
        // 正对光源：漫反射 1 + 高光 1 × SPECULAR_STRENGTH
        let r = directional_radiance(&light, Vec3::Y, view, DEFAULT_SHININESS);
        let expect = 1.0 + SPECULAR_STRENGTH;
        assert!((r.x - expect).abs() < EPS);
        assert!((r.y - expect).abs() < EPS);
        // 背对光源：无贡献
        assert_eq!(directional_radiance(&light, -Vec3::Y, view, DEFAULT_SHININESS), Vec3::ZERO);
    }

    #[test]
    fn point_light_attenuation_and_range() {
        let light = PointLight::new(Vec3::new(0.0, 10.0, 0.0), Vec3::ONE, 1.0);
        let view = Vec3::Y;
        let pos = Vec3::ZERO;
        let r = point_radiance(&light, pos, Vec3::Y, view, DEFAULT_SHININESS);
        // dist = 10：atten = 1 / (1 + 0.9 + 3.2) ≈ 0.196
        let atten = point_attenuation(10.0, light.constant, light.linear, light.quadratic);
        assert!((r.y - (1.0 + SPECULAR_STRENGTH) * atten).abs() < 1e-4);
        // 超出 range → 无贡献
        let mut ranged = light;
        ranged.range = 5.0;
        assert_eq!(
            point_radiance(&ranged, pos, Vec3::Y, view, DEFAULT_SHININESS),
            Vec3::ZERO
        );
    }

    #[test]
    fn shadow_bias_comparison() {
        // 深度差超过 bias → 阴影
        assert!(shadow_depth_test(0.90, 0.95, 0.01));
        // 深度差在 bias 内 → 照亮（防 acne）
        assert!(!shadow_depth_test(0.90, 0.905, 0.01));
        // 相等深度 → 照亮
        assert!(!shadow_depth_test(0.90, 0.90, 0.01));
        // 更靠近光源 → 照亮
        assert!(!shadow_depth_test(0.90, 0.80, 0.01));
        // 可见性语义：0 = 阴影，1 = 照亮
        assert_eq!(shadow_visibility(0.90, 0.95, 0.01), 0.0);
        assert_eq!(shadow_visibility(0.90, 0.905, 0.01), 1.0);
    }

    #[test]
    fn shadow_view_proj_maps_target_to_uv_center() {
        let vp = shadow_view_proj(Vec3::Y, Vec3::ZERO, 50.0, 1.0, 200.0);
        let (uv, depth) = world_to_shadow_uv(Vec3::ZERO, vp);
        // 场景中心 → shadow uv 中心，深度处于 [0,1] 中段
        assert!((uv.x - 0.5).abs() < 1e-3);
        assert!((uv.y - 0.5).abs() < 1e-3);
        assert!(depth > 0.0 && depth < 1.0);
        // 沿远离光源方向（向下）更远处 → 深度单调增大
        let far_depth = world_to_shadow_uv(Vec3::new(0.0, -10.0, 0.0), vp).1;
        let near_depth = world_to_shadow_uv(Vec3::new(0.0, 10.0, 0.0), vp).1;
        assert!(far_depth > near_depth);
        // 覆盖范围内 → uv 落在 [0,1]
        let (uv2, _) = world_to_shadow_uv(Vec3::new(30.0, 0.0, 30.0), vp);
        assert!(uv2.x > 0.0 && uv2.x < 1.0 && uv2.y > 0.0 && uv2.y < 1.0);
    }

    #[test]
    fn world_to_shadow_uv_mirrors_v_and_maps_depth() {
        // light_dir=+Y 的光基构造：光空间右轴=+Z 世界、上轴=+X 世界
        let vp = shadow_view_proj(Vec3::Y, Vec3::ZERO, 50.0, 1.0, 200.0);
        // 光空间 +X（世界 +Z）→ uv.x > 0.5
        let (uv_px, _) = world_to_shadow_uv(Vec3::new(0.0, 0.0, 10.0), vp);
        assert!(uv_px.x > 0.5);
        // V 镜像铁律：光空间 +Y（世界 +X）→ uv.y < 0.5
        // （depth-only pass 顶点输出经 ADJUST_COORDINATE_SPACE Y 翻转，
        //  不镜像则阴影采样方向整体错位）
        let (uv_py, _) = world_to_shadow_uv(Vec3::new(10.0, 0.0, 0.0), vp);
        assert!(uv_py.y < 0.5);
        // 深度映射铁律（2026-08-31 修正）：glam ortho_rh 本版本产出 [0,1] 深度，
        // frag_depth = clip.z 原值（禁止 *0.5+0.5 二重映射，旧映射整体偏移 +0.25
        // 致全场误判阴影）；场景中心在光视锥中段 → 深度 ≈0.5 邻域且在 (0,1) 内
        let (_, depth_origin) = world_to_shadow_uv(Vec3::ZERO, vp);
        assert!(depth_origin > 0.0 && depth_origin < 1.0);
        // 本测试配置（near=1, far=200，中心距光 100m）：深度 ≈0.4975 ∈ (0.4, 0.6)
        assert!(depth_origin > 0.4 && depth_origin < 0.6);
        // 沿远离光源方向（-Y）更深处 → 深度单调更大
        let (_, depth_deep) = world_to_shadow_uv(Vec3::new(0.0, -50.0, 0.0), vp);
        assert!(depth_deep > depth_origin);
    }

    #[test]
    fn light_uniform_default_is_disabled() {
        let u = LightUniform::default();
        assert_eq!(u.flags.x, 0.0);
        assert_eq!(u.flags.y, 0.0);
        assert_eq!(u.ambient, Vec4::ZERO);
        assert_eq!(u.directional.direction, Vec4::ZERO);
        assert_eq!(u.shadow.light_view_proj, Mat4::ZERO);
    }

    #[test]
    fn light_uniform_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<LightUniform>(), LIGHT_UBO_SIZE);
        assert_eq!(std::mem::size_of::<PointLightUniform>(), 48);
        assert_eq!(std::mem::size_of::<ShadowUniform>(), 96);
    }

    #[test]
    fn build_packs_directional_points_and_shadow() {
        let dir = DirectionalLight::new(Vec3::Y, Vec3::ONE, 1.0);
        let pt = PointLight::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 0.5, 0.25), 0.5);
        let shadow = ShadowConfig::new(Vec3::Y, Vec3::ZERO, 50.0, 1.0, 200.0);
        let u = LightUniform::build(
            Some(&dir),
            &[pt],
            Vec3::new(0.1, 0.1, 0.1),
            0.2,
            Some(&shadow),
        );

        assert_eq!(u.flags.x, 1.0);
        assert_eq!(u.flags.y, 1.0);
        assert_eq!(u.ambient, Vec4::new(0.1, 0.1, 0.1, 0.2));
        assert_eq!(u.directional.direction, Vec4::new(0.0, 1.0, 0.0, 1.0));
        assert_eq!(u.directional.color_intensity, Vec4::new(1.0, 1.0, 1.0, 1.0));
        assert_eq!(u.points[0].position, Vec4::new(1.0, 2.0, 3.0, 1.0));
        assert_eq!(u.points[0].color_intensity, Vec4::new(1.0, 0.5, 0.25, 0.5));
        assert_eq!(u.points[0].attenuation, Vec4::new(1.0, 0.09, 0.032, 0.0));
        assert_eq!(u.points[1].position.w, 0.0);
        assert_eq!(u.shadow.bias.x, DEFAULT_SHADOW_DEPTH_BIAS);
        assert_eq!(u.shadow.bias.y, DEFAULT_SHADOW_NORMAL_BIAS);
        assert_eq!(u.shadow.config.x, shadow.map_size as f32);
        assert_ne!(u.shadow.light_view_proj, Mat4::ZERO);
    }

    /// 画质 M5：阴影贴图分辨率可调（`RV3D_SHADOW_MAP`），且**唯一取用入口**是
    /// `shadow_map_size()` —— 分配/视口/着色器 `config.x` 三处必须同源。
    #[test]
    fn shadow_map_size_is_shared_by_allocation_and_uniform() {
        let size = shadow_map_size();
        assert!((1024..=2048).contains(&size), "阴影贴图边长 {size} 超出 1024..=2048");
        assert_eq!(size, shadow_map_size(), "第二次取值不稳定（缓存失效）");
        let shadow = ShadowConfig::new(Vec3::Y, Vec3::ZERO, 50.0, 1.0, 200.0);
        assert_eq!(shadow.map_size, size);
        let u = LightUniform::build(None, &[], Vec3::ZERO, 0.0, Some(&shadow));
        assert_eq!(u.shadow.config.x, size as f32);
        let shadow_rs = std::fs::read_to_string("src/engine/renderer/shadow.rs")
            .expect("读不到 renderer/shadow.rs（判据扫描面失效）");
        assert!(shadow_rs.contains("crate::engine::lighting::shadow_map_size()"),
            "renderer/shadow.rs 没走 shadow_map_size()，分配尺寸可能与 uniform 分叉");
        assert!(!shadow_rs.contains("use crate::engine::lighting::SHADOW_MAP_SIZE;"),
            "renderer/shadow.rs 仍直接使用 SHADOW_MAP_SIZE 常量");
    }

    /// 画质 M5 判据：**ACES 色调映射系数单一真源**。
    ///
    /// 世界片元与天空 pass 是两份独立 WGSL，必须用同一组系数，否则"雾化到地平线的楼"
    /// 与天空会重新出现接缝。做法 = build.rs 五个 Rust 常量 + 构建期注入两处 + 构建期断言
    /// （删掉任一处 tonemap 会直接构建失败）；本判据再核对生成系数 == build.rs 常量。
    #[test]
    fn aces_tonemap_coefficients_are_shared() {
        let build = include_str!("../../build.rs");
        let generated = crate::shaders::ACES_COEFFS;
        for (name, expect) in [
            ("ACES_A", generated[0]),
            ("ACES_B", generated[1]),
            ("ACES_C", generated[2]),
            ("ACES_D", generated[3]),
            ("ACES_E", generated[4]),
        ] {
            let needle = format!("const {name}: f32 = ");
            let at = build.find(&needle).unwrap_or_else(|| panic!("build.rs 里找不到 {name}"));
            let rest = &build[at + needle.len()..];
            let end = rest.find(';').expect("常量没有分号结尾");
            let parsed: f32 = rest[..end].trim().parse().unwrap_or_else(|_| panic!("{name} 之后不是数字"));
            assert!((parsed - expect).abs() < 1e-6, "{name}: build.rs {parsed} vs shaders.rs {expect}");
        }
        assert!(build.matches("aces_inject(").count() >= 2, "aces_inject 没有同时作用于两份 WGSL");
        assert!(
            build.contains("片元着色器丢了 ACES tonemap") && build.contains("天空着色器丢了 ACES tonemap"),
            "构建期硬断言被删 —— 删掉任一处 tonemap 不会再报错"
        );
        let exposure = crate::shaders::TONEMAP_EXPOSURE;
        assert!((0.05..=8.0).contains(&exposure), "曝光 {exposure} 超出合理范围");
    }
    
    #[test]
    fn build_with_no_lights_stays_disabled() {
        let u = LightUniform::build(None, &[], Vec3::ZERO, 0.0, None);
        assert_eq!(u, LightUniform::default());
    }

    /// CPU 侧**参考光照**与光栅 WGSL 必须用同一组常数。
    ///
    /// 本模块头注释自称「`FRAGMENT_SHADER_WGSL` 光照计算的参考实现
    /// （函数/常量与 WGSL **一一对应**）」，而下面这两个常数各自带着「（与 WGSL 一致）」
    /// 的注释与 `#[allow(dead_code)]`（只被参考测试用）。⇒ 它们是**声明过却没有守卫**的
    /// 不变量：一旦分叉，本模块的参考测试验证的就是着色器**已经不再实现**的模型。
    ///
    /// | CPU 参考 | 值 | WGSL 侧 |
    /// |---|---|---|
    /// | `DEFAULT_SHININESS` | 32.0 | `let shininess = 32.0;`（`build.rs:535`，全场景硬编） |
    /// | `SPECULAR_STRENGTH` | 0.4  | `const SPEC_CONTRIB: f32 = 0.4;`（`build.rs:190`） |
    ///
    /// `SPEC_CONTRIB` 还被 `pt_panorama.glsl:260` 的注释点名引用，是三方知情的那一族。
    /// 判据全部**从 build.rs 源码解析**，不抄数字（同值 ≠ 同源，这里逐个核实过用法才收）。
    #[test]
    fn reference_lighting_constants_match_the_raster_shader() {
        let build = include_str!("../../build.rs");

        /// 取 `let NAME = V;` / `const NAME: f32 = V;` 的数值。
        fn value_after(src: &str, needle: &str, what: &str) -> f32 {
            let at = src
                .find(needle)
                .unwrap_or_else(|| panic!("{what} 里找不到 `{needle}`（被改名或删了？）"));
            let rest = &src[at + needle.len()..];
            let end = rest
                .find(';')
                .unwrap_or_else(|| panic!("`{needle}` 没有分号结尾，解析口径失效"));
            let tok = rest[..end].trim();
            tok.parse::<f32>()
                .unwrap_or_else(|_| panic!("{what} 的 `{needle}` 之后不是数字：{tok:?}"))
        }

        let cpu_shininess = DEFAULT_SHININESS;
        let cpu_spec = SPECULAR_STRENGTH;

        // 1) 高光指数：WGSL 里是全场景硬编的 `let shininess = …`，不读任何材质
        let shader_shininess = value_after(build, "let shininess = ", "build.rs WGSL");
        assert!(
            (shader_shininess - cpu_shininess).abs() < 1e-6,
            "光栅 WGSL 的 shininess = {shader_shininess}，CPU 参考实现用 {cpu_shininess}：\
             分叉后本模块的参考测试验证的是着色器不再实现的模型"
        );

        // 2) 高光贡献权重：两侧都在 `diffuse + K * spec` 里当同一个 K
        let shader_spec = value_after(build, "const SPEC_CONTRIB: f32 = ", "build.rs WGSL");
        assert!(
            (shader_spec - cpu_spec).abs() < 1e-6,
            "光栅 WGSL 的 SPEC_CONTRIB = {shader_spec}，CPU 参考实现用 {cpu_spec}：\
             两者必须同时出现在 diffuse + K*spec 这一个式子里"
        );
    }
    /// 画质 M5 判据：**雾色与天空地平线色必须同源**（2026-10-22 天空渐变落地时立）。
    ///
    /// 背景：天空 WGSL 与片元雾色一度各写一份色，远处楼雾化成深蓝而地平线天空是灰白，
    /// 出现"楼—天"接缝。修法 = `build.rs` 的 `SKY_HORIZON_XYZ` 作为唯一真源，构建期
    /// 同时注入两份 WGSL。本判据关注三件事：① 占位符还在（没被改回硬编码）；
    /// ② 注入用的确实是同一个常量；③ 生成出来的运行时常量与天空一致。
    #[test]
    fn sky_and_fog_colors_share_one_source() {
        let build = include_str!("../../build.rs");
        assert!(
            build.contains("const FOG_TINT: vec3<f32> = vec3<f32>({FOG_TINT});"),
            "片元雾色的 FOG_TINT 不再是构建期占位符（有人在 WGSL 里写死了颜色）"
        );
        assert!(
            build.contains(".replace(\"{FOG_TINT}\", &vec3_lit(SKY_HORIZON_XYZ))"),
            "FOG_TINT 的注入源不是 SKY_HORIZON_XYZ —— 雾色与天空地平线会分叉"
        );
        assert!(
            !build.contains("vec3<f32>(0.24, 0.36, 0.60)"),
            "build.rs 里仍有旧的硬编码天空/雾色 (0.24, 0.36, 0.60)"
        );
        // 生成出来的常量（build.rs 写入 OUT_DIR/shaders.rs）：雾色 == 地平线色
        let fog = crate::shaders::FOG_TINT_RGB;
        let horizon = crate::shaders::SKY_HORIZON_RGB;
        assert_eq!(fog, horizon, "雾色与天空地平线色不同源：{fog:?} vs {horizon:?}");
        for c in [fog, horizon, crate::shaders::SKY_ZENITH_RGB, crate::shaders::SKY_BELOW_RGB] {
            for v in c {
                assert!(
                    (0.0..=1.0).contains(&v),
                    "线性空间颜色分量越界（>1 会被 sRGB 编码钳掉）：{c:?}"
                );
            }
        }
        // 天顶必须比地平线暗（否则"天空渐变"是反的）
        let lum = |c: [f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
        assert!(
            lum(crate::shaders::SKY_ZENITH_RGB) < lum(horizon),
            "天顶亮度 {} 不小于地平线 {} —— 渐变方向反了",
            lum(crate::shaders::SKY_ZENITH_RGB),
            lum(horizon)
        );
    }


    /// 太阳方向：单位长，且 y 分量 = sin(elev)（与阴影/天空共用同一向量）。
    #[test]
    fn sun_direction_is_unit_and_follows_elevation() {
        for (e, a) in [(8.0f32, 0.0f32), (45.0, 90.0), (61.0, 233.0)] {
            let d = super::sun_direction(e, a);
            assert!((d.length() - 1.0).abs() < 1e-5, "不是单位向量：{d:?}");
            assert!((d.y - e.to_radians().sin()).abs() < 1e-4, "elev 不对：{d:?}");
        }
    }

    /// 低太阳必须更暖、更弱（10° vs 60°）。反向验证：把 warm 换成 noon 色 ⇒ 红。
    #[test]
    fn low_sun_is_warmer_and_dimmer() {
        let (lo, ilo) = super::sun_tint(10.0);
        let (hi, ihi) = super::sun_tint(60.0);
        assert!(lo.z < hi.z, "低太阳的蓝分量没有降低（不暖）：{lo:?} vs {hi:?}");
        assert!(ilo < ihi, "低太阳强度没有降低：{ilo} vs {ihi}");
        assert!((ihi - 1.0).abs() < 1e-5, "60° 应回到满强度：{ihi}");
    }

    /// 路灯挑选必须按距离取最近的两个（不是"表里前两个"）。
    /// 反向验证：改成 `idx.truncate(n)` 不排序 ⇒ 红。
    #[test]
    fn nearest_lamps_are_sorted_by_distance() {
        let lamps = [[0.0, 4.4, 100.0], [50.0, 4.4, 50.0], [1.0, 4.4, 1.0]];
        let picked = super::pick_nearest_lamps(&lamps, [0.0, 0.0], 2);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0], [1.0, 4.4, 1.0], "最近的一个不是 (1,1)");
        assert_eq!(picked[1], [50.0, 4.4, 50.0], "第二个应为 (50,50)");
    }

    /// 夜间视距：白天 1.0、夜里 0.55、且随太阳高度单调。反向验证：改成恒 1.0 ⇒ 红。
    #[test]
    fn vision_factor_drops_at_night() {
        assert!((super::vision_factor_for_sun(0.90) - 1.0).abs() < 1e-5, "白天应为满视距");
        assert!((super::vision_factor_for_sun(0.0) - 0.55).abs() < 1e-5, "夜里应为 0.55");
        let a = super::vision_factor_for_sun(0.05);
        let b = super::vision_factor_for_sun(0.20);
        assert!(a < b, "视距应随太阳高度单调上升：{a} vs {b}");
    }

    /// 手电必须在相机前方（yaw=0 ⇒ -Z；yaw=90° ⇒ -X），并略低于眼位。
    /// 反向验证：改成返回相机位置 ⇒ 红。
    #[test]
    fn flashlight_sits_ahead_of_the_camera() {
        let p = super::flashlight_position([0.0, 1.7, 0.0], 0.0);
        assert!(p[0].abs() < 1e-4 && (p[2] + 1.5).abs() < 1e-4, "yaw=0 时应在 -Z：{p:?}");
        assert!((p[1] - 1.55).abs() < 1e-4, "应比眼位低 0.15：{p:?}");
        let q = super::flashlight_position([0.0, 1.7, 0.0], std::f32::consts::FRAC_PI_2);
        assert!((q[0] + 1.5).abs() < 1e-4 && q[2].abs() < 1e-4, "yaw=90° 时应在 -X：{q:?}");
    }

    /// 电量推进：开灯按 on_secs 用光、关灯按 off_secs 回充、两端夹紧。
    /// 反向验证：把开灯分支改成返回 b ⇒ 红。
    #[test]
    fn battery_drains_and_recharges() {
        let b = super::battery_step(1.0, true, 60.0, 180.0, 240.0);
        assert!((b - (1.0 - 60.0 / 180.0)).abs() < 1e-5, "开灯 60s 应耗 1/3：{b}");
        let e = super::battery_step(0.01, true, 60.0, 180.0, 240.0);
        assert_eq!(e, 0.0, "电量不能为负");
        let r = super::battery_step(0.0, false, 120.0, 180.0, 240.0);
        assert!((r - 0.5).abs() < 1e-5, "关灯 120s 应回充一半：{r}");
        let f = super::battery_step(0.9, false, 600.0, 180.0, 240.0);
        assert_eq!(f, 1.0, "回充不能超过 1");
    }

    /// 手电亮度随电量下降（而不是突然熄灭）：满电 1.0/14m，空电 0.25/8m。
    /// 反向验证：把 `battery` 改成常数 1.0 ⇒ 红。
    #[test]
    fn flashlight_dims_with_battery() {
        let (hi, r_hi) = super::flashlight_dim(1.0);
        let (lo, r_lo) = super::flashlight_dim(0.0);
        assert!((hi - 1.0).abs() < 1e-5 && (r_hi - 14.0).abs() < 1e-5);
        assert!((lo - 0.25).abs() < 1e-5 && (r_lo - 8.0).abs() < 1e-5);
        let (mid, _) = super::flashlight_dim(0.25);
        assert!(lo < mid && mid < hi, "亮度应随电量单调：{lo} {mid} {hi}");
    }

    /// 夜间开手电 ⇒ AI 视距 ×1.25（白天开灯无效，因为白天本来满视距）。
    /// 反向验证：忽略 `player_lit` ⇒ 红。
    #[test]
    fn flashlight_makes_the_player_easier_to_see_at_night() {
        assert!((super::ai_sight_factor(1.0, true) - 1.25).abs() < 1e-5);
        assert!((super::ai_sight_factor(0.55, false) - 0.55).abs() < 1e-5);
        assert!((super::ai_sight_factor(0.55, true) - 0.6875).abs() < 1e-5);
    }
}
