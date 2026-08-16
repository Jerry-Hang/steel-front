/// 钢铁前线 (Steel Front) - 构建脚本
/// 将 WGSL 着色器源代码编译为内联 SPIR-V 字节数组
use std::env;
use std::fs;
use std::path::Path;

// ⛔ 传统顶点着色器【已冻结维护】（2026-08-16）：仅作 WSLg/dzn 无 mesh 扩展回退，
// 不再新增功能。新功能一律走 MESH_SHADER_WGSL 网格着色器路径。
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
// 世界障碍 marker 起始槽（与 renderer.rs MARKER_SLOT_BASE 一致：65536 identity 之后）。
// 槽位 >= 该值的实例（marker/NPC/自发光）不采样地面贴图，走「材质编码」路径：
// flat_flag = 1（marker）/ 2（NPC）由片元着色器决定采样程序化皮肤纹理还是纯色
// （RV3D_SKIN_TEX=1 启用皮肤纹理，缺省 0 保持纯 tint 色，冒烟基线不变）。
const MARKER_INSTANCE_BASE: u32 = 65536u + 1u;
// NPC 士兵段实例起始槽（与 renderer.rs NPC_SLOT_BASE 一致：65536 identity + 64 marker 之后）。
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
    // 材质编码（片元着色器按值分路径）：
    //   0 = 地面/地形（world-space UV 采样地面材质）
    //   1 = marker 障碍（RV3D_SKIN_TEX=1 采样木板墙皮肤，缺省纯 tint 色）
    //   2 = NPC 士兵（RV3D_SKIN_TEX=1 采样迷彩军服皮肤，缺省纯阵营色）
    if (instance_index >= NPC_INSTANCE_BASE) {
        output.flat_flag = 2.0;
    } else if (instance_index >= MARKER_INSTANCE_BASE) {
        output.flat_flag = 1.0;
    } else {
        output.flat_flag = 0.0;
    }
    // 枪模专用 identity 槽（renderer.rs GUN_INSTANCE_INDEX = 65536+1+64+1024+64）：
    // 走 marker 纯色路径（flat=1），避免被地面纹理 0.75 混合 → 枪模隐形（2026-08-16）
    if (instance_index == 66689u) {
        output.flat_flag = 1.0;
        output.fade = 1.0;
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
// 程序化皮肤纹理（marker/NPC；RV3D_SKIN_TEX=1 时采样，缺省 0 纯色回退。
// 绑定号必须与 renderer.rs init_descriptors / update_texture_descriptor_sets 同步）
@group(0) @binding(7) var marker_skin_tex: texture_2d<f32>;
@group(0) @binding(8) var npc_skin_tex: texture_2d<f32>;
// 阴影贴图（2026-08-11）：depth-only pass 渲光空间深度，片元 3x3 PCF 深度比较
@group(0) @binding(5) var shadow_map: texture_depth_2d;
@group(0) @binding(6) var shadow_sampler: sampler;

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
    // marker/NPC 材质编码路径：flat_flag=1（marker）/ 2（NPC），按顶点 UV 采样
    // 程序化皮肤纹理（立方体每面 UV 铺满 0..1，与 procedural.rs 皮肤纹理逐面对齐）。
    // RV3D_SKIN_TEX=1（light_data.flags.z）启用；缺省 0 保持纯色路径（冒烟基线不变）。
    if (input.flat_flag > 0.5) {
        if (light_data.flags.z >= 0.5) {
            if (input.flat_flag > 1.5) {
                // NPC 士兵：迷彩军服纹理 × 阵营 tint（权重 0.65，阵营识别保留）
                let texel = textureSample(npc_skin_tex, texture_sampler, input.uv);
                let colored = mix(input.color, texel.rgb, 0.65) * input.fade;
                return vec4<f32>(colored, 1.0);
            } else {
                // marker 障碍：军备木板墙纹理 × 障碍 tint（权重 0.8，材质可辨识）
                let texel = textureSample(marker_skin_tex, texture_sampler, input.uv);
                let colored = mix(input.color, texel.rgb, 0.8) * input.fade;
                return vec4<f32>(colored, 1.0);
            }
        }
        return vec4<f32>(input.color * input.fade, 1.0);
    }
    // 世界空间 UV：地面/地形用 world_pos.xz 映射到全图 [0,1]（覆盖 2*256 米），
    // 与 procedural.rs 烘焙纹理严格对齐（marker/NPC/自发光已走 flat_flag 纯色路径，不采样）。
    let world_uv = (input.world_pos.xz + vec2<f32>(256.0, 256.0)) / 512.0;
    let texel = textureSample(texture_sampled, texture_sampler, world_uv);
    // 地面/地形：纹理占主导（0.75），顶点 tint 色仅轻微着色，保证材质配色可辨识
    let mixed = mix(input.color, texel.rgb, 0.75) * input.fade;

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

    // 阴影因子（方向光）：shadow map 3x3 PCF 深度比较 + bias。
    // 阴影图由 depth-only pass 渲染：顶点输出经 ADJUST_COORDINATE_SPACE Y 翻转，
    // 光空间 clip.y → 帧缓冲行 y_img=(1-clip.y)/2*H，采样 V 必须同式镜像
    // （uv.y=1-(clip.y*0.5+0.5)，否则阴影整体前后颠倒）；glam ortho_rh 的 clip.z∈[0,1]
    // 经 viewport 映射到深度 [0.5,1]，比较前同样映射 frag_depth = clip.z*0.5+0.5。
    var shadow_factor = 0.0;
    if (light_data.flags.y >= 0.5 && light_data.shadow.bias.z >= 0.5) {
        let sp = light_data.shadow.light_view_proj * vec4<f32>(input.world_pos, 1.0);
        let shadow_uv = vec2<f32>(sp.x * 0.5 + 0.5, 1.0 - (sp.y * 0.5 + 0.5));
        let frag_depth = sp.z * 0.5 + 0.5;
        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0
            && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0
            && sp.z >= 0.0 && sp.z <= 1.0) {
            let texel = 1.0 / light_data.shadow.config.x;
            var occluded = 0.0;
            for (var dy = -1; dy <= 1; dy = dy + 1) {
                for (var dx = -1; dx <= 1; dx = dx + 1) {
                    let d = textureSample(shadow_map, shadow_sampler,
                        shadow_uv + vec2<f32>(f32(dx), f32(dy)) * texel);
                    if (frag_depth - light_data.shadow.bias.x > d) {
                        occluded = occluded + 1.0;
                    }
                }
            }
            shadow_factor = occluded / 9.0;
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

/// 网格着色器（VK_EXT_mesh_shader，可选路径）：
/// 每个 workgroup 负责一个实例槽位，GPU 端逐实例视锥剔除 + 顶点变换，
/// 输出与顶点着色器完全一致的 VertexOutput（片元着色器原样复用）。
/// 仅当物理设备支持 VK_EXT_mesh_shader 时才被启用（WSLg/dzn 实测不支持，
/// 本机回退传统顶点管线）。SPIR-V 输出要求 >= 1.4（MeshShadingEXT）。
const MESH_SHADER_WGSL: &str = r#"
enable wgpu_mesh_shader;

struct MeshCamera {
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    // lod_params = (lod_dist, fade_start, fade_end, 0)
    lod_params: vec4<f32>,
    // 视锥 6 平面（Gribb–Hartmann，法线朝内、归一化；与 renderer.rs CPU 剔除同源）
    planes: array<vec4<f32>, 6>,
    // xyz = 相机世界位置，w = 近档距离²（几何 LOD 切换阈值，随画质预设变化）
    cam_pos: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: MeshCamera;

struct Instance {
    model: mat4x4<f32>,
    tint: vec4<f32>,
}
@group(0) @binding(2) var<storage, read> instances: array<Instance>;

// 槽位约定（必须与 renderer.rs 常量同步）：
// TERRAIN_INSTANCE_INDEX=65536（地形 identity，mesh 路径不绘制该槽）、
// MARKER_INSTANCE_BASE=65537、NPC_INSTANCE_BASE=65536+1+64=65601、
// EMISSIVE_INSTANCE_BASE=NPC_INSTANCE_BASE+1024=66625
const TERRAIN_INSTANCE_INDEX: u32 = 65536u;
const MARKER_INSTANCE_BASE: u32 = 65536u + 1u;
const NPC_INSTANCE_BASE: u32 = 65536u + 1u + 64u;
const EMISSIVE_INSTANCE_BASE: u32 = NPC_INSTANCE_BASE + 1024u;

// 与顶点着色器输出逐成员一致（片元着色器原样复用，location 0..5 不可改）
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) fade: f32,
    @location(3) world_pos: vec3<f32>,
    @location(4) view_dir: vec3<f32>,
    @location(5) flat_flag: f32,
}

struct MeshPrimitive {
    @builtin(triangle_indices) indices: vec3<u32>,
}

// 最大几何 = 近档立方体（24 顶点 / 12 三角形）
struct MeshOutput {
    @builtin(vertex_count) vertex_count: u32,
    @builtin(primitive_count) primitive_count: u32,
    @builtin(vertices) vertices: array<VertexOutput, 24>,
    @builtin(primitives) primitives: array<MeshPrimitive, 12>,
}
var<workgroup> mesh_out: MeshOutput;

// 本次 mesh draw 的起始实例槽：地面=0 / marker=65537 / NPC=65601 / 自发光=66625
struct MeshPush {
    base_slot: u32,
    // 填充到 16 字节（与 renderer.rs push constant range size=16 精确一致）
    pad: array<u32, 3>,
}
// naga 30 用 immediate 地址空间替代 push_constant（SPIR-V 后端映射为 PushConstant 存储类）
var<immediate> mesh_push: MeshPush;

// ---- 几何表：与 renderer.rs 顶点缓冲数据逐值一致（模型空间）----
// 地面平铺 quad（GROUND_VERTS + GROUND_INDICES，绕序 [0,2,1,0,3,2]）
const GROUND_POS: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(-1.0, 0.0, 1.0),
    vec3<f32>(1.0, 0.0, 1.0),
    vec3<f32>(1.0, 0.0, -1.0),
    vec3<f32>(-1.0, 0.0, -1.0),
);
const GROUND_UV: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 1.0),
);
const GROUND_TRI: array<vec3<u32>, 2> = array<vec3<u32>, 2>(
    vec3<u32>(0u, 2u, 1u),
    vec3<u32>(0u, 3u, 2u),
);

