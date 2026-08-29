# -*- coding: utf-8 -*-
import io
s = io.open('build.rs', encoding='utf-8').read()
old = '    output.push_str(&format!("pub const MESH_SPIRV: &[u32] = &{:?};\\n", mesh_spirv));'
new = old + '\n    output.push_str("/// RT 求交基准 SPIR-V（手工汇编；naga 不支持 WGSL ray-query）\\n");\n    output.push_str("#[allow(dead_code)]\\n");\n    output.push_str(&format!("pub const RT_BENCH_SPV: &[u32] = &{:?};\\n\\n", spv_rt_bench::rt_bench_spv()));'
if old in s:
    s = s.replace(old, new, 1)
    io.open('build.rs', 'w', encoding='utf-8', newline='').write(s)
    print('RT_BENCH push added')
else:
    print('anchor missing')
