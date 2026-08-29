# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
old = '{ let mut bb = spv_rt_bench::rt_bench_spv(); let mut by = Vec::with_capacity(bb.len()*4); for w in &bb { by.extend_from_slice(&w.to_le_bytes()); } let _ = std::fs::write(Path::new(&out_dir).join("rt_bench.spv"), &by); }'
new = old + """
    { let bb = spv_rt_bench::rt_bench_spv();
      match naga::front::spv::parse_u32_slice(&bb, &naga::front::spv::Options::default()) {
        Ok(_) => println!("cargo:warning=RT_SPV_PARSE_OK"),
        Err(e) => println!("cargo:warning=RT_SPV_PARSE_ERR: {}", e.to_string().chars().take(300).collect::<String>()),
      } }"""
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('self-check added')
else:
    print('anchor missing')
