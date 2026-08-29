# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""      let by: Vec<u32> = bb;
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
    }""", """      let mut module = rspirv::dr::Module::default();
      match rspirv::binary::parse_words(&bb, &mut module) {
        Ok(_) => {
          match rspirv::dr::validate_module(&module) {
            Ok(_) => println!("cargo:warning=RT_SPV_VALID_OK"),
            Err(e) => println!("cargo:warning=RT_SPV_VALID_ERR: {}", e),
          }
        }
        Err(e) => println!("cargo:warning=RT_SPV_PARSE_ERR: {}", e),
      }
    }""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('parse_words via Consumer')
