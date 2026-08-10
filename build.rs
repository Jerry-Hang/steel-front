/// 钢铁前线 (Steel Front) - 构建脚本
/// 将 WGSL 着色器源代码编译为内联 SPIR-V 字节数组
use std::env;
use std::fs;
use std::path::Path;

const VERTEX_SHADER_WGSL: &str = r#"
struct ViewProj {
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    // lod_params = (lod_dist, fade_start, fade_end, 0)
    lod_params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: ViewProj;

struct Instance {
    model: mat4x4<f32>,
    tint: vec4<f32>,
}
@group(0) @binding(2) var<storage, read> instances: array<Instance>;

// 地形 draw 使用的保留 identity 实例索引（buffer 最后一个 slot），不参与 LOD 淡出
const TERRAIN_INSTANCE_INDEX: u32 = 65536u;
// NPC 士兵段实例起始槽（与 renderer.rs NPC_SLOT_BASE 一致：65536 identity + 64 marker 之后）。
// 槽位 >= 该值的实例走「纯色渲染」路径：不混合贴图，保证红/蓝阵营色清晰可辨。
const NPC_INSTANCE_BASE: u32 = 65536u + 1u + 64u;
// 槽位 >= 该值的实例为「自发光」实体（爆炸闪光等）：片元跳过光照与贴图混合，直出纯色。
// 必须与 renderer.rs 的 EMISSIVE_SLOT_BASE（NPC_SLOT_BASE + MAX_NPC_INSTANCES）同步。
const EMISSIVE_INSTANCE_BASE: u32 = NPC_INSTANCE_BASE + 1024u;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) fade: f32,
    @location(3) world_pos: vec3<f32>,
    @location(4) view_dir: vec3<f32>,
    @location(5) flat_flag: f32,
}
@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var output: VertexOutput;
    let inst = instances[instance_index];
    let world_pos = inst.model * vec4<f32>(position, 1.0);
    output.position = camera.proj * camera.view * world_pos;
    output.world_pos = world_pos.xyz;
    // 相机世界位置：view = [R|t]，相机位置 = -R^T * t（刚体变换）
    let t = camera.view[3].xyz;
    let cam_pos = -(camera.view[0].xyz * t.x + camera.view[1].xyz * t.y + camera.view[2].xyz * t.z);
    // 片元光照使用：表面 → 相机方向
    output.view_dir = normalize(cam_pos - world_pos.xyz);
    output.color = color * inst.tint.rgb;
    output.uv = uv;
    if (instance_index >= NPC_INSTANCE_BASE) {
        output.flat_flag = 1.0;
    } else {
        output.flat_flag = 0.0;
    }
    if (instance_index >= EMISSIVE_INSTANCE_BASE) {
        // 自发光实体：fade > 1 作为 emissive 信号（片元直出颜色，跳过光照/贴图混合）
        output.flat_flag = 1.0;
        output.fade = 2.0;
    } else if (instance_index == TERRAIN_INSTANCE_INDEX) {
        output.fade = 1.0;
    } else {
        // 地面平面距离（不随相机高度变化，俯瞰全场时不误淡出）
        let center = inst.model[3].xyz;
        let dx = center.x - cam_pos.x;
        let dz = center.z - cam_pos.z;
        let dist = sqrt(dx * dx + dz * dz);
        output.fade = 1.0 - smoothstep(camera.lod_params.y, camera.lod_params.z, dist);
    }
    return output;
}
"#;

/// 片元着色器 - 接收颜色并输出
const FRAGMENT_SHADER_WGSL: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) fade: f32,
    @location(3) world_pos: vec3<f32>,
    @location(4) view_dir: vec3<f32>,
    @location(5) flat_flag: f32,
}

@group(0) @binding(1) var texture_sampled: texture_2d<f32>;
@group(0) @binding(3) var texture_sampler: sampler;

// ---- 光照 Uniform（默认全零 = 光照关闭，保持原混合渲染向后兼容）----
struct DirectionalLight {
    // xyz = 表面→光源方向, w = enabled(1.0/0.0)
    direction: vec4<f32>,
    // rgb = 颜色, w = 强度
    color_intensity: vec4<f32>,
}
struct PointLight {
    // xyz = 世界位置, w = enabled(1.0/0.0)
    position: vec4<f32>,
    // rgb = 颜色, w = 强度
    color_intensity: vec4<f32>,
    // x = constant, y = linear, z = quadratic, w = range
    attenuation: vec4<f32>,
}
struct ShadowInfo {
    // 光空间 view-proj（世界空间 → 光裁剪空间）
    light_view_proj: mat4x4<f32>,
    // x = depth_bias, y = normal_bias, z = enabled, w = 0
    bias: vec4<f32>,
    // x = shadow map 尺寸, y/z/w = 0
    config: vec4<f32>,
}
struct LightUniform {
    // x = lighting enabled, y = shadow enabled, z/w = 0
    flags: vec4<f32>,
    // rgb = 环境色, w = 环境强度
    ambient: vec4<f32>,
    directional: DirectionalLight,
    points: array<PointLight, 4>,
    shadow: ShadowInfo,
}
@group(0) @binding(4) var<uniform> light_data: LightUniform;

