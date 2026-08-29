# -*- coding: utf-8 -*-
import io
s = io.open('build.rs', encoding='utf-8').read()
if 'path = "build_spv_rt.rs"' not in s:
    s = s.replace("fn main() {", '#[path = "build_spv_rt.rs"]\nmod spv_rt_bench;\n\nfn main() {', 1)
    # 生成 RT_BENCH_SPV 到 shaders.rs
    s = s.replace("""    output.push_str("/// 网格着色器 SPIR-V 字节码\\n");
    output.push_str("#[allow(dead_code)]\\n");
    output.push_str(&format!("pub const MESH_SPIRV: &[u32] = &{:?};\\n\\n", mesh_spirv));""", """    output.push_str("/// 网格着色器 SPIR-V 字节码\\n");
    output.push_str("#[allow(dead_code)]\\n");
    output.push_str(&format!("pub const MESH_SPIRV: &[u32] = &{:?};\\n\\n", mesh_spirv));
    output.push_str("/// RT 求交基准 SPIR-V（手工汇编；naga 不支持 WGSL ray-query）\\n");
    output.push_str("#[allow(dead_code)]\\n");
    output.push_str(&format!("pub const RT_BENCH_SPV: &[u32] = &{:?};\\n\\n", spv_rt_bench::rt_bench_spv()));""")
    io.open('build.rs', 'w', encoding='utf-8', newline='').write(s)
    print('integrated')
else:
    print('already')
