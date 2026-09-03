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
const NPC_INSTANCE_BASE: u32 = 65536u + 1u + 8192u; // marker 区 = MAX_MARKER_INSTANCES(8192)，与 renderer.rs 对齐（2026-09-01 建模重构：1024 装不下真城市；实测 CPU 剔除 4034 个 marker 只花 20µs，所以容量不是瓶颈，再翻一档到 8192。改容量必须同步改本行两处副本 + renderer.rs + 枪槽字面量，见 gun_slot_layout_is_pinned）
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
    // 片元光照使用：表面 → 相机方向。**故意不归一化**：cam 为常量、world_pos 为世界线性量，
    // 透视校正插值对线性量精确，片元 length() 即真实视距，省掉一个插值属性（雾需要它）。
    // 所有消费端本来就各自 normalize()，语义不变（mesh 路径 write_vertex 同步改）。
    output.view_dir = cam_pos - world_pos.xyz;
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
    // 外部建模网格（geom.rs Shape::Authored，tint.w = 6.0）→ flat_flag = 1.25。
    // 片元用这个**专属区间**（1.1, 1.4）识别"这是 Blender 建模的资产，别再给它做任何
    // 程序化立面加工"：窗带、玻璃分格+菲涅耳、树冠噪声、按每面 0..1 UV 采样的混凝土皮肤。
    // 取 1.25 而不是 1.0 是因为片元里 1.0 与 marker 完全同形、无法再区分；而 1.25 既落在
    // (0.5, 1.5) 的 marker 侧（不会误入 NPC 轮廓光或枪模直出），又与 1.0 有足够间隙。
    // 必须放在枪槽判断**之前**，让 83009 的 flat=3.0 始终能覆盖它。
    var authored_mesh = false;
    if (inst.tint.w > 5.5 && inst.tint.w < 6.5) {
        output.flat_flag = 1.25;
        authored_mesh = true;
    }
    // 枪模专用 identity 槽（renderer.rs GUN_INSTANCE_INDEX = 65536+1+8192+3072*3+64 = 83009）：
    // flat=3 = baked 顶点光照直出路径（2026-08-22：marker 改走实时光照后，枪模保持烘焙）。
    // ⚠ 这个字面量随 MAX_MARKER_INSTANCES 变化，改容量必须同步改这里两处 + renderer.rs
    //   的 gun_slot_layout_is_pinned 测试会兜住漏改。
    if (instance_index == 83009u) {
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
    // 道具合并网格走 identity 实例矩阵，`inst.model[3].xyz` 恒为原点 → 上面那条会按
    // "离世界原点多远"来淡出整片道具，相机走到地图边缘时全城一起变透明。和地形/枪模
    // 一样，必须强制不淡出；距离相关的衰减本来就由雾负责。
    if (authored_mesh) {
        output.fade = 1.0;
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
// 地面微细节层（procedural.rs::generate_default_ground_detail_texture，2m 一 tile）。
// 绑定号 9 是本 set layout 里的第一个空位（0..8 已被 camera/主纹理/instances/sampler/
// light UBO/shadow map/shadow sampler/两张皮肤占用）；采样器复用 binding 3 的
// texture_sampler（REPEAT + LINEAR/LINEAR mip + max_lod=mip_levels-1，正好够用）。
@group(0) @binding(9) var ground_detail_tex: texture_2d<f32>;
// 与 procedural.rs 的 GROUND_DETAIL_METRES / GROUND_DETAIL_SIZE 严格同步（改了必须两边改）
const GROUND_DETAIL_METRES: f32 = 2.0;
// 一个纹素的米数 = 2.0 / 256
const GROUND_DETAIL_TEXEL_M: f32 = 0.0078125;
// 纹素 → 亮度调制的增益。约定 **纹素 r = 调制值 / 2**（调制 1.0 → 128），这样
// 0..2 的调制能完整落进 8bit 而不被 clamp 掉上限（直接存 mod 的话凡是 >1 的
// 提亮部分全被削成 255，均值会掉到 1 以下，地面整体只会变暗不会变亮）。
// ⚠ 该纹理必须以 **UNORM（线性）view** 创建，不能沿用现有 SRGB 的那个 helper：
//   128 经 sRGB 解码是 0.214，乘 2 得 0.43 → 全场地面暗一半。
const GROUND_DETAIL_GAIN: f32 = 2.0;
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

// ---- 大气 / 色调 / 表面风化（2026-09-01 建模重构第 1 批）----
// 视距直接由未归一化的 view_dir 还原（见 vs_main / write_vertex 注释）。
fn view_distance(input: VertexOutput) -> f32 {
    return length(input.view_dir);
}

// 地平线雾色必须与 swapchain clear color 逐分量相同（renderer.rs 的
// record_command_buffer clear_values[0]，当前 [0.24, 0.36, 0.60]），否则远处几何
// 与天空之间出现一条硬边。改其中一处必须同步改另一处。
const FOG_TINT: vec3<f32> = vec3<f32>(0.24, 0.36, 0.60);

// 雾：70m 起、630m 满，上限 0.92 而非 1.0 —— 完全抹平会让最远处的楼群整体消失，
// 保留 8% 反差不只是好看，也是让玩家仍能读出天际线轮廓。
fn fog_amount(d: f32) -> f32 {
    let t = clamp((d - 70.0) / 560.0, 0.0, 1.0);
    return t * t * 0.92;
}

// 世界坐标格点哈希 → 值噪声。频率锚定在"米"上，与物体尺寸、与逐面 0..1 UV 都无关，
// 所以 24m 大立面和 0.3m 木箱得到相同的纹素密度。旧的逐面 0..1 UV 是"纸片感"的
// 主要来源之一：同一张皮肤图被拉伸到 80 倍尺度差的不同面上。
//
// ⚠ 必须是 u32 **位混淆**（与 mesh 路径 terrain_lattice_hash 同族）。旧实现是
// "乘常数取小数"：fract(127.1*i) = fract(0.1*i) 只跟 i 的个位数走 → 整数格点上以
// 10 为周期重复。fp32 复现（tools 外的一次性脚本）实测 i=0..29 只有 21 个不同值，
// 且 i 为 10 的倍数时两个分量同时归零 → 哈希恒等于 0.0。
// 后果不是"噪点不够好看"而是**画出具体的线**：细频格距 1/1.7=0.59m，周期 10 格
// = 5.9m，于是地面与立面上每隔 5.9m 就有一条被压暗 ~3.5% 的通长直格线，外加一批
// 恒零格点连成的暗斑。玩家报"整个画面大量线条"，其中一条就是这个（世界锚定、
// 与地面烘焙纹理无关，只在着色器里）。
fn lattice_hash(ix: i32, iz: i32) -> f32 {
    var h: u32 = u32(ix) * 0x1B873593u ^ u32(iz) * 0xCC9E2D51u;
    h = h ^ (h >> 16u);
    h = h * 0x7FEB352Du;
    h = h ^ (h >> 15u);
    h = h * 0x846CA68Bu;
    h = h ^ (h >> 16u);
    return f32(h & 0xFFFFu) / 65535.0;
}

fn vnoise2(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = lattice_hash(i.x, i.y);
    let b = lattice_hash(i.x + 1, i.y);
    let c = lattice_hash(i.x, i.y + 1);
    let d2 = lattice_hash(i.x + 1, i.y + 1);
    return mix(mix(a, b, u.x), mix(c, d2, u.x), u.y);
}

// 世界坐标各轴的屏幕足迹（米/像素，绝对值）。凡"按米定义频率"的着色项（风化斑驳、
// 窗带、玻璃分格、阴影核宽度）都必须用它来收敛或做盒式平均。
//
// ⚠ 不要用 fwidth(uv) 代替：marker/NPC 的 uv 是**逐面铺满 0..1** 的，同一个 foot
// 数值对 1m 木箱和对 30m 大楼意味着相差 30 倍的世界尺度。D11 窗带正是拿 uv 域的
// detail 去收敛一个世界域信号，结果大立面上要到一个像素盖 6.6m 才开始收，中间整段
// 都在亚像素采样 → 相机一移动就满楼爬莫尔线（缺陷单"移动时大量线条"的第二条）。
// fwidth 必须在一致控制流里取，所以这里无条件调用，调用方丢掉不用的分量。
fn world_derivatives(world_pos: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(abs(fwidth(world_pos.x)),
                     abs(fwidth(world_pos.y)),
                     abs(fwidth(world_pos.z)));
}

// 把"世界坐标里的 0/1 条带"在一个像素上**严格盒式平均**：返回条带 [lo,hi]（相位域内）
// 覆盖该像素的比例 ∈ [0,1]。phase = 像素中心相位，half = 一个像素在该相位域里的半宽。
// 为什么不是"把 smoothstep 边缘加宽"：那只是让边缘变软，像素盖满一整层时条带内部仍然
// 顶到 1.0，会把"26% 玻璃 + 74% 混凝土"整面画成纯玻璃 —— 越远越黑，等于自己再造一次黑洞。
fn band_coverage(phase: f32, half: f32, lo: f32, hi: f32) -> f32 {
    let a0 = phase - half;
    let a1 = phase + half;
    return clamp((min(hi, a1) - max(lo, a0)) / max(a1 - a0, 1e-5), 0.0, 1.0);
}

// 沿主平面的双频风化斑驳，幅度 ±7%。
//
// **收敛量必须用屏幕足迹 fwidth(world_pos)（米/像素），不能用视距。**
// 第一版我按视距 40..150m 衰减，实机截图里地面和墙面上全是白点麻点——掠射角下
// 同一个 40m 距离对应的像素可以覆盖 0.05m 也可以覆盖 3m，用距离当闸门必然放行
// 亚像素细节，混叠成 salt-and-pepper。这正是本项目 D12 已经记过的坑，我自己又踩了一遍。
// 细频在一个像素盖过 0.22m 时退场，粗频到 1.0m 才退场，两者都走光时返回 1.0（纯平涂，
// 但至少不闪）。除以权重和是为了只剩一个倍频时幅度不缩水。
//
// 足迹取**该主平面内两轴**的最大值（旧实现取三轴全局最大）：30m 高的墙只要竖直方向
// 跨 0.3m/像素，就会把与横向分辨率无关的细频整体关掉，楼脚一片平涂、楼腰突然有细节。
fn weather_stain(world_pos: vec3<f32>, nrm: vec3<f32>, deriv: vec3<f32>) -> f32 {
    let an = abs(nrm);
    var p = world_pos.xz;
    var px = max(deriv.x, deriv.z);
    if (an.x > an.y && an.x > an.z) {
        p = world_pos.zy;
        px = max(deriv.y, deriv.z);
    } else if (an.z > an.x && an.z > an.y) {
        p = world_pos.xy;
        px = max(deriv.x, deriv.y);
    }
    let fine_k = 1.0 - smoothstep(0.05, 0.22, px);
    let coarse_k = 1.0 - smoothstep(0.25, 1.0, px);
    let w = fine_k + coarse_k;
    if (w <= 0.001) {
        return 1.0;
    }
    let n = (vnoise2(p * 1.7) * fine_k + vnoise2(p * 0.31) * coarse_k) / w;
    return 1.0 + (n - 0.5) * 0.14;
}

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
    let deriv = world_derivatives(input.world_pos);
    var shadow_factor = 0.0;
    var debug_outside = true;
    var d_avg = 0.0;
    var frag_depth = 0.0;
    if (light_data.flags.y >= 0.5 && light_data.shadow.bias.z >= 0.5) {
        // ---- 光空间帧尺度：从 light_view_proj 反解，零新增 uniform / 零新 pass ----
        // 正交光矩阵的 3x3 块 = diag(1/extent, 1/extent, 1/(near-far)) · 光相机单位基向量，
        // 因此**每一行的长度就是该轴的投影缩放**（基向量单位长，与旋转无关）。
        // 一次性脚本复现 glam 0.29.3 look_to_rh + orthographic_rh 逐位核对过：
        //   |行0| = 1/400 → extent = 400m；|行2| = 1/799 → 深度 1.0 = 799m（正交→线性）；
        //   -normalize(行2) = (-0.3885, 0.8742, -0.2914) = game.rs 的 sun.direction（指向太阳）；
        //   米/texel = 2*400/2048 = 0.390625。
        let lvp = light_data.shadow.light_view_proj;
        let row_x = vec3<f32>(lvp[0].x, lvp[1].x, lvp[2].x);
        let row_z = vec3<f32>(lvp[0].z, lvp[1].z, lvp[2].z);
        let extent_m = 1.0 / max(length(row_x), 1e-6);
        let depth_m = 1.0 / max(length(row_z), 1e-6);
        let map = max(light_data.shadow.config.x, 256.0);
        let texel = 1.0 / map;
        let m_per_texel = 2.0 * extent_m / map;
        let to_light = -normalize(row_z);
        // ---- bias：从"沿光轴 4 米"改成"texel 与斜率自适应的法线偏移" ----
        // 旧实现只有一个常量深度 bias = shadow.bias.x = 0.005，而本项目光空间深度是
        // **线性**的（正交，1.0 = far-near = 799m）→ 0.005 等价于沿光轴 4.0 米。
        // 受光面到遮挡物的光轴间距每离开墙根 1m 只涨 0.486m（= |L 的水平分量|），于是：
        //   · 高度 < 3.5m 的物体（木箱/油桶/沙袋/汽车/灯杆/整条窗带）对地面的间距
        //     不足 4m → **一个阴影都不剩**；
        //   · 9m 高的楼影全长 5.0m，靠楼根 1.9m 内被吃掉 → 影子整体脱离楼体漂在地上；
        //   · 0.30/0.44/0.62m 的立面进深台阶（窗带/层线/壁柱）不可能自阴影 → 楼体零层次。
        // 这三条合起来就是玩家原话："阴影像刻意贴在地面上的一张贴图" + "楼像纸片/不 3D"。
        // 新做法：① 采样点沿法线外推 push_m = (1.25 + 0.9·斜率)·texel，斜率封顶 1.6
        // （acne 只出现在掠光方向，按斜率给才不用一刀切压暗全场；不封顶的话背光面
        //  斜率→∞，横推 2m 反而把墙根的影子从地面上抹掉）；② 只留 0.12m 常量深度
        // bias 兜浮点余量；③ **bias.x 的语义改为"米"**并 clamp 到 ≤0.5m：现值 0.005
        // 旧语义是 4m、新语义是 5mm≈无，所以不改 Rust 侧数值就能立刻拿到正确行为
        // （要回退请改 lighting.rs::DEFAULT_SHADOW_DEPTH_BIAS，别再指望 shader 兼容旧量纲）。
        let ndotl = clamp(dot(normal, to_light), 0.0, 1.0);
        let slope = min(sqrt(max(0.0, 1.0 - ndotl * ndotl)) / max(ndotl, 0.25), 1.6);
        let push_m = light_data.shadow.bias.y + m_per_texel * (1.25 + 0.9 * slope);
        let bias_d = (min(light_data.shadow.bias.x, 0.5) + 0.12) / depth_m;
        let sp = lvp * vec4<f32>(input.world_pos + normal * push_m, 1.0);
        let shadow_uv = vec2<f32>(sp.x * 0.5 + 0.5, 1.0 - (sp.y * 0.5 + 0.5));
        // glam ortho_rh（本版本）产出 [0,1] 深度，与 GPU 写入的阴影图同基准，
        // 禁止再乘 0.5+0.5（OpenGL [-1,1] 旧映射，二重偏移 +0.25 曾致全场误判阴影）。
        frag_depth = sp.z;
        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0
            && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0
            && sp.z >= 0.0 && sp.z <= 1.0) {
            debug_outside = false;
            // 核**对齐到 texel 中心**：不 snap 时核的相位随像素在世界里滑，同一个像素的
            // 9 次比较结果在 0/1 间随机翻 → 阴影边界每隔 0.39m 爬一条线，相机一动整片
            // 影子轮廓是"活的"（缺陷单"移动时大量线条"的第三条）。snap 后核相对 texel
            // 网格相位恒定，边界改由下面的分数深度测试连续过渡。
            let base_uv = (floor(shadow_uv * map) + vec2<f32>(0.5)) * texel;
            // 中心一次采样估"遮挡物→受光面"的光轴距离 → 接触处硬、远处半影张开。
            // 0.06 是把太阳张角放大约 6 倍的美术值（真实 0.53°→0.009，会硬得像刀切）。
            // 再按下限 = 一个像素覆盖的 shadow texel 数加宽：像素盖得下多个 texel 时，
            // 任何小于足迹的核都会重新量化成台阶（等价于必须按 mip 模糊）。
            let d_c = textureSample(shadow_map, shadow_sampler, base_uv);
            let gap_m = max(0.0, frag_depth - bias_d - d_c) * depth_m;
            let foot_m = max(deriv.x, max(deriv.y, deriv.z));
            let pen_m = clamp(0.06 * gap_m, 0.45, 4.0) + foot_m;
            // 核半径（uv 单位）：半影米数 → texel 数 → uv，钳在 [1.5, 5] texel
            let step_uv = clamp(pen_m / m_per_texel, 1.5, 5.0) * texel;
            // 深度方向的软过渡宽度：至少一个 texel，否则 NEAREST 读数的台阶又是硬跳变
            let w_d = max(pen_m, m_per_texel) / depth_m;
            var occluded = 0.0;
            var dsum = 0.0;
            for (var dy = -1; dy <= 1; dy = dy + 1) {
                for (var dx = -1; dx <= 1; dx = dx + 1) {
                    let d = textureSample(shadow_map, shadow_sampler,
                        base_uv + vec2<f32>(f32(dx), f32(dy)) * step_uv);
                    dsum = dsum + d;
                    // 分数测试（percentage-closer filtering）代替 if/>+1.0：
                    // 把"9 个 0/1 计票"变成连续量，影子里侧到外侧是渐变而不是 9 档跳变
                    occluded = occluded + smoothstep(0.0, w_d, frag_depth - bias_d - d);
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
    // 半球环境光：上方取天光（冷、满量），下方取地面反弹（暖、约 0.4 倍）。
    // 原来是单一常数环境项 → 檐下、凹角、物体底面与开阔面一样亮，所有东西像贴
    // 在背景上、没有"坐"在地上。分半球后接触面自然压暗，体积感立刻出来。
    //
    // 阴影必须**同时吃掉一部分天光**（sky_occ）：旧实现影子只乘方向光，阴影区的环境
    // 项与开阔地完全等亮 → 影子边缘一过就是一片均匀灰、没有厚度，这也是"贴图像"的一半。
    // 地面反弹不受头顶遮挡影响，所以只压天光那一支。
    let up = clamp(normal.y * 0.5 + 0.5, 0.0, 1.0);
    let sky_occ = 1.0 - 0.35 * shadow_factor;
    var radiance = light_data.ambient.rgb * light_data.ambient.w
        * mix(vec3<f32>(0.55, 0.47, 0.38), vec3<f32>(1.0, 1.02, 1.10) * sky_occ, up);
    radiance = radiance + evaluate_directional(light_data.directional, normal, view_dir, shininess) * (1.0 - shadow_factor);
    for (var i = 0u; i < 4u; i = i + 1u) {
        radiance = radiance + evaluate_point(light_data.points[i], input.world_pos, normal, view_dir, shininess);
    }
    // 原实现 min(radiance, 1) 硬截顶：太阳强度 1.5 时所有 NdotL > 0.5 的面全部饱和成
    // 同一个 1.0（水平屋顶与 30° 斜面完全同色），大面朝向梯度被抹平成一片"塑料"。
    // 改指数压缩：单调、永不截顶，越亮压缩越狠但顺序不变，朝向差异得以保留。
    let tone = vec3<f32>(1.0) - exp(-radiance * 1.55);
    return color * tone * weather_stain(input.world_pos, normal, deriv);
}

// ============================================================
// 立面图案（世界坐标定义 + 严格盒式平均；只在 marker 分支调用）
// ============================================================
// ⚠ FLOOR_H 必须等于 city.rs 的 FLOOR_H（3.15m）。层带/层线/壁柱/女儿墙全按它排布，
// 着色器里的"层相位"若与它不同步，暗带就会画在**看得见的**混凝土裙墙和层线上，而不是
// 画在已被真实窗带盒遮住的玻璃位置 → 整栋楼被切成一道亮一道黑的空框架。
// （旧值用 3.0m 周期：每层错 0.15m，20 层累积 3m ≈ 一整层，正好把暗带推到混凝土上。）
const FLOOR_H: f32 = 3.15;
// 幕墙分格宽度（米）：16m 宽的楼 ≈ 11 格；金属竖梃 0.055*2*1.45 ≈ 16cm
const GLASS_PANE_W: f32 = 1.45;
// city.rs::building 的窗带在本层里的标高区间（band_base = fy+0.62，band_top = fy+2.73）
const BAND_LO: f32 = 0.62;
const BAND_HI: f32 = 2.73;

// 沿墙横向的世界坐标 u + 该轴的米/像素足迹（两者必须同源，否则竖梃周期与收敛量不匹配）
fn facade_u(nrm: vec3<f32>, world_pos: vec3<f32>, deriv: vec3<f32>) -> vec2<f32> {
    // 法线主轴是 X → 该面由 (y,z) 张成 → 横向是 z；主轴是 Z → 横向是 x。
    // ⚠ 旧 D11 恰好选反（对 |n.x|>|n.z| 的面取 world_pos.x，而该面上 x 是**常数**）
    // → 竖梃项在每面墙上都退化成同一个常数 → 一根竖梃都画不出来，窗带读成通长的
    // 近黑横条（缺陷单第 1 条最直接的成因；它只影响带竖梃的那一项，所以一直没被查到这里）。
    let x_face = abs(nrm.x) > abs(nrm.z);
    return vec2<f32>(select(world_pos.x, world_pos.z, x_face),
                     select(deriv.x, deriv.z, x_face));
}

// 玻璃幕墙底色调制：逐层上亮下暗 + 金属竖梃 + 逐格随机（"这格卷帘放下了/那格开着灯"）。
// 均值刻意压在 ≈0.95：玻璃必须仍比混凝土墙暗一档，只是不再是一片死平的黑。
fn glass_shade(nrm: vec3<f32>, world_pos: vec3<f32>, deriv: vec3<f32>) -> f32 {
    let axes = facade_u(nrm, world_pos, deriv);
    let u = axes.x;
    let du = axes.y;
    // ① 逐层上亮下暗：同层玻璃顶部反射天顶、下沿望进室内深处。与真实窗带同锁相位，
    //    所以带体内（本层的 BAND_LO~BAND_HI）正好走完 1.28→0.74 这一趟渐变。
    let ph = fract(world_pos.y * (1.0 / FLOOR_H));
    let grad = mix(1.28, 0.74, smoothstep(BAND_LO / FLOOR_H, BAND_HI / FLOOR_H, ph));
    // ② 竖梃：金属框比玻璃亮，把"一整条玻璃"切成一格一格（没有它，窗带就是悬挑黑鳍片）。
    //    相位 +0.5 让梃落在 [0,1) 内部——否则盒式平均会在 fract 的接缝处算错。
    let pu = fract(u * (1.0 / GLASS_PANE_W) + 0.5);
    let half_u = clamp(du * (0.5 / GLASS_PANE_W), 1e-4, 0.5);
    let mull = band_coverage(pu, half_u, 0.5 - 0.055, 0.5 + 0.055);
    // ③ 逐格随机：格号 = (开间, 层)。这是**世界哈希**，一个像素盖不满一格时必须收敛，
    //    否则就是移动时的闪点。（旧实现用 fract(sin(dot(world_pos.xz))) 逐像素随机，
    //    且收敛量取自 fwidth(uv)——而 ICO/SPH 模板的所有顶点 uv 恒为 (0,0)，
    //    fwidth=0 → detail 恒等于 1 → 树冠噪声在**任何距离**都是满幅，见 fs_main 的
    //    is_canopy 分支（那条已改成世界值噪声 + 按米收敛）。）
    let near = 1.0 - smoothstep(0.10, 0.45, max(du, deriv.y));
    let pane_i = i32(floor(u * (1.0 / GLASS_PANE_W)));
    let floor_i = i32(floor(world_pos.y * (1.0 / FLOOR_H)));
    let rooms = mix(1.0, 0.62 + 0.50 * lattice_hash(pane_i, floor_i), near);
    return grad * rooms * (1.0 + 0.55 * mull);
}

// 中性混凝土立面上"窗洞"的暗化量 ∈ [0,1]。半宽上限 0.5 = 一整层：取到时结果恰好等于
// 一个完整周期的平均覆盖率，于是远处自然收敛到真实灰底而不是越远越黑（盒式平均自带）。
fn window_dark(nrm: vec3<f32>, world_pos: vec3<f32>, deriv: vec3<f32>) -> f32 {
    let axes = facade_u(nrm, world_pos, deriv);
    let half_y = clamp(deriv.y * (0.5 / FLOOR_H), 1e-4, 0.5);
    let ph = fract(world_pos.y * (1.0 / FLOOR_H));
    let win = band_coverage(ph, half_y, BAND_LO / FLOOR_H, BAND_HI / FLOOR_H);
    let pu = fract(axes.x * 0.5 + 0.5);
    let half_u = clamp(axes.y * 0.25, 1e-4, 0.5);
    let mull = band_coverage(pu, half_u, 0.5 - 0.07, 0.5 + 0.07);
    return win * (1.0 - 0.7 * mull);
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
        // 世界足迹 + 面法线在这里**统一算一次**：原来 D11、NPC 轮廓分离各自
        // normalize(cross(dpdx,dpdy)) 一遍，且都在 if 分支里取导数（分支在面的边界上
        // 非一致控制流，fwidth 属未定义），统一提到分支外既省钱也更稳。
        let deriv = world_derivatives(input.world_pos);
        let dmax = max(deriv.x, max(deriv.y, deriv.z));
        let vdir = normalize(input.view_dir);
        let cr = cross(dpdx(input.world_pos), dpdy(input.world_pos));
        var fnrm = normalize(cr);
        if (dot(fnrm, vdir) < 0.0) {
            fnrm = -fnrm;
        }
        // 退化法线判据：|cross| / |足迹|² ≈ 两条屏幕导数向量的夹角正弦。
        // 三角形轮廓/亚像素面上它趋 0 → 法线是纯噪声。玻璃的菲涅耳项若不加这道闸，
        // ndv 会随机取到 0 → fres=1 → **每栋楼的轮廓长出一圈亮边**（自己引入的走样）。
        let valid_nrm = smoothstep(0.03, 0.22, length(cr) / max(dmax * dmax, 1e-6));
        // D12：皮肤/迷彩是**逐面 0..1 UV** 域的信号，所以它的收敛量必须用 fwidth(uv)
        // （一个像素盖过面的 1.5% 就该收）。⚠ 这个 detail 不能拿去收敛任何"按米定义"
        // 的信号（窗带/玻璃分格/树冠斑驳）：面的物理尺寸从 0.3m 木箱到 30m 大楼差 100 倍，
        // 同一个 detail 值意味着差 100 倍的世界频率——那条纪律现在由 world_derivatives 承担。
        let foot = max(abs(fwidth(input.uv).x), abs(fwidth(input.uv).y));
        let detail = 1.0 - smoothstep(0.015, 0.22, foot);
        // 外部建模网格（vs_main 给 flat_flag = 1.25）：下面四条"给纯 tint 盒子补细节"的
        // 程序化效果一律跳过，只保留 base = 顶点色 → 光照 → 阴影 → 雾。
        // 不跳过的后果是实打实的：窗带按 FLOOR_H=3.15 画在 3.4m 楼层的 GLB 立面上、
        // 每层错位 0.25m 越往上漂越多；混凝土皮肤按"每面 0..1 UV"采样，而 GLB 是世界
        // 投影 UV，会把墙纹任意铺开。
        let authored = input.flat_flag > 1.1 && input.flat_flag < 1.4;
        // 玻璃判据（b > r*1.4）沿用 D2 不动；但必须提到皮肤分支**之外**：
        // 旧代码写在 if (flags.z >= 0.5) 里面，RV3D_SKIN_TEX=0 时这条路径整体不成立。
        let is_glass = !authored && input.flat_flag < 1.5 && input.color.b > input.color.r * 1.4;
        let is_canopy = !authored
            && input.flat_flag < 1.5
            && base.g > base.r && base.g > base.b * 1.4;
        // 树冠杂色（2026-08-23 纸片树修复的第二次修正）：旧实现是**逐像素**随机
        // fract(sin(dot(world_pos.xz))*43758) 且按 uv 域的 detail 收敛——而 mesh 路径
        // ICO/SPH 模板所有顶点 uv 恒为 (0,0) → fwidth(uv)=0 → detail 恒为 1 →
        // 满幅逐像素白噪覆盖整个树冠，站远看就是"一簇彼此分离的、闪着噪点的扁平多边形"，
        // 相机一移动噪点整体沸腾（缺陷单"移动时大量线条"的第四条来源）。
        // 改成：① 世界坐标**值噪声**（连续、可平均，格距 ~1.1m = 一团叶子的大小）；
        //      ② 按米/像素足迹收敛，一个像素盖过 0.25m 就退场。
        if (is_canopy) {
            let lump = 1.0 - smoothstep(0.06, 0.25, dmax);
            let v = vnoise2(input.world_pos.xz * 0.9);
            base = base * mix(1.0, 0.80 + 0.40 * v, lump);
        }
        if (light_data.flags.z >= 0.5 && input.flat_flag > 1.5) {
            // NPC 士兵：迷彩军服纹理 × 阵营 tint（去饱和纹素保留色相）
            let texel = textureSample(npc_skin_tex, texture_sampler, input.uv);
            let luma = dot(texel.rgb, vec3<f32>(0.299, 0.587, 0.114));
            // 近处幅度 0.55+0.90·luma 与 D1 验收式逐位一致（勿回退），远处收到 0.55+0.12·luma
            base = input.color * (0.55 + (0.12 + 0.78 * detail) * luma);
        } else if (light_data.flags.z >= 0.5 && !is_glass && !authored) {
            // marker 障碍：混凝土墙纹理 × 障碍 tint（近期权重 0.45：tint 保色相，纹理供细节）
            base = mix(input.color,
                       textureSample(marker_skin_tex, texture_sampler, input.uv).rgb,
                       0.45 * (0.25 + 0.75 * detail));
        }
        // 玻璃底面：逐层渐变 + 分格 + 逐格随机（见 glass_shade）。
        // 只给**近竖直**的面画"层/格"图案：天窗、占领底盘这类横放的蓝面没有楼层可言，
        // 让它们保持纯色 tint + 下面的天空反射（屋顶玻璃本来就该泛天光）。
        if (is_glass) {
            let gfac = smoothstep(0.35, 0.75, 1.0 - abs(fnrm.y));
            base = base * mix(1.0, glass_shade(fnrm, input.world_pos, deriv), gfac);
        }
        // D11（第二次修正）：窗带由片元画在**中性混凝土立面**上，零新增几何、零深度风险。
        // 分类一律用 input.color（逐实例常量），不用被皮肤纹理改写过的 base。
        // 通道极差判据天然排除砖墙(红)/木掩体(棕)/树冠(绿)——否则围墙上会长出窗户。
        if (!authored && input.flat_flag < 1.5 && !is_glass) {
            let cmax = max(input.color.r, max(input.color.g, input.color.b));
            let cmin = min(input.color.r, min(input.color.g, input.color.b));
            let neutral = 1.0 - smoothstep(0.06, 0.14, cmax - cmin);
            let vert = 1.0 - abs(fnrm.y);
            // 窗带只从 3m（二层）以上开始：混凝土围墙/矮裙房同样是中性灰，
            // 没有这道下限会在墙上画出一条通长深色窗带。
            if (vert > 0.5 && neutral > 0.01 && input.world_pos.y > 3.0) {
                // 暗化 0.22：玻璃带应比混凝土立面暗一档，而不是变成黑洞。
                // 收敛由 window_dark 的盒式平均负责（严格守恒覆盖率），不再挂 detail。
                base = base * mix(1.0, 1.0 - 0.22 * window_dark(fnrm, input.world_pos, deriv),
                                  vert * neutral);
            }
        }
        // 轮廓分离（D12）：掠射角提亮，让士兵从背景里"读得出来"。
        // 只乘一个亮度标量、不改 RGB 分量比例 → 阵营色相与饱和度保持 D1 验收结论有效。
        if (input.flat_flag > 1.5) {
            base = base * (1.0 + 0.45 * pow(1.0 - abs(dot(fnrm, vdir)), 3.0));
        }
        // 障碍/NPC：应用 Blinn-Phong（同地面路径）——消除纯色剪影的纸片感
        let lit = apply_lighting(input, base);
        var shaded = lit;
        if (is_glass && light_data.flags.x >= 0.5) {
            // ---- 玻璃反射通路（缺陷单第 1 条的另一半）----
            // 旧实现只做了"玻璃跳过皮肤纹理、直出 tint"→ 一整面死平的近黑，
            // 玩家原话"建筑物里面的墙壁就像一片纸""能看穿"。真实玻璃的可辨识度不是
            // "暗"，而是：掠射角泛出天空（菲涅耳）、镜面方向有一道太阳 glint。
            // 这里给玻璃加一条独立的反射通路，**而不是把它提亮成灰墙**。
            let ndv = clamp(dot(fnrm, vdir), 0.0, 1.0);
            let fres = (0.04 + 0.96 * pow(1.0 - ndv, 5.0)) * valid_nrm;
            let rr = reflect(-vdir, fnrm);
            let up_t = clamp(rr.y * 0.5 + 0.5, 0.0, 1.0);
            // 反射向天 → 取天空色（与 FOG_TINT 同源，远处"楼—天"交界才连续）；
            // 反射向地 → 取暖而暗的地面反弹（楼脚下沿最暗，正是玻璃的样子）。
            let sky_col = mix(vec3<f32>(0.26, 0.23, 0.20),
                              FOG_TINT * vec3<f32>(1.30, 1.26, 1.12), up_t);
            // 影子里的天空反射要压一档（头顶被遮 → 看到的天空立体角变小）。
            // 用"光照结果 / 底色"反推该像素的照明水平，省掉第二次阴影采样、
            // 也不需要给 apply_lighting 加输出参数。
            let illum = min(1.0, dot(lit, vec3<f32>(0.3333))
                          / max(dot(base, vec3<f32>(0.3333)), 1e-3));
            var sheen = fres * sky_col * mix(0.30, 1.0, illum) * (0.30 + 0.70 * up_t);
            if (light_data.directional.direction.w >= 0.5) {
                // 太阳 glint：幂 140 → 一条很窄的高光带，随相机移动在楼面上滑过。
                // 这是"认出那是玻璃"最便宜也最强的一条线索，且它只出现在镜面角上，
                // 不会把整面楼提亮。
                sheen = sheen + pow(max(dot(rr, normalize(light_data.directional.direction.xyz)), 0.0), 140.0)
                      * light_data.directional.color_intensity.rgb
                      * light_data.directional.color_intensity.w * 0.5;
            }
            shaded = lit + sheen;
        }
        let fg = fog_amount(view_distance(input));
        return vec4<f32>(mix(shaded, FOG_TINT, fg) * input.fade, 1.0);
    }
    // 世界空间 UV：地面/地形用 world_pos.xz 映射到全图 [0,1]（覆盖 2*256 米），
    // 与 procedural.rs 烘焙纹理严格对齐（marker/NPC/自发光已走 flat_flag 纯色路径，不采样）。
    let world_uv = (input.world_pos.xz + vec2<f32>(256.0, 256.0)) / 512.0;
    let texel = textureSample(texture_sampled, texture_sampler, world_uv);
    // 地面/地形：**烘焙纹理是唯一的颜色来源**，顶点色只做乘法性明暗调制。
    //
    // 旧式 `mix(input.color, texel.rgb, 0.75)` 是**加法**混合，它给每个像素抬了一个
    // 与内容无关的底：沥青纹素线性 0.11、人行道方砖 0.25（原图对比 2.3:1），代进
    // 0.25 权重的底（传统路径 input.color = 地面实例 tint 0.7 灰，mesh 路径 =
    // terrain_tint 的草/沙绿，亮度 0.15~0.55）→
    //   沥青 0.25·0.7 + 0.75·0.11 = 0.2575；方砖 0.25·0.7 + 0.75·0.25 = 0.3625
    //   → 对比塌到 1.41:1，而且整层地面被抬亮 2~3 倍（"惨白"）。
    // 更本质的两条：① mip 过滤只可能把暗部**平均**掉，不可能让最暗的东西比最亮的
    // 东西还亮，所以这不是纹理过滤问题；② mesh 路径抬上来的那 25% 是 terrain_tint 的
    // **草绿**——它与烘焙图里的城市分区色是两个互不相关的噪声场，所以玩家看到的街面
    // 绿雾与 city_zone_color 写的沥青/方砖毫无关系，改基色当然"几乎看不出效果"。
    // 乘法调制保留地形 LOD 的顶点色约定（mesh 路径 terrain_tint 的草地/沙粒分域仍
    // 参与明暗，只是不再染色、不再抬底），同时把原始对比完整送进光照。
    let gtone = dot(input.color, vec3<f32>(0.2126, 0.7152, 0.0722));
    var mixed = texel.rgb * mix(0.85, 1.15, clamp(gtone, 0.0, 1.0));
    // ---- 地面微细节层（procedural.rs::generate_default_ground_detail_texture）----
    // 烘焙地面纹理锚定在"512² 覆盖 512m"上 → 一个纹素 = 1 米，街面上任何厘米级质感
    // 都不存在，近看就是一张平涂灰纸（"二维化的一个面"在地面上的那半条表现）。
    // 细节图按 GROUND_DETAIL_METRES 平铺 → 7.8mm/纹素，两个数量级的差距。
    // ⚠ 闸门必须用**米/像素**，不能用视距（本项目已踩过一次：视距闸门在掠射角下
    // 混叠成满屏白点）。每像素盖到 0.25m（=32 纹素）时完全退出。
    let gderiv = world_derivatives(input.world_pos);
    let gpx = max(gderiv.x, gderiv.z);
    let gdetail = 1.0 - smoothstep(0.06, 0.25, gpx);
    // ⚠ 本层由 light_data.flags.w 显式开关，默认关（2026-09-03 定位的"地面全黑/街区黑块"
    // 根因）。binding 9（ground_detail_tex）是本文件里唯一"片元声明了、管线却可能没绑定"
    // 的槽位：set layout 缺项或任一帧描述符漏写 → 驱动给空描述符 → 采样恒 0 → 本层乘法
    // 把 albedo **乘成 0**。gpx < 0.06 米/像素（相机周边整圈地面 = 全部街面/广场）时
    // gdetail=1 → 纯黑；远处掠射 gpx > 0.25 → gdetail=0 → 画面正常，所以症状是"近处一圈
    // 黑、远处正常"，看着像"没画、露出 clear color"，其实是画了再乘 0。这条乘法位于地面/
    // 地形分支（flat_flag <= 0.5 才走到）→ marker/NPC/自发光完全不受影响；且与光照、阴影、
    // 纹理内容无关 —— 精确复现交接文档那三条"三重排除"（NO_SHADOW=1 仍黑 / 光照开关同黑 /
    // 纹理 dump 完美）以及"改城市分区基色无 measurable 效果"（乘 0 之后改什么都没区别）。
    // mesh 与传统两条路径**共用本片元**（mesh 管线加载 assets/triangle.frag.spv）⇒ 同病；
    // 两路径的 gdetail 逐像素相同（gpx 只取 world_pos 的 x/z 屏幕导数，terrain_bump 只动
    // y），mesh 侧唯一的额外变暗是 write_vertex 用 terrain_tint 顶掉实例 tint
    // （gtone 0.7 → ≈0.25 ⇒ mixed 系数 1.05 → 0.93，约 12%），不足以解释"全黑"。
    // 接线方（renderer.rs）必须：binding 9 进 set layout + 每帧写描述符 + UNORM(线性)view
    // + ≥6 级 mip，**并把 light_data.flags.w 置 1.0**；缺任何一条本层就不参与，
    // 地面保持无细节层的正常画面（fail-closed，绝不再变黑）。
    if (gdetail > 0.001 && light_data.flags.w >= 0.5) {
        // 显式 LOD：一个像素盖多少个纹素就取第几级 mip。
        // 为什么不用隐式 textureSample：它在非一致控制流（这个 if）里取屏幕导数属于
        // 未定义行为，而且这里本就是按米算出来的，没必要绕一圈导数。
        let lvl = log2(max(gpx / GROUND_DETAIL_TEXEL_M, 1.0));
        let g = textureSampleLevel(ground_detail_tex, texture_sampler,
                                   input.world_pos.xz / GROUND_DETAIL_METRES, lvl).r;
        // 纹素编码约定：r = 亮度调制 / 2（调制 1.0 → 128）。见 GROUND_DETAIL_GAIN 注释。
        // 二级兜底（flag 已开但图本身有问题时）：clamp 到 [0.5,2.0] 且把 g<=0 判为"该层
        // 不参与"。真实纹素域 ≈[0.33,0.61]（procedural.rs::generate_ground_detail_texture
        // 注释"永不取到 0"）⇒ 调制域 ≈[0.66,1.22] 整个落在带内，对正确接线逐位无影响。
        var gmod = clamp(g * GROUND_DETAIL_GAIN, 0.5, 2.0);
        if (g <= 0.0) {
            gmod = 1.0;
        }
        mixed = mixed * mix(1.0, gmod, gdetail);
    }
    if (light_data.flags.x < 0.5) {
        return vec4<f32>(mixed * input.fade, 1.0);
    }
    let lit = apply_lighting(input, mixed);
    let fg2 = fog_amount(view_distance(input));
    return vec4<f32>(mix(lit, FOG_TINT, fg2) * input.fade, 1.0);
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
// MARKER_INSTANCE_BASE=65537、NPC_INSTANCE_BASE=65537+8192=73729、
// EMISSIVE_INSTANCE_BASE=NPC_INSTANCE_BASE+9216=82945（NPC 三几何区：盒/圆柱/球，各 3072）
const TERRAIN_INSTANCE_INDEX: u32 = 65536u;
const MARKER_INSTANCE_BASE: u32 = 65536u + 1u;
const NPC_INSTANCE_BASE: u32 = 65536u + 1u + 8192u; // marker 区 = MAX_MARKER_INSTANCES(8192)，与 renderer.rs 对齐（2026-09-01 建模重构：1024 装不下真城市；实测 CPU 剔除 4034 个 marker 只花 20µs，所以容量不是瓶颈，再翻一档到 8192。改容量必须同步改本行两处副本 + renderer.rs + 枪槽字面量，见 gun_slot_layout_is_pinned）
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

// 本次 mesh draw 的起始实例槽：地面=0 / marker=65537 / NPC=73729 / 自发光=82945 / 枪=83009
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
    // 与 vs_main 同步：不归一化，片元用 length() 取真实视距驱动雾。
    v.view_dir = cam - wp.xyz;
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
    // 枪模槽位 = GUN_INSTANCE_INDEX（83009；旧式 NPC_INSTANCE_BASE+1024-16 是 1024 时代
    // 残留，范围 67569..75777 把 NPC 圆柱/球体段全部误判为枪 → 四肢/头被 z=0 深度覆盖，
    // 「鬼魂/穿模」观感的另一来源；mesh 路径不画枪模，此判定仅保护传统 draw 的 GUN 槽语义）。
    let is_gun = slot == 83009u;

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
    // ---- 形状标签分派（2026-09-01 建模重构；标签语义见 src/engine/geom.rs）----
    // 此前 marker 的形状是从 tint 颜色**猜**的（is_foliage：绿色→二十面体，其余→立方体），
    // 等于"想要圆的就不能有颜色"，所以整座 5×5 街区只能画 43 个盒子。现在形状是数据：
    // 写在 tint.w（marker 带此前恒为 1.0，且片元只用 tint.rgb，是现成的空闲位）。
    // 只有 marker 带读标签——NPC/自发光带的形状与 flat_flag 材质模式都由槽位决定。
    let shape_tag = inst.tint.w;
    let m_cyl = is_marker && shape_tag > 1.5 && shape_tag < 2.5;
    let m_ico = is_marker && shape_tag > 2.5 && shape_tag < 3.5;
    let m_sph = is_marker && shape_tag > 3.5 && shape_tag < 4.5;
    // 过渡兜底：只有"未打标签"（Shape::Legacy = 1.0）的绿色 marker 才沿用旧的颜色嗅探，
    // 这样 main.rs 里手写 tint=[r,g,b,1.0] 的掩体/植被画面逐位不变；显式 Shape::Box(0.0)
    // 不受影响。等所有构造点都显式打标后可删掉这一行。
    let is_tree = is_foliage(inst.tint) && shape_tag > 0.5 && shape_tag < 1.5;
    if (is_npc_cyl || m_cyl) {
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
    } else if (is_npc_sph || m_ico) {
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
    } else if (is_glow || m_sph) {
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