// 立方体 24 顶点（VERTICES，每面 UV 铺满 0..1；顶点色白化后 color = tint）
const CUBE_POS: array<vec3<f32>, 24> = array<vec3<f32>, 24>(
    vec3<f32>(-1.0, -1.0, 1.0), vec3<f32>(1.0, -1.0, 1.0),
    vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(-1.0, 1.0, 1.0),
    vec3<f32>(1.0, -1.0, -1.0), vec3<f32>(-1.0, -1.0, -1.0),
    vec3<f32>(-1.0, 1.0, -1.0), vec3<f32>(1.0, 1.0, -1.0),
    vec3<f32>(1.0, -1.0, 1.0), vec3<f32>(1.0, -1.0, -1.0),
    vec3<f32>(1.0, 1.0, -1.0), vec3<f32>(1.0, 1.0, 1.0),
    vec3<f32>(-1.0, -1.0, -1.0), vec3<f32>(-1.0, -1.0, 1.0),
    vec3<f32>(-1.0, 1.0, 1.0), vec3<f32>(-1.0, 1.0, -1.0),
    vec3<f32>(-1.0, 1.0, 1.0), vec3<f32>(1.0, 1.0, 1.0),
    vec3<f32>(1.0, 1.0, -1.0), vec3<f32>(-1.0, 1.0, -1.0),
    vec3<f32>(-1.0, -1.0, -1.0), vec3<f32>(1.0, -1.0, -1.0),
    vec3<f32>(1.0, -1.0, 1.0), vec3<f32>(-1.0, -1.0, 1.0),
);
const CUBE_UV: array<vec2<f32>, 24> = array<vec2<f32>, 24>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
);
const CUBE_TRI: array<vec3<u32>, 12> = array<vec3<u32>, 12>(
    vec3<u32>(0u, 1u, 2u), vec3<u32>(0u, 2u, 3u),
    vec3<u32>(4u, 5u, 6u), vec3<u32>(4u, 6u, 7u),
    vec3<u32>(8u, 9u, 10u), vec3<u32>(8u, 10u, 11u),
    vec3<u32>(12u, 13u, 14u), vec3<u32>(12u, 14u, 15u),
    vec3<u32>(16u, 17u, 18u), vec3<u32>(16u, 18u, 19u),
    vec3<u32>(20u, 21u, 22u), vec3<u32>(20u, 22u, 23u),
);