/// Blinn-Phong 漫反射：max(dot(n, l), 0)
fn bp_diffuse(normal: vec3<f32>, light_dir: vec3<f32>) -> f32 {
    return max(dot(normal, light_dir), 0.0);
}

/// Blinn-Phong 高光：pow(max(dot(n, h), 0), shininess)，h = normalize(l + v)
fn bp_specular(normal: vec3<f32>, light_dir: vec3<f32>, view_dir: vec3<f32>, shininess: f32) -> f32 {
    let half_dir = normalize(light_dir + view_dir);
    return pow(max(dot(normal, half_dir), 0.0), shininess);
}

/// 点光源衰减：1 / (c + l*d + q*d*d)，clamp 到 [0, 1]
fn point_attenuation(dist: f32, constant: f32, linear: f32, quadratic: f32) -> f32 {
    let denom = constant + linear * dist + quadratic * dist * dist;
    if (denom <= 0.0) {
        return 1.0;
    }
    return min(1.0, 1.0 / denom);
}

/// 阴影深度比较：1.0 = 阴影，0.0 = 照亮（bias 缓解 acne）
fn shadow_test(shadow_depth: f32, fragment_depth: f32, bias: f32) -> f32 {
    if (fragment_depth - bias > shadow_depth) {
        return 1.0;
    }
    return 0.0;
}

fn evaluate_directional(light: DirectionalLight, normal: vec3<f32>, view_dir: vec3<f32>, shininess: f32) -> vec3<f32> {
    if (light.direction.w < 0.5) {
        return vec3<f32>(0.0);
    }
    let light_dir = normalize(light.direction.xyz);
    let diffuse = bp_diffuse(normal, light_dir);
    let spec = bp_specular(normal, light_dir, view_dir, shininess);
    return light.color_intensity.xyz * light.color_intensity.w * (diffuse + 0.4 * spec);
}

fn evaluate_point(light: PointLight, world_pos: vec3<f32>, normal: vec3<f32>, view_dir: vec3<f32>, shininess: f32) -> vec3<f32> {
    if (light.position.w < 0.5) {
        return vec3<f32>(0.0);
    }
    let to_light = light.position.xyz - world_pos;
    let dist = length(to_light);
    if (light.attenuation.w > 0.0 && dist > light.attenuation.w) {
        return vec3<f32>(0.0);
    }
    let light_dir = to_light / dist;
    let diffuse = bp_diffuse(normal, light_dir);
    let spec = bp_specular(normal, light_dir, view_dir, shininess);
    let atten = point_attenuation(dist, light.attenuation.x, light.attenuation.y, light.attenuation.z);
    return light.color_intensity.xyz * light.color_intensity.w * atten * (diffuse + 0.4 * spec);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if (input.fade <= 0.02) {
        discard;
    }
    // 自发光（爆炸闪光等，顶点阶段 fade=2.0 标记）：直出顶点色 × tint，不受光照影响
    if (input.fade > 1.0) {
        return vec4<f32>(input.color, 1.0);
    }
    // 士兵等纯色实例（NPC 槽位）：直接输出顶点色 × fade，跳过贴图 50% 混合，
    // 保证红/蓝阵营色在灰地场景中清晰可辨（marker/地形仍走纹理混合路径）。
    if (input.flat_flag > 0.5) {
        return vec4<f32>(input.color * input.fade, 1.0);
    }
    let texel = textureSample(texture_sampled, texture_sampler, input.uv);
    let mixed = mix(input.color, texel.rgb, 0.5) * input.fade;

    // 默认关闭：light UBO 全零（flags.x = 0）时保持原「纹理+顶点颜色 50% 混合」渲染
    if (light_data.flags.x < 0.5) {
        return vec4<f32>(mixed, 1.0);
    }

    // 法线：顶点数据无法线，用世界坐标的屏幕导数求面法线，并朝向相机（双面凸体近似）
    let view_dir = normalize(input.view_dir);
    var normal = normalize(cross(dpdx(input.world_pos), dpdy(input.world_pos)));
    if (dot(normal, view_dir) < 0.0) {
        normal = -normal;
    }

    // 阴影因子（方向光）：光空间深度比较 + bias。
    // 基础实现未接入 shadow map 贴图，占位采样深度 1.0（远平面外 = 无遮挡）。
    var shadow_factor = 0.0;
    if (light_data.flags.y >= 0.5 && light_data.shadow.bias.z >= 0.5) {
        let sp = light_data.shadow.light_view_proj * vec4<f32>(input.world_pos, 1.0);
        let shadow_uv = sp.xy * 0.5 + vec2<f32>(0.5, 0.5);
        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0
            && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0
            && sp.z >= 0.0 && sp.z <= 1.0) {
            shadow_factor = shadow_test(1.0, sp.z, light_data.shadow.bias.x);
        }
    }

    // Blinn-Phong 光照：环境光 + 方向光（乘阴影因子）+ 点光源（最多 4 个）
    let shininess = 32.0;
    var radiance = light_data.ambient.rgb * light_data.ambient.w;
    radiance = radiance + evaluate_directional(light_data.directional, normal, view_dir, shininess) * (1.0 - shadow_factor);
    for (var i = 0u; i < 4u; i = i + 1u) {
        radiance = radiance + evaluate_point(light_data.points[i], input.world_pos, normal, view_dir, shininess);
    }
    let lit = mixed * min(radiance, vec3<f32>(1.0));
    return vec4<f32>(lit, 1.0);
}
"#;

