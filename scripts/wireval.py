# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
if 'rspirv' in s:
    print('already')
else:
    old = """    { let bb = spv_rt_bench::rt_bench_spv(); let mut yy: Vec<u8> = Vec::with_capacity(bb.len()*4); for w in &bb { yy.extend_from_slice(&w.to_le_bytes()); }
      match naga::front::spv::parse_u8_slice(&yy, &naga::front::spv::Options::default()) {
        Ok(_) => println!("cargo:warning=RT_SPV_PARSE_OK"),
        Err(e) => println!("cargo:warning=RT_SPV_PARSE_ERR: {}", e.to_string().chars().take(300).collect::<String>()),
      } }"""
    new = """    { let bb = spv_rt_bench::rt_bench_spv();
      match rspirv::binary::parse_bytes(&bb.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>() /* 需要 u32 解析 */) {
        Ok(_) => println!("cargo:warning=RT_SPV_PARSE_OK"),
        Err(e) => println!("cargo:warning=RT_SPV_PARSE_ERR: {}", e),
      }
      // 独立语义校验（rspirv validate）
      let loader = rspirv::dr::Loader::new();
      match rspirv::binary::parse_bytes(&{ let mut by = Vec::with_capacity(bb.len()*4); for w in &bb { by.extend_from_slice(&w.to_le_bytes()); } by }) {
        Ok(mut m) => {
          let mut module = rspirv::dr::Module::default();
          if let Err(e) = loader.consume_module(&mut m, &mut module) {
            println!("cargo:warning=RT_SPV_LOAD_ERR: {}", e);
          } else {
            match module.validate_with_limits(rspirv::dr::ValidationLimits::default()) {
              Ok(_) => println!("cargo:warning=RT_SPV_VALID_OK"),
              Err(e) => println!("cargo:warning=RT_SPV_VALID_ERR: {}", e),
            }
          }
        }
        Err(e) => println!("cargo:warning=RT_SPV_PARSE_ERR2: {}", e),
      }
    }"""
    if old in s:
        s = s.replace(old, new, 1)
        io.open(p, 'w', encoding='utf-8', newline='').write(s)
        print('rspirv validation wired')
    else:
        print('anchor missing')