// 远档十字双 quad（FAR_VERTS + FAR_INDICES）
const CROSS_POS: array<vec3<f32>, 8> = array<vec3<f32>, 8>(
    vec3<f32>(-1.0, -1.0, 0.0), vec3<f32>(1.0, -1.0, 0.0),
    vec3<f32>(1.0, 1.0, 0.0), vec3<f32>(-1.0, 1.0, 0.0),
    vec3<f32>(0.0, -1.0, 1.0), vec3<f32>(0.0, -1.0, -1.0),
    vec3<f32>(0.0, 1.0, -1.0), vec3<f32>(0.0, 1.0, 1.0),
);
const CROSS_UV: array<vec2<f32>, 8> = array<vec2<f32>, 8>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
);
const CROSS_TRI: array<vec3<u32>, 4> = array<vec3<u32>, 4>(
    vec3<u32>(0u, 1u, 2u), vec3<u32>(0u, 2u, 3u),
    vec3<u32>(4u, 5u, 6u), vec3<u32>(4u, 6u, 7u),
);

fn write_vertex(
    lid: u32,
    pos: vec3<f32>,
    uv: vec2<f32>,
    inst: Instance,
    cam: vec3<f32>,
    fade: f32,
    flat: f32,
    is_gun: bool,
) {
    var v: VertexOutput;
    let wp = inst.model * vec4<f32>(pos, 1.0);
    v.position = camera.proj * camera.view * wp;
    // 枪模深度覆盖：第一人称枪槽（NPC 区末 16 槽）强制 z_clip=0（NDC z=0 →
    // 深度 0.5，恒小于世界几何），杜绝枪管/枪托被近墙/地形"穿模遮住"
    if (is_gun) {
        v.position.z = 0.0;
    }
    // 顶点色已白化：color = 白 × tint（与顶点着色器 color * inst.tint.rgb 一致）
    v.color = vec3<f32>(1.0) * inst.tint.rgb;
    v.uv = uv;
    v.fade = fade;
    v.world_pos = wp.xyz;
    // 相机世界位置：view = [R|t]，相机位置 = -R^T * t（与顶点着色器同源）
    v.view_dir = normalize(cam - wp.xyz);
    v.flat_flag = flat;
    mesh_out.vertices[lid] = v;
}

