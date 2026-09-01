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
const NPC_INSTANCE_BASE: u32 = 65536u + 1u + 1024u; // marker 区 = MAX_MARKER_INSTANCES(1024)，与 renderer.rs 对齐（2026-08-24 C2 修正：曾误写 3072 导致前 2048 个 NPC 盒体被当 marker）
// NPC 圆柱段（四肢）/ 球体段（头）起始槽：与 renderer.rs NPC_CYL_SLOT_BASE/NPC_SPH_SLOT_BASE 一致（各区 3072）
const NPC_CYL_BASE: u32 = NPC_INSTANCE_BASE + 3072u;
const NPC_SPH_BASE: u32 = NPC_INSTANCE_BASE + 6144u;
// 槽位 >= 该值的实例为「自发光」实体（爆炸闪光等）：片元跳过光照与贴图混合，直出纯色。
// 必须与 renderer.rs 的 EMISSIVE_SLOT_BASE 同步（NPC 区 3×1024：
// 盒体段 + 圆柱段（四肢）+ 球体段（头），见 NPC_SLOT_BASE/NPC_CYL_SLOT_BASE/NPC_SPH_SLOT_BASE）。
const EMISSIVE_INSTANCE_BASE: u32 = NPC_INSTANCE_BASE + 9216u;

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
    // 树冠（绿色 tint 的障碍 marker）顶点揉皱：伪随机扰动局部坐标 → 立方块变有机团块，
    // 配合片元杂色让树冠呈现体积感（2026-08-23 纸片树修复）
    var pos = position;
    if (instance_index >= MARKER_INSTANCE_BASE && instance_index < NPC_INSTANCE_BASE
        && inst.tint.g > inst.tint.r && inst.tint.g > inst.tint.b * 1.4) {
        let h = position.x * 12.9898 + position.y * 78.233 + position.z * 37.719
            + f32(instance_index % 97u) * 0.618;
        let n = fract(sin(h) * 43758.5453) - 0.5;
        pos = position + vec3<f32>(n, n * 0.7, n) * 0.38;
    }
    let world_pos = inst.model * vec4<f32>(pos, 1.0);
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
    // 枪模专用 identity 槽（renderer.rs GUN_INSTANCE_INDEX = 65536+1+64+3072+64）：
    // flat=3 = baked 顶点光照直出路径（2026-08-22：marker 改走实时光照后，枪模保持烘焙）
    if (instance_index == 75841u) {
        output.flat_flag = 3.0;
        output.fade = 1.0;
    }
    // 自发光区间（EMISSIVE_BASE .. +64）；枪槽（+64 后一槽）在此区间之外（2026-08-27 修复：
    // 原 >= 使枪槽被当作自发光 → flag=1+fade=2 → 走光照路径被太阳光×7.7 刷成纯白）
    if (instance_index >= EMISSIVE_INSTANCE_BASE && instance_index < EMISSIVE_INSTANCE_BASE + 64u) {
        // 自发光实体：fade > 1 作为 emissive 信号（片元跳过光照/贴图混合，走体积光晕分支）。
        // fade 的取值进一步编码「火 / 烟」两种响应，编码来源是实例 tint.w
        // （0 = 火，>= 0.5 = 烟）；粒子种类由 main.rs 决定，片元以 3.0 为分界。
        output.flat_flag = 1.0;
        if (inst.tint.w >= 0.5) {
            output.fade = 3.5; // 烟
        } else {
            output.fade = 2.0; // 火
        }
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
// 高光贡献系数（2026-08-24：避免 evaluate_directional/point 两处魔法数字重复）
const SPEC_CONTRIB: f32 = 0.4;
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

fn evaluate_directional(light: DirectionalLight, normal: vec3<f32>, view_dir: vec3<f32>, shininess: f32) -> vec3<f32> {
    if (light.direction.w < 0.5) {
        return vec3<f32>(0.0);
    }
    let light_dir = normalize(light.direction.xyz);
    let diffuse = bp_diffuse(normal, light_dir);
    let spec = bp_specular(normal, light_dir, view_dir, shininess);
    return light.color_intensity.xyz * light.color_intensity.w * (diffuse + SPEC_CONTRIB * spec);
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
    return light.color_intensity.xyz * light.color_intensity.w * atten * (diffuse + SPEC_CONTRIB * spec);
}

// 光照应用（地面与 marker/NPC 共用，2026-08-22）：屏幕导数法线 + 3x3 PCF 阴影 + 方向/点光。
// 障碍/建筑此前走“纯色直出”无面光照 → 一律同色剪影，纸片感；现在与地面同光源后，
// 顶面/迎光面/背光面自然分层，建筑/树/集装箱有立体明暗。
fn apply_lighting(input: VertexOutput, color: vec3<f32>) -> vec3<f32> {
    if (light_data.flags.x < 0.5) {
        return color;
    }
    let view_dir = normalize(input.view_dir);
    var normal = normalize(cross(dpdx(input.world_pos), dpdy(input.world_pos)));
    if (dot(normal, view_dir) < 0.0) {
        normal = -normal;
    }
    var shadow_factor = 0.0;
    var debug_outside = true;
    var d_avg = 0.0;
    var frag_depth = 0.0;
    if (light_data.flags.y >= 0.5 && light_data.shadow.bias.z >= 0.5) {
        let sp = light_data.shadow.light_view_proj * vec4<f32>(input.world_pos, 1.0);
        let shadow_uv = vec2<f32>(sp.x * 0.5 + 0.5, 1.0 - (sp.y * 0.5 + 0.5));
        // glam ortho_rh（本版本）产出 [0,1] 深度，与 GPU 写入的阴影图同基准，
        // 禁止再乘 0.5+0.5（OpenGL [-1,1] 旧映射，二重偏移 +0.25 曾致全场误判阴影）。
        frag_depth = sp.z;
        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0
            && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0
            && sp.z >= 0.0 && sp.z <= 1.0) {
            debug_outside = false;
            let texel = 1.0 / light_data.shadow.config.x;
            var occluded = 0.0;
            var dsum = 0.0;
            for (var dy = -1; dy <= 1; dy = dy + 1) {
                for (var dx = -1; dx <= 1; dx = dx + 1) {
                    let d = textureSample(shadow_map, shadow_sampler,
                        shadow_uv + vec2<f32>(f32(dx), f32(dy)) * texel);
                    dsum = dsum + d;
                    if (frag_depth - light_data.shadow.bias.x > d) {
                        occluded = occluded + 1.0;
                    }
                }
            }
            d_avg = dsum / 9.0;
            shadow_factor = occluded / 9.0;
        }
    }
    let shininess = 32.0;
    if light_data.shadow.config.y >= 0.5 {
        if (debug_outside) {
            return vec3<f32>(0.0, 0.0, 1.0);
        }
        return vec3<f32>(frag_depth, d_avg, 0.0);
    }
    var radiance = light_data.ambient.rgb * light_data.ambient.w;
    radiance = radiance + evaluate_directional(light_data.directional, normal, view_dir, shininess) * (1.0 - shadow_factor);
    for (var i = 0u; i < 4u; i = i + 1u) {
        radiance = radiance + evaluate_point(light_data.points[i], input.world_pos, normal, view_dir, shininess);
    }
    return color * min(radiance, vec3<f32>(1.0));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if (input.fade <= 0.02) {
        discard;
    }
    // 自发光（爆炸/枪口焰/烟雾粒子；顶点阶段用 fade 编码种类）。
    // 旧实现直出纯色 → 均匀缩放的球壳在屏幕上就是一张平面着色的多边形纸片（D8）。
    // 这里不引入 alpha 混合（主 pass 是全不透明管线），纯靠视相关径向衰减把亮度压向
    // 轮廓来伪造体积光晕：ndv = |N·V| 在球心≈1、轮廓≈0。
    if (input.fade > 1.0) {
        let edir = normalize(input.view_dir);
        let enorm = normalize(cross(dpdx(input.world_pos), dpdy(input.world_pos)));
        let ndv = abs(dot(enorm, edir));
        if (input.fade > 3.0) {
            // 烟：暗灰、中心略亮、边缘缓衰减，并吃一点环境光，避免变成纯黑洞
            let amb = light_data.ambient.rgb * light_data.ambient.w;
            return vec4<f32>(input.color * (0.40 + 0.60 * ndv) + amb * 0.35, 1.0);
        }
        // 火：中心过曝白热 + 向外快速衰减成 tint 本色边缘 → 有核有晕，不再是一张饼
        let core = pow(ndv, 2.2);
        let hot = pow(ndv, 7.0);
        return vec4<f32>(input.color * (0.22 + 1.55 * core) + vec3<f32>(1.0) * hot * 0.55, 1.0);
    }
    // 枪模（flat=3）：顶点色已含烘焙光照，直出（不再被实时光照二次处理）
    if (input.flat_flag > 2.5) {
        return vec4<f32>(input.color * input.fade, 1.0);
    }
    // marker/NPC 材质编码路径：flat_flag=1（marker）/ 2（NPC），按顶点 UV 采样
    // 程序化皮肤纹理（立方体每面 UV 铺满 0..1，与 procedural.rs 皮肤纹理逐面对齐）。
    // RV3D_SKIN_TEX=1（light_data.flags.z）启用；缺省 0 保持纯色路径（冒烟基线不变）。
    if (input.flat_flag > 0.5) {
        var base: vec3<f32> = input.color;
        // D12：皮肤/墙面/树冠杂色都是高频信号，远距时一个像素盖住多个细节单元，
        // MIP 平均后仍残留 salt-and-pepper 闪点（士兵"蓝底白斑平板"、建筑立面满屏麻点）。
        // 用 fwidth(uv) 直接量出单像素覆盖的纹理面积（marker/NPC 的 uv 每面铺满 0..1，
        // 故其导数即屏幕足迹的倒数），越迷你越把细节权重收回 tint 均值。
        // detail=1（近）→ 与旧式逐位一致；detail=0（远）→ 近似纯色，闪点消失。
        let foot = max(abs(fwidth(input.uv).x), abs(fwidth(input.uv).y));
        let detail = 1.0 - smoothstep(0.015, 0.22, foot);
        // 树冠杂色（2026-08-23）：绿色 tint 的障碍，按世界坐标噪声微调颜色 → 团簇感。
        // 该噪声是世界坐标哈希（无 MIP 可平均），远距必然逐像素乱闪 → 同样按 detail 收敛。
        if (input.flat_flag < 1.5 && base.g > base.r && base.g > base.b * 1.4) {
            let hh = fract(sin(dot(input.world_pos.xz, vec2<f32>(0.137, 0.211))) * 43758.5453);
            base = base * mix(1.0, 0.82 + hh * 0.36, detail);
        }
        if (light_data.flags.z >= 0.5) {
            if (input.flat_flag > 1.5) {
                // NPC 士兵：迷彩军服纹理 × 阵营 tint（去饱和纹素保留色相）
                let texel = textureSample(npc_skin_tex, texture_sampler, input.uv);
                let luma = dot(texel.rgb, vec3<f32>(0.299, 0.587, 0.114));
                // 近处幅度 0.55+0.90·luma 与 D1 验收式逐位一致（勿回退），远处收到 0.55+0.12·luma
                base = input.color * (0.55 + (0.12 + 0.78 * detail) * luma);
            } else {
                // marker 障碍：混凝土墙纹理 × 障碍 tint（近期权重 0.45：tint 保色相，纹理供细节）
                // 玻璃类 tint（蓝灰系：b 显著大于 r）跳过纹理，保持干净透亮
                if (input.color.b > input.color.r * 1.4) {
                    base = input.color;
                } else {
                    let texel = textureSample(marker_skin_tex, texture_sampler, input.uv);
                    base = mix(input.color, texel.rgb, 0.45 * (0.25 + 0.75 * detail));
                }
            }
        }
        // D11：窗带改由片元着色表达（原为每层叠一个独立薄盒）。薄盒在掠射角下自身前后面
        // 落到同一批像素 → 立面出现 V 形锯齿深度干涉，且外挑读作悬挑鳍片；把条带改薄是反方向。
        // 这里零新增几何、零深度风险：建筑都坐落 y≈0、层高约 3m，故世界 Y 天然对齐楼层。
        // 分类一律用 input.color（常量），不用已被皮肤纹理改写的 base。
        // 只给"中性混凝土立面"加窗：用通道极差判定，天然排除砖墙(红)、木掩体(棕)、
        // 树冠(绿)等强色相 tint —— 否则围墙上会长出窗户。玻璃判据(b>r*1.4)沿用 D2 不动。
        if (input.flat_flag < 1.5 && input.color.b <= input.color.r * 1.4) {
            let cmax = max(input.color.r, max(input.color.g, input.color.b));
            let cmin = min(input.color.r, min(input.color.g, input.color.b));
            let neutral = 1.0 - smoothstep(0.06, 0.14, cmax - cmin);
            let fnrm = normalize(cross(dpdx(input.world_pos), dpdy(input.world_pos)));
            let vert = 1.0 - abs(fnrm.y);
            // 只给立面加窗，屋面/地面（法线近竖直）不加；且窗带只从 3m（二层）以上开始——
            // 混凝土围墙/矮裙房同样是中性灰，没有这道下限会在墙上画出一条通长深色窗带。
            if (vert > 0.5 && neutral > 0.01 && input.world_pos.y > 3.0) {
                let fl = fract(input.world_pos.y * (1.0 / 3.0));
                let wy = smoothstep(0.18, 0.26, fl) * (1.0 - smoothstep(0.64, 0.72, fl));
                // 竖梃每 2m 一根，避免窗带读成连续黑条；取水平主轴作为横向坐标
                let hx = select(input.world_pos.z, input.world_pos.x, abs(fnrm.x) > abs(fnrm.z));
                let tp = fract(hx * 0.5);
                let mull = min(1.0, step(0.88, tp) + step(tp, 0.06));
                let win = wy * (1.0 - 0.7 * mull);
                // 远距随 D12 的 detail 一起收敛：世界坐标高频信号不收敛就会变成新的走样源
                base = base * mix(1.0, 1.0 - 0.5 * win, detail * vert * neutral);
            }
        }
        // 轮廓分离（D12）：掠射角提亮，让士兵从背景里"读得出来"。
        // 只乘一个亮度标量、不改 RGB 分量比例 → 阵营色相与饱和度保持 D1 验收结论有效。
        if (input.flat_flag > 1.5) {
            let ndv = abs(dot(normalize(cross(dpdx(input.world_pos), dpdy(input.world_pos))),
                              normalize(input.view_dir)));
            base = base * (1.0 + 0.45 * pow(1.0 - ndv, 3.0));
        }
        // 障碍/NPC：应用 Blinn-Phong（同地面路径）——消除纯色剪影的纸片感
        let lit = apply_lighting(input, base);
        return vec4<f32>(lit * input.fade, 1.0);
    }
    // 世界空间 UV：地面/地形用 world_pos.xz 映射到全图 [0,1]（覆盖 2*256 米），
    // 与 procedural.rs 烘焙纹理严格对齐（marker/NPC/自发光已走 flat_flag 纯色路径，不采样）。
    let world_uv = (input.world_pos.xz + vec2<f32>(256.0, 256.0)) / 512.0;
    let texel = textureSample(texture_sampled, texture_sampler, world_uv);
    // 地面/地形：纹理占主导（0.75） + Blinn-Phong（默认关闭时保持旧混合渲染）
    let mixed = mix(input.color, texel.rgb, 0.75);
    if (light_data.flags.x < 0.5) {
        return vec4<f32>(mixed * input.fade, 1.0);
    }
    let lit = apply_lighting(input, mixed);
    return vec4<f32>(lit * input.fade, 1.0);
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
// MARKER_INSTANCE_BASE=65537、NPC_INSTANCE_BASE=65537+1024=66561、
// EMISSIVE_INSTANCE_BASE=NPC_INSTANCE_BASE+9216=75777（NPC 三几何区：盒/圆柱/球，各 3072）
const TERRAIN_INSTANCE_INDEX: u32 = 65536u;
const MARKER_INSTANCE_BASE: u32 = 65536u + 1u;
const NPC_INSTANCE_BASE: u32 = 65536u + 1u + 1024u; // marker 区 = MAX_MARKER_INSTANCES(1024)，与 renderer.rs 对齐（2026-08-24 C2 修正：曾误写 3072 导致前 2048 个 NPC 盒体被当 marker）
// NPC 圆柱段（四肢）/ 球体段（头）起始槽：与 renderer.rs NPC_CYL_SLOT_BASE/NPC_SPH_SLOT_BASE 一致（各区 3072）
const NPC_CYL_BASE: u32 = NPC_INSTANCE_BASE + 3072u;
const NPC_SPH_BASE: u32 = NPC_INSTANCE_BASE + 6144u;
const EMISSIVE_INSTANCE_BASE: u32 = NPC_INSTANCE_BASE + 9216u;

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

// 最大几何 = NPC 四肢圆柱（50 顶点 / 96 三角形；含立方体 24/12、二十面体 12/20——
// 旧 12 图元上限曾使二十面体 20 三角的 12..19 写入越界被钳位，随本扩容一并根治）
struct MeshOutput {
    @builtin(vertex_count) vertex_count: u32,
    @builtin(primitive_count) primitive_count: u32,
    @builtin(vertices) vertices: array<VertexOutput, 50>,
    @builtin(primitives) primitives: array<MeshPrimitive, 96>,
}
var<workgroup> mesh_out: MeshOutput;

// 本次 mesh draw 的起始实例槽：地面=0 / marker=65537 / NPC=66561 / 自发光=75777 / 枪=75841
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
// 二十面体（12 顶点/20 三角，单位球近似）：树冠（绿色 tint marker）与
// 自发光（爆炸等）用——摆脱「立方体树冠/方块冲击波」的纸片观感（2026-08-25）
const ICO_POS: array<vec3<f32>, 12> = array<vec3<f32>, 12>(
    vec3<f32>(0.0, 1.0, 0.618), vec3<f32>(0.0, 1.0, -0.618),
    vec3<f32>(0.0, -1.0, 0.618), vec3<f32>(0.0, -1.0, -0.618),
    vec3<f32>(1.0, 0.618, 0.0), vec3<f32>(-1.0, 0.618, 0.0),
    vec3<f32>(1.0, -0.618, 0.0), vec3<f32>(-1.0, -0.618, 0.0),
    vec3<f32>(0.618, 0.0, 1.0), vec3<f32>(-0.618, 0.0, 1.0),
    vec3<f32>(0.618, 0.0, -1.0), vec3<f32>(-0.618, 0.0, -1.0),
);
const ICO_TRI: array<vec3<u32>, 20> = array<vec3<u32>, 20>(
    vec3<u32>(0, 4, 1), vec3<u32>(0, 9, 4), vec3<u32>(9, 5, 4), vec3<u32>(4, 5, 8),
    vec3<u32>(4, 8, 1), vec3<u32>(8, 10, 1), vec3<u32>(8, 3, 10), vec3<u32>(5, 3, 8),
    vec3<u32>(5, 2, 3), vec3<u32>(2, 7, 3), vec3<u32>(7, 10, 3), vec3<u32>(7, 6, 10),
    vec3<u32>(7, 11, 6), vec3<u32>(11, 0, 6), vec3<u32>(0, 1, 6), vec3<u32>(6, 1, 10),
    vec3<u32>(9, 0, 11), vec3<u32>(9, 11, 2), vec3<u32>(9, 2, 5), vec3<u32>(7, 2, 11),
);
// SPH：一级细分二十面体（42 顶点/80 三角，单位半径）——自发光爆炸火球圆润化（D5，
// 确定性脚本预生成，勿手改数值）；ICO 保留给树冠（12v/20t 足够小尺度）
const SPH_POS: array<vec3<f32>, 42> = array<vec3<f32>, 42>(
    vec3<f32>(0.0000, 0.8507, 0.5257), vec3<f32>(0.0000, 0.8507, -0.5257), vec3<f32>(0.0000, -0.8507, 0.5257),
    vec3<f32>(0.0000, -0.8507, -0.5257), vec3<f32>(0.8507, 0.5257, 0.0000), vec3<f32>(-0.8507, 0.5257, 0.0000),
    vec3<f32>(0.8507, -0.5257, 0.0000), vec3<f32>(-0.8507, -0.5257, 0.0000), vec3<f32>(0.5257, 0.0000, 0.8507),
    vec3<f32>(-0.5257, 0.0000, 0.8507), vec3<f32>(0.5257, 0.0000, -0.8507), vec3<f32>(-0.5257, 0.0000, -0.8507),
    vec3<f32>(0.5000, 0.8090, 0.3090), vec3<f32>(0.5000, 0.8090, -0.3090), vec3<f32>(0.0000, 1.0000, 0.0000),
    vec3<f32>(-0.3090, 0.5000, 0.8090), vec3<f32>(0.3090, 0.5000, 0.8090), vec3<f32>(-0.8090, 0.3090, 0.5000),
    vec3<f32>(0.0000, 1.0000, 0.0000), vec3<f32>(-0.3090, 0.5000, 0.8090), vec3<f32>(0.8090, 0.3090, 0.5000),
    vec3<f32>(0.5000, 0.8090, 0.3090), vec3<f32>(1.0000, 0.0000, 0.0000), vec3<f32>(0.3090, 0.5000, -0.8090),
    vec3<f32>(0.5000, -0.8090, 0.3090), vec3<f32>(0.3090, -0.5000, -0.8090), vec3<f32>(-0.8090, -0.3090, -0.5000),
    vec3<f32>(-0.8090, -0.3090, 0.5000), vec3<f32>(0.0000, -1.0000, 0.0000), vec3<f32>(-0.5000, -0.8090, 0.3090),
    vec3<f32>(-0.5000, -0.8090, -0.3090), vec3<f32>(-0.3090, -0.5000, -0.8090), vec3<f32>(0.0000, -1.0000, 0.0000),
    vec3<f32>(0.8090, -0.3090, -0.5000), vec3<f32>(-0.8090, -0.3090, -0.5000), vec3<f32>(0.3090, -0.5000, -0.8090),
    vec3<f32>(-0.5000, 0.8090, -0.3090), vec3<f32>(0.8090, 0.3090, 0.5000), vec3<f32>(0.8090, 0.3090, -0.5000),
    vec3<f32>(-1.0000, 0.0000, 0.0000), vec3<f32>(-0.5000, -0.8090, -0.3090), vec3<f32>(-0.3090, -0.5000, 0.8090),
);
const SPH_TRI: array<vec3<u32>, 80> = array<vec3<u32>, 80>(
    vec3<u32>(0u, 12u, 14u), vec3<u32>(4u, 13u, 12u), vec3<u32>(1u, 14u, 13u), vec3<u32>(12u, 13u, 14u),
    vec3<u32>(0u, 15u, 12u), vec3<u32>(9u, 16u, 15u), vec3<u32>(4u, 12u, 16u), vec3<u32>(15u, 16u, 12u),
    vec3<u32>(9u, 17u, 16u), vec3<u32>(5u, 18u, 17u), vec3<u32>(4u, 16u, 18u), vec3<u32>(17u, 18u, 16u),
    vec3<u32>(4u, 18u, 20u), vec3<u32>(5u, 19u, 18u), vec3<u32>(8u, 20u, 19u), vec3<u32>(18u, 19u, 20u),
    vec3<u32>(4u, 20u, 13u), vec3<u32>(8u, 21u, 20u), vec3<u32>(1u, 13u, 21u), vec3<u32>(20u, 21u, 13u),
    vec3<u32>(8u, 22u, 21u), vec3<u32>(10u, 23u, 22u), vec3<u32>(1u, 21u, 23u), vec3<u32>(22u, 23u, 21u),
    vec3<u32>(8u, 24u, 22u), vec3<u32>(3u, 25u, 24u), vec3<u32>(10u, 22u, 25u), vec3<u32>(24u, 25u, 22u),
    vec3<u32>(5u, 26u, 19u), vec3<u32>(3u, 24u, 26u), vec3<u32>(8u, 19u, 24u), vec3<u32>(26u, 24u, 19u),
    vec3<u32>(5u, 27u, 26u), vec3<u32>(2u, 28u, 27u), vec3<u32>(3u, 26u, 28u), vec3<u32>(27u, 28u, 26u),
    vec3<u32>(2u, 29u, 28u), vec3<u32>(7u, 30u, 29u), vec3<u32>(3u, 28u, 30u), vec3<u32>(29u, 30u, 28u),
    vec3<u32>(7u, 31u, 30u), vec3<u32>(10u, 25u, 31u), vec3<u32>(3u, 30u, 25u), vec3<u32>(31u, 25u, 30u),
    vec3<u32>(7u, 32u, 31u), vec3<u32>(6u, 33u, 32u), vec3<u32>(10u, 31u, 33u), vec3<u32>(32u, 33u, 31u),
    vec3<u32>(7u, 34u, 32u), vec3<u32>(11u, 35u, 34u), vec3<u32>(6u, 32u, 35u), vec3<u32>(34u, 35u, 32u),
    vec3<u32>(11u, 36u, 35u), vec3<u32>(0u, 37u, 36u), vec3<u32>(6u, 35u, 37u), vec3<u32>(36u, 37u, 35u),
    vec3<u32>(0u, 14u, 37u), vec3<u32>(1u, 38u, 14u), vec3<u32>(6u, 37u, 38u), vec3<u32>(14u, 38u, 37u),
    vec3<u32>(6u, 38u, 33u), vec3<u32>(1u, 23u, 38u), vec3<u32>(10u, 33u, 23u), vec3<u32>(38u, 23u, 33u),
    vec3<u32>(9u, 15u, 39u), vec3<u32>(0u, 36u, 15u), vec3<u32>(11u, 39u, 36u), vec3<u32>(15u, 36u, 39u),
    vec3<u32>(9u, 39u, 41u), vec3<u32>(11u, 40u, 39u), vec3<u32>(2u, 41u, 40u), vec3<u32>(39u, 40u, 41u),
    vec3<u32>(9u, 41u, 17u), vec3<u32>(2u, 27u, 41u), vec3<u32>(5u, 17u, 27u), vec3<u32>(41u, 27u, 17u),
    vec3<u32>(7u, 29u, 34u), vec3<u32>(2u, 40u, 29u), vec3<u32>(11u, 34u, 40u), vec3<u32>(29u, 40u, 34u),
);
// 树冠识别：绿色 tint（g 显著大于 r/b）
fn is_foliage(tint: vec4<f32>) -> bool {
    return tint.g > tint.r && tint.g > tint.b * 1.4;
}

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

// ---- 地形 PBR 细节噪声（确定性、无随机种子：世界坐标纯函数，跨帧/跨象限稳定）----
// 与 CPU 侧 terrain_hash 同族整数哈希（u32 模算术逐位一致），格点值噪声 C1 连续；
// 相邻实例共享顶点世界坐标 → 同一 (x,z) 恒得同一值，quad 间无缝无裂缝。
fn terrain_lattice_hash(ix: i32, iz: i32) -> f32 {
    var h: u32 = u32(ix) * 0x1B873593u ^ u32(iz) * 0xCC9E2D51u;
    h = h ^ (h >> 16u);
    h = h * 0x7FEB352Du;
    h = h ^ (h >> 15u);
    h = h * 0x846CA68Bu;
    h = h ^ (h >> 16u);
    return f32(h & 0xFFFFu) / 65535.0;
}

fn terrain_smooth(t: f32) -> f32 {
    return t * t * (3.0 - 2.0 * t);
}

/// 双线性 smoothstep 值噪声（确定性、C1 连续，与 CPU terrain_value_noise 同族）
fn terrain_value_noise(x: f32, z: f32, cell: f32) -> f32 {
    let fx = x / cell;
    let fz = z / cell;
    let ix = i32(floor(fx));
    let iz = i32(floor(fz));
    let tx = terrain_smooth(fx - f32(ix));
    let tz = terrain_smooth(fz - f32(iz));
    let h00 = terrain_lattice_hash(ix, iz);
    let h10 = terrain_lattice_hash(ix + 1, iz);
    let h01 = terrain_lattice_hash(ix, iz + 1);
    let h11 = terrain_lattice_hash(ix + 1, iz + 1);
    let a = h00 + (h10 - h00) * tx;
    let b = h01 + (h11 - h01) * tx;
    return a + (b - a) * tz;
}

/// 地形顶点色（草地/沙粒/泥土分域）：大尺度干湿分域（干燥处偏沙黄）叠加
/// 高频草色斑驳（黄绿 ↔ 深绿交替）与细颗粒明暗抖动（泥土感）。
/// 片元着色器按 0.75 纹理 + 0.25 顶点色混合，此 tint 在烘焙地面纹理上
/// 叠加材质变化，光照开/关两条路径均生效。
fn terrain_tint(x: f32, z: f32) -> vec3<f32> {
    let biome = terrain_value_noise(x, z, 48.0) * 0.65 + terrain_value_noise(x, z, 24.0) * 0.35;
    let grass = mix(
        vec3<f32>(0.13, 0.30, 0.08), // 深绿（阴湿草丛）
        vec3<f32>(0.42, 0.44, 0.15), // 黄绿（枯草/阳面）
        terrain_value_noise(x, z, 3.0),
    );
    let sand_col = vec3<f32>(0.60, 0.50, 0.28); // 沙黄（干燥处）
    let col = mix(grass, sand_col, smoothstep(0.45, 0.62, biome));
    // 细颗粒明暗抖动（泥土斑驳，±14%）
    return col * (0.86 + 0.28 * terrain_value_noise(x, z, 1.5));
}

/// 高度扰动（轻微法线扰动）：低频起伏 + 高频颗粒，幅度 ±3cm。
/// 片元着色器用世界坐标屏幕导数求面法线 → 扰动自动转为 quad 法线倾斜，
/// 光照下表面呈颗粒/起伏质感而非单一平面。
fn terrain_bump(x: f32, z: f32) -> f32 {
    let low = terrain_value_noise(x, z, 7.0);
    let high = terrain_value_noise(x, z, 1.6);
    return (low * 0.55 + high * 0.45 - 0.5) * 0.06;
}

fn write_vertex(
    lid: u32,
    pos: vec3<f32>,
    uv: vec2<f32>,
    inst: Instance,
    cam: vec3<f32>,
    fade: f32,
    flat: f32,
    is_gun: bool,
    terrain: bool,
) {
    var v: VertexOutput;
    var p = pos;
    var col = inst.tint.rgb;
    if (terrain) {
        // 地形 PBR 细节：仅地面/地形槽应用（marker/NPC/自发光不受影响）。
        // 高度扰动经片元世界坐标屏幕导数转法线倾斜（光照下颗粒质感）；
        // 颜色变化表达草地/沙粒/泥土分域（片元 0.75 纹理混合下仍可见）。
        let wpos = (inst.model * vec4<f32>(pos, 1.0)).xyz;
        p = pos + vec3<f32>(0.0, terrain_bump(wpos.x, wpos.z), 0.0);
        col = terrain_tint(wpos.x, wpos.z);
    }
    let wp = inst.model * vec4<f32>(p, 1.0);
    v.position = camera.proj * camera.view * wp;
    // 枪模深度覆盖：第一人称枪槽（NPC 区末 16 槽）强制 z_clip=0（NDC z=0 →
    // 深度 0.5，恒小于世界几何），杜绝枪管/枪托被近墙/地形"穿模遮住"
    if (is_gun) {
        v.position.z = 0.0;
    }
    // 顶点色：地形走程序化 terrain_tint（草地/沙粒/泥土分域）；其余白 × tint
    // （与顶点着色器 color * inst.tint.rgb 一致）
    v.color = col;
    v.uv = uv;
    v.fade = fade;
    v.world_pos = wp.xyz;
    // 相机世界位置：view = [R|t]，相机位置 = -R^T * t（与顶点着色器同源）
    v.view_dir = normalize(cam - wp.xyz);
    v.flat_flag = flat;
    mesh_out.vertices[lid] = v;
}

@mesh(mesh_out)
@workgroup_size(96)
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
    if (slot >= EMISSIVE_INSTANCE_BASE && slot < EMISSIVE_INSTANCE_BASE + 64u) {
        // 自发光实体：fade > 1 作为 emissive 信号（片元跳过光照/贴图混合，走体积光晕分支）。
        // tint.w 编码种类（0 = 火，>= 0.5 = 烟），与顶点着色器路径完全一致。
        flat = 1.0;
        if (inst.tint.w >= 0.5) {
            fade = 3.5; // 烟
        } else {
            fade = 2.0; // 火
        }
    } else if (slot >= NPC_INSTANCE_BASE) {
        flat = 2.0;
    } else if (slot >= MARKER_INSTANCE_BASE) {
        flat = 1.0;
    }
    // 枪模槽位 = GUN_INSTANCE_INDEX（75841；旧式 NPC_INSTANCE_BASE+1024-16 是 1024 时代
    // 残留，范围 67569..75777 把 NPC 圆柱/球体段全部误判为枪 → 四肢/头被 z=0 深度覆盖，
    // 「鬼魂/穿模」观感的另一来源；mesh 路径不画枪模，此判定仅保护传统 draw 的 GUN 槽语义）。
    let is_gun = slot == 75841u;

    if (is_ground) {
        if (lid < 4u) {
            write_vertex(lid, GROUND_POS[lid], GROUND_UV[lid], inst, cam, fade, flat, false, true);
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
    // 障碍 marker 槽恒用立方体（远距十字 quad 俯视是"方块贴图+缝隙"，用户反馈的边界方块感）
    let is_marker = slot >= MARKER_INSTANCE_BASE && slot < NPC_INSTANCE_BASE;
    // NPC 四肢（圆柱）/ 头部（二十面体球）优先判定（2026-08-31 D6：方块人恢复圆柱四肢/球形头）
    let is_npc_cyl = slot >= NPC_CYL_BASE && slot < NPC_SPH_BASE;
    let is_npc_sph = slot >= NPC_SPH_BASE && slot < EMISSIVE_INSTANCE_BASE;
    // 树冠（绿色 marker）→ 二十面体；自发光（爆炸光球）→ SPH 细分球（D5）
    let is_glow = slot >= EMISSIVE_INSTANCE_BASE && slot < EMISSIVE_INSTANCE_BASE + 64u;
    let is_tree = is_foliage(inst.tint);
    if (is_npc_cyl) {
        // 四肢：程序化单位圆柱（r=1、y∈[-0.5,0.5]、Y 轴、24 段含盖；
        // 与 CPU create_cylinder_geometry 同单位空间，实例矩阵按此构建）。
        // 顶点布局：底圈 lid 0..23（y=-0.5）、顶圈 lid 24..47（y=+0.5）、顶盖中心 48、底盖中心 49。
        if (lid < 48u) {
            let i = lid % 24u;
            let theta = f32(i) * 0.2617993877991494; // TAU/24
            let y = select(-0.5, 0.5, lid >= 24u);
            write_vertex(lid, vec3<f32>(cos(theta), y, sin(theta)), vec2<f32>(f32(i) * 0.041666666667, y + 0.5), inst, cam, fade, flat, is_gun, false);
        } else if (lid < 50u) {
            let y = select(-0.5, 0.5, lid == 48u);
            write_vertex(lid, vec3<f32>(0.0, y, 0.0), vec2<f32>(0.5, y + 0.5), inst, cam, fade, flat, is_gun, false);
        }
        // 图元 96：侧面 48（每段 2 三角）+ 顶盖 24 + 底盖 24；绕序模型空间外侧 CCW（同 CUBE 约定）
        if (lid < 48u) {
            let i = lid / 2u;
            if ((lid & 1u) == 0u) {
                mesh_out.primitives[lid].indices = vec3<u32>(i, i + 24u, (i + 1u) % 24u);
            } else {
                mesh_out.primitives[lid].indices = vec3<u32>(i + 24u, ((i + 1u) % 24u) + 24u, (i + 1u) % 24u);
            }
        } else if (lid < 72u) {
            let i = lid - 48u;
            mesh_out.primitives[lid].indices = vec3<u32>(48u, ((i + 1u) % 24u) + 24u, i + 24u);
        } else if (lid < 96u) {
            let i = lid - 72u;
            mesh_out.primitives[lid].indices = vec3<u32>(49u, i, (i + 1u) % 24u);
        }
        if (lid == 0u) {
            mesh_out.vertex_count = 50u;
            mesh_out.primitive_count = 96u;
        }
    } else if (is_npc_sph) {
        // 头部：二十面体归一化到半径 1（与 CPU 球体同单位空间）
        if (lid < 12u) {
            write_vertex(lid, normalize(ICO_POS[lid]), vec2<f32>(0.0, 0.0), inst, cam, fade, flat, is_gun, false);
        }
        if (lid < 20u) {
            mesh_out.primitives[lid].indices = ICO_TRI[lid];
        }
        if (lid == 0u) {
            mesh_out.vertex_count = 12u;
            mesh_out.primitive_count = 20u;
        }
    } else if (is_glow) {
        // D5：自发光爆炸用 SPH 一级细分二十面体（42v/80t）保圆润，消除 ICO 大三角面尖刺观感
        if (lid < 42u) {
            write_vertex(lid, SPH_POS[lid], vec2<f32>(0.0, 0.0), inst, cam, fade, flat, is_gun, false);
        }
        if (lid < 80u) {
            mesh_out.primitives[lid].indices = SPH_TRI[lid];
        }
        if (lid == 0u) {
            mesh_out.vertex_count = 42u;
            mesh_out.primitive_count = 80u;
        }
    } else if (is_tree) {
        if (lid < 12u) {
            write_vertex(lid, ICO_POS[lid] * 0.9, vec2<f32>(0.0, 0.0), inst, cam, fade, flat, is_gun, false);
        }
        if (lid < 20u) {
            mesh_out.primitives[lid].indices = ICO_TRI[lid];
        }
        if (lid == 0u) {
            mesh_out.vertex_count = 12u;
            mesh_out.primitive_count = 20u;
        }
    } else if (is_marker || dist2 < camera.cam_pos.w) {
        if (lid < 24u) {
            write_vertex(lid, CUBE_POS[lid], CUBE_UV[lid], inst, cam, fade, flat, is_gun, false);
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
            write_vertex(lid, CROSS_POS[lid], CROSS_UV[lid], inst, cam, fade, flat, is_gun, false);
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

// 【临时探针】naga 对 WGSL ray-query 支持
#[path = "build_spv_rt.rs"]
mod spv_rt_bench;

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
    output.push_str("/// RT 求交基准 SPIR-V（手工汇编；naga 不支持 WGSL ray-query）\n");
    output.push_str("#[allow(dead_code)]\n");
    output.push_str(&format!("pub const RT_BENCH_SPV: &[u32] = &{:?};\n\n", spv_rt_bench::rt_bench_spv()));
    // PT 帧着色器：assets/rt/pt_panorama.glsl --glslang--> .spv（2026-08-31 弃手工拼装 SPIR-V）
    {
        let dir = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets").join("rt");
        let spv_path = dir.join("pt_panorama.spv");
        let glsl_path = dir.join("pt_panorama.glsl");
        println!("cargo:rerun-if-changed=assets/rt/pt_panorama.spv");
        println!("cargo:rerun-if-changed=assets/rt/pt_panorama.glsl");
        let stale = match (std::fs::metadata(&glsl_path), std::fs::metadata(&spv_path)) {
            (Ok(g), Ok(s)) => {
                let gm = g.modified().ok();
                let sm = s.modified().ok();
                matches!((gm, sm), (Some(g), Some(s)) if g > s)
            }
            _ => false,
        };
        if stale {
            println!("cargo:warning=PT GLSL 比 SPV 新，请跑 scripts/compile_pt.ps1 重新编译 pt_panorama.spv");
        }
        let bytes = std::fs::read(&spv_path).ok();
        if let Some(bb) = bytes {
            let mut wv = Vec::with_capacity(bb.len() / 4);
            for i in 0..bb.len() / 4 {
                wv.push(u32::from_le_bytes([bb[i*4], bb[i*4+1], bb[i*4+2], bb[i*4+3]]));
            }
            output.push_str("/// PT 帧 SPIR-V（glslang 编译 assets/rt/pt_panorama.glsl，严格 spirv-val 通过）\n");
            output.push_str("#[allow(dead_code)]\n");
            output.push_str(&format!("pub const PT_FRAME_SPV: &[u32] = &{:?};\n\n", wv));
        }
    }
    { let mut bb = spv_rt_bench::rt_bench_spv(); let mut by = Vec::with_capacity(bb.len()*4); for w in &bb { by.extend_from_slice(&w.to_le_bytes()); } let _ = std::fs::write(Path::new(&out_dir).join("rt_bench.spv"), &by); }
    { let bb = spv_rt_bench::rt_bench_spv();
      let mut loader = rspirv::dr::Loader::new();
      match rspirv::binary::parse_words(&bb, &mut loader) {
        Ok(_) => println!("cargo:warning=RT_SPV_STRUCT_OK"),
        Err(e) => println!("cargo:warning=RT_SPV_STRUCT_ERR: {}", e),
      }
    }
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