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

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) fade: f32,
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
    output.position = camera.proj * camera.view * inst.model * vec4<f32>(position, 1.0);
    output.color = color * inst.tint.rgb;
    output.uv = uv;
    if (instance_index == TERRAIN_INSTANCE_INDEX) {
        output.fade = 1.0;
    } else {
        // 相机世界位置：view = [R|t]，相机位置 = -R^T * t（刚体变换）
        let t = camera.view[3].xyz;
        let cam_pos = -(camera.view[0].xyz * t.x + camera.view[1].xyz * t.y + camera.view[2].xyz * t.z);
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
}

@group(0) @binding(1) var texture_sampled: texture_2d<f32>;
@group(0) @binding(3) var texture_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if (input.fade <= 0.02) {
        discard;
    }
    let texel = textureSample(texture_sampled, texture_sampler, input.uv);
    let mixed = mix(input.color, texel.rgb, 0.5) * input.fade;
    return vec4<f32>(mixed, 1.0);
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

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let vs_spirv = compile_wgsl(VERTEX_SHADER_WGSL);
    let fs_spirv = compile_wgsl(FRAGMENT_SHADER_WGSL);
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
    println!("cargo:info=着色器编译完成");
}