@mesh(mesh_out)
@workgroup_size(32)
fn mesh_main(
    @builtin(workgroup_id) wg_id: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let slot = mesh_push.base_slot + wg_id.x;
    let inst = instances[slot];
    let is_ground = slot < TERRAIN_INSTANCE_INDEX;
    // 包围球半径：地面 quad 半对角线×0.5（与 CPU instance_radii 一致）；
    // 立方体/远档十字覆盖 ±1 三轴 → sqrt(3)
    // 半径与 CPU instance_radii 一致：地面 2x2m quad 半对角线 sqrt(2)≈1.414
    // （2026-08-15 修正：旧 0.707 低估一半 → 屏幕四角地面被过早剔除穿帮）；
    // 其余几何 ±1 三轴 → sqrt(3)≈1.732
    let radius = select(1.7320508, 1.41421356, is_ground);
    let center = inst.model[3].xyz;

    // GPU 视锥剔除：与 CPU 同源（法线朝内平面，d = dot(n,c)+d，d < -r 剔除）
    var visible = true;
    for (var i = 0u; i < 6u; i = i + 1u) {
        let p = camera.planes[i];
        if (dot(p.xyz, center) + p.w < -radius) {
            visible = false;
            break;
        }
    }
    if (!visible) {
        if (lid == 0u) {
            mesh_out.vertex_count = 0u;
            mesh_out.primitive_count = 0u;
        }
        return;
    }

    let cam = camera.cam_pos.xyz;
    let delta = center - cam;
    let dist2 = dot(delta, delta);
    // 地面平面距离（不随相机高度变化，俯瞰全场时不误淡出）——与顶点着色器一致
    let hdist = sqrt(delta.x * delta.x + delta.z * delta.z);
    var fade = 1.0 - smoothstep(camera.lod_params.y, camera.lod_params.z, hdist);
    // 材质编码与顶点着色器一致：0=地面/地形、1=marker、2=NPC
    // （片元着色器按值决定采样哪张皮肤纹理；RV3D_SKIN_TEX=1 启用，缺省纯色）
    var flat = 0.0;
    if (slot >= EMISSIVE_INSTANCE_BASE) {
        // 自发光实体：fade > 1 作为 emissive 信号（片元直出颜色，跳过光照/贴图混合）
        flat = 1.0;
        fade = 2.0;
    } else if (slot >= NPC_INSTANCE_BASE) {
        flat = 2.0;
    } else if (slot >= MARKER_INSTANCE_BASE) {
        flat = 1.0;
    }
    // 枪模槽位 = NPC 区末 16 槽（与 renderer.rs set_first_person_gun 的
    // MAX_NPC_INSTANCES-16 起始一致）：深度覆盖标记。
    // 上界必须 < EMISSIVE_INSTANCE_BASE（自发光 66625+ 不得命中，否则
    // 爆炸/手雷/粒子被强制画最前——修复越界回归）
    let is_gun = slot >= NPC_INSTANCE_BASE + 1024u - 16u && slot < EMISSIVE_INSTANCE_BASE;

    if (is_ground) {
        if (lid < 4u) {
            write_vertex(lid, GROUND_POS[lid], GROUND_UV[lid], inst, cam, fade, flat, false);
        }
        if (lid < 2u) {
            mesh_out.primitives[lid].indices = GROUND_TRI[lid];
        }
        if (lid == 0u) {
            mesh_out.vertex_count = 4u;
            mesh_out.primitive_count = 2u;
        }
        return;
    }

    // 近档立方体 / 远档十字双 quad：与 CPU 近/远分档同一阈值（全 3D 距离²）
    if (dist2 < camera.cam_pos.w) {
        if (lid < 24u) {
            write_vertex(lid, CUBE_POS[lid], CUBE_UV[lid], inst, cam, fade, flat, is_gun);
        }
        if (lid < 12u) {
            mesh_out.primitives[lid].indices = CUBE_TRI[lid];
        }
        if (lid == 0u) {
            mesh_out.vertex_count = 24u;
            mesh_out.primitive_count = 12u;
        }
    } else {
        if (lid < 8u) {
            write_vertex(lid, CROSS_POS[lid], CROSS_UV[lid], inst, cam, fade, flat, is_gun);
        }
        if (lid < 4u) {
            mesh_out.primitives[lid].indices = CROSS_TRI[lid];
        }
        if (lid == 0u) {
            mesh_out.vertex_count = 8u;
            mesh_out.primitive_count = 4u;
        }
    }
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

/// 网格着色器编译：SPIR-V 必须 >= 1.4（MeshShadingEXT 能力），
/// 其余选项（含 ADJUST_COORDINATE_SPACE 的 Y 翻转）与顶点着色器完全一致。
fn compile_wgsl_mesh(source: &str) -> Vec<u32> {
    let module = naga::front::wgsl::parse_str(source).expect("WGSL 网格着色器解析失败");
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::default(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("WGSL 网格着色器验证失败");
    let options = naga::back::spv::Options {
        lang_version: (1, 4),
        ..naga::back::spv::Options::default()
    };
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

/// 阴影 depth-only 顶点着色器（2026-08-11）：光空间 view-proj + 实例变换，
/// 输出裁剪空间位置（shadow pass 渲地形/实例场/障碍/NPC 到 shadow map）。
const SHADOW_VERTEX_SHADER_WGSL: &str = r#"
struct ShadowVP {
    view_proj: mat4x4<f32>,
}
@group(0) @binding(0) var<uniform> shadow_vp: ShadowVP;

struct Instance {
    model: mat4x4<f32>,
    tint: vec4<f32>,
}
@group(0) @binding(2) var<storage, read> instances: array<Instance>;

@vertex
fn shadow_main(
    @location(0) position: vec3<f32>,
    @builtin(instance_index) instance_index: u32,
) -> @builtin(position) vec4<f32> {
    let inst = instances[instance_index];
    return shadow_vp.view_proj * inst.model * vec4<f32>(position, 1.0);
}
"#;

/// 阴影 depth-only 片元着色器：无输出（render pass 无颜色附件）
const SHADOW_FRAGMENT_SHADER_WGSL: &str = r#"
@fragment
fn fs_main() {}
"#;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let vs_spirv = compile_wgsl(VERTEX_SHADER_WGSL);
    let fs_spirv = compile_wgsl(FRAGMENT_SHADER_WGSL);
    let mesh_spirv = compile_wgsl_mesh(MESH_SHADER_WGSL);
    let hud_vs_spirv = compile_wgsl(HUD_VERTEX_SHADER_WGSL);
    let hud_fs_spirv = compile_wgsl(HUD_FRAGMENT_SHADER_WGSL);
    let shadow_vs_spirv = compile_wgsl(SHADOW_VERTEX_SHADER_WGSL);
    let shadow_fs_spirv = compile_wgsl(SHADOW_FRAGMENT_SHADER_WGSL);
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
    output.push_str("/// 网格着色器 SPIR-V 字节码\n");
    output.push_str("#[allow(dead_code)]\n");
    output.push_str(&format!("pub const MESH_SPIRV: &[u32] = &{:?};\n", mesh_spirv));
    output.push_str("/// HUD 顶点着色器 SPIR-V 字节码\n");
    output.push_str("#[allow(dead_code)]\n");
    output.push_str(&format!("pub const HUD_VS_SPIRV: &[u32] = &{:?};\n\n", hud_vs_spirv));
    output.push_str("/// HUD 片元着色器 SPIR-V 字节码\n");
    output.push_str("#[allow(dead_code)]\n");
    output.push_str(&format!("pub const HUD_FS_SPIRV: &[u32] = &{:?};\n", hud_fs_spirv));
    output.push_str("/// 阴影 depth-only 顶点着色器 SPIR-V 字节码\n");
    output.push_str("#[allow(dead_code)]\n");
    output.push_str(&format!("pub const SHADOW_VS_SPIRV: &[u32] = &{:?};\n", shadow_vs_spirv));
    output.push_str("/// 阴影 depth-only 片元着色器 SPIR-V 字节码\n");
    output.push_str("#[allow(dead_code)]\n");
    output.push_str(&format!("pub const SHADOW_FS_SPIRV: &[u32] = &{:?};\n", shadow_fs_spirv));
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
    write_spv(&mesh_spirv, "mesh.spv");
    write_spv(&hud_vs_spirv, "hud.vert.spv");
    write_spv(&hud_fs_spirv, "hud.frag.spv");
    write_spv(&shadow_vs_spirv, "shadow.vert.spv");
    write_spv(&shadow_fs_spirv, "shadow.frag.spv");
    println!("cargo:info=着色器编译完成");
}