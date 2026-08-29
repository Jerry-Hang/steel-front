# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""      match rspirv::binary::parse_bytes(&bb.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>() /* 需要 u32 解析 */) {
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
    }""", """      let by: Vec<u32> = bb;
      // rspirv 解析（u32 切片）+ 校验
      let mut mi = rspirv::binary::Parser::new();
      let mut module = rspirv::dr::Module::default();
      match mi.parse(&by, &mut module) {
        Ok(_) => {
          match module.validate() {
            Ok(_) => println!("cargo:warning=RT_SPV_VALID_OK"),
            Err(e) => println!("cargo:warning=RT_SPV_VALID_ERR: {}", e),
          }
        }
        Err(e) => println!("cargo:warning=RT_SPV_PARSE_ERR: {}", e),
      }
    }""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('rspirv api fixed')
