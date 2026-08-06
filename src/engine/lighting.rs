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
pub const SHADOW_MAP_SIZE: u32 = 2048;
/// Shadow map 深度格式（对应 Vulkan `VK_FORMAT_D32_SFLOAT` = 126）
pub const SHADOW_MAP_FORMAT: u32 = 126;
/// 默认阴影深度 bias（缓解 shadow acne）
pub const DEFAULT_SHADOW_DEPTH_BIAS: f32 = 0.005;
/// 默认阴影法线 bias
pub const DEFAULT_SHADOW_NORMAL_BIAS: f32 = 0.02;
/// 默认高光指数（与 WGSL 一致）
pub const DEFAULT_SHININESS: f32 = 32.0;
/// 高光强度系数（与 WGSL 一致）
pub const SPECULAR_STRENGTH: f32 = 0.4;

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
    /// 光线传播方向（如正午太阳光为 -Y）
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
            map_size: SHADOW_MAP_SIZE,
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
pub fn blinn_phong_diffuse(normal: Vec3, light_dir: Vec3) -> f32 {
    normal.dot(light_dir).max(0.0)
}

/// Blinn-Phong 高光系数：`pow(max(dot(n, h), 0), shininess)`，`h = normalize(l + v)`
pub fn blinn_phong_specular(normal: Vec3, light_dir: Vec3, view_dir: Vec3, shininess: f32) -> f32 {
    let half = (light_dir + view_dir).normalize_or_zero();
    normal.dot(half).max(0.0).powf(shininess)
}

/// 点光源衰减：`1 / (c + l*d + q*d²)`，clamp 到 [0, 1]
pub fn point_attenuation(distance: f32, constant: f32, linear: f32, quadratic: f32) -> f32 {
    let denom = constant + linear * distance + quadratic * distance * distance;
    if denom <= 0.0 {
        1.0
    } else {
        (1.0 / denom).min(1.0)
    }
}

/// 方向光辐射度（与 WGSL `evaluate_directional` 一致）
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
/// `light_dir` 为光线传播方向；以 `target` 为中心、`extent` 为半宽/半高，
/// 视线沿光线方向观察场景，把场景中心置于光视锥中段。
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

/// 把世界坐标投影到光空间：返回 (shadow uv, 光空间深度)
pub fn world_to_shadow_uv(world_pos: Vec3, shadow_vp: Mat4) -> (Vec2, f32) {
    let p = shadow_vp * world_pos.extend(1.0);
    let uv = Vec2::new(p.x * 0.5 + 0.5, p.y * 0.5 + 0.5);
    (uv, p.z)
}

/// 阴影深度比较（bias 缓解 acne）：`fragment_depth - bias > shadow_depth` 判定为阴影
pub fn shadow_depth_test(shadow_depth: f32, fragment_depth: f32, depth_bias: f32) -> bool {
    fragment_depth - depth_bias > shadow_depth
}

/// 阴影可见性：0.0 = 阴影，1.0 = 照亮（与片元着色器 `1.0 - shadow_test(...)` 语义一致）
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
        // 沿光线传播方向（向下）更远处 → 深度单调增大
        let far_depth = world_to_shadow_uv(Vec3::new(0.0, -10.0, 0.0), vp).1;
        let near_depth = world_to_shadow_uv(Vec3::new(0.0, 10.0, 0.0), vp).1;
        assert!(far_depth > near_depth);
        // 覆盖范围内 → uv 落在 [0,1]
        let (uv2, _) = world_to_shadow_uv(Vec3::new(30.0, 0.0, 30.0), vp);
        assert!(uv2.x > 0.0 && uv2.x < 1.0 && uv2.y > 0.0 && uv2.y < 1.0);
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
        assert_eq!(u.shadow.config.x, SHADOW_MAP_SIZE as f32);
        assert_ne!(u.shadow.light_view_proj, Mat4::ZERO);
    }

    #[test]
    fn build_with_no_lights_stays_disabled() {
        let u = LightUniform::build(None, &[], Vec3::ZERO, 0.0, None);
        assert_eq!(u, LightUniform::default());
    }
}
