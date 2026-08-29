# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
anchor = '    output.push_str(&format!("pub const RT_BENCH_SPV: &[u32] = &{:?};\\n\\n", spv_rt_bench::rt_bench_spv()));'
if anchor in s:
    # 写文件 + word 计数
    s = s.replace(anchor, anchor + '\n    { let mut bb = spv_rt_bench::rt_bench_spv(); let mut by = Vec::with_capacity(bb.len()*4); for w in &bb { by.extend_from_slice(&w.to_le_bytes()); } let _ = std::fs::write(Path::new(&out_dir).join("rt_bench.spv"), &by); }', 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('dump added')
else:
    print('anchor missing')