fn compile_wgsl(source: &str) -> Vec<u32> {
    let module = naga::front::wgsl::parse_str(source).expect("WGSL 着色器解析失败");
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::default(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("WGSL 着色器验证失败");
    let options = naga::back::spv::Options::default();
    naga::back::spv::write_vec(&module, &info, &options, None).expect("SPIR-V 生成失败")
}

/// HUD 覆盖层顶点着色器：屏幕空间直通（位置已由 CPU 转为 NDC，Y 翻转完成）
const HUD_VERTEX_SHADER_WGSL: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.color = color;
    return output;
}
"#;

/// HUD 覆盖层片元着色器：直接输出顶点色（alpha 混合由管线状态控制）
const HUD_FRAGMENT_SHADER_WGSL: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
"#;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let vs_spirv = compile_wgsl(VERTEX_SHADER_WGSL);
    let fs_spirv = compile_wgsl(FRAGMENT_SHADER_WGSL);
    let hud_vs_spirv = compile_wgsl(HUD_VERTEX_SHADER_WGSL);
    let hud_fs_spirv = compile_wgsl(HUD_FRAGMENT_SHADER_WGSL);
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR 环境变量未设置");
    let dest_path = Path::new(&out_dir).join("shaders.rs");
    let mut output = String::new();
    output.push_str("// 自动生成的着色器 SPIR-V 数据 - 请勿手动修改\n\n");
    output.push_str("/// 顶点着色器 SPIR-V 字节码\n");
    output.push_str("#[allow(dead_code)]\n");
    output.push_str(&format!("pub const VS_SPIRV: &[u32] = &{:?};\n\n", vs_spirv));
    output.push_str("/// 片元着色器 SPIR-V 字节码\n");
    output.push_str("#[allow(dead_code)]\n");
    output.push_str(&format!("pub const FS_SPIRV: &[u32] = &{:?};\n", fs_spirv));
    output.push_str("/// HUD 顶点着色器 SPIR-V 字节码\n");
    output.push_str("#[allow(dead_code)]\n");
    output.push_str(&format!("pub const HUD_VS_SPIRV: &[u32] = &{:?};\n\n", hud_vs_spirv));
    output.push_str("/// HUD 片元着色器 SPIR-V 字节码\n");
    output.push_str("#[allow(dead_code)]\n");
    output.push_str(&format!("pub const HUD_FS_SPIRV: &[u32] = &{:?};\n", hud_fs_spirv));
    fs::write(&dest_path, &output).expect("写入着色器数据失败");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR 未设置");
    let assets_dir = Path::new(&manifest_dir).join("assets");
    fs::create_dir_all(&assets_dir).expect("创建 assets 目录失败");
    let write_spv = |spirv: &[u32], name: &str| {
        let bytes: Vec<u8> = spirv.iter().flat_map(|w| w.to_le_bytes()).collect();
        let spv_path = assets_dir.join(name);
        fs::write(&spv_path, &bytes).unwrap_or_else(|e| panic!("写入 {} 失败: {}", name, e));
        println!("cargo:info=写入 SPIR-V 文件: {:?} ({} 字节)", spv_path, bytes.len());
    };
    write_spv(&vs_spirv, "triangle.vert.spv");
    write_spv(&fs_spirv, "triangle.frag.spv");
    write_spv(&hud_vs_spirv, "hud.vert.spv");
    write_spv(&hud_fs_spirv, "hud.frag.spv");
    println!("cargo:info=着色器编译完成");
}
