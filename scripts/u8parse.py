# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("match naga::front::spv::parse_u32_slice(&bb, &naga::front::spv::Options::default()) {", "match naga::front::spv::parse_u8_slice(&by, &naga::front::spv::Options::default()) {")
# by 需在作用域（dump 块的 by 局部）——把两块合并：直接用 yy
s = s.replace("""    { let bb = spv_rt_bench::rt_bench_spv();
      match naga::front::spv::parse_u8_slice(&by, &naga::front::spv::Options::default()) {
        Ok(_) => println!("cargo:warning=RT_SPV_PARSE_OK"),
        Err(e) => println!("cargo:warning=RT_SPV_PARSE_ERR: {}", e.to_string().chars().take(300).collect::<String>()),
      } }""", """    { let bb = spv_rt_bench::rt_bench_spv(); let mut yy: Vec<u8> = Vec::with_capacity(bb.len()*4); for w in &bb { yy.extend_from_slice(&w.to_le_bytes()); }
      match naga::front::spv::parse_u8_slice(&yy, &naga::front::spv::Options::default()) {
        Ok(_) => println!("cargo:warning=RT_SPV_PARSE_OK"),
        Err(e) => println!("cargo:warning=RT_SPV_PARSE_ERR: {}", e.to_string().chars().take(300).collect::<String>()),
      } }""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('u8 slice fixed')
