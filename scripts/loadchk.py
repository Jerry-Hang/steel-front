# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
old = """      let mut module = rspirv::dr::Module::default();
      match rspirv::binary::parse_words(&bb, &mut module) {
        Ok(_) => {
          match rspirv::dr::validate_module(&module) {
            Ok(_) => println!("cargo:warning=RT_SPV_VALID_OK"),
            Err(e) => println!("cargo:warning=RT_SPV_VALID_ERR: {}", e),
          }
        }
        Err(e) => println!("cargo:warning=RT_SPV_PARSE_ERR: {}", e),
      }
    }"""
new = """      let mut loader = rspirv::dr::Loader::new();
      match rspirv::binary::parse_words(&bb, &mut loader) {
        Ok(_) => {
          let module = loader.module();
          println!(
            "cargo:warning=RT_SPV_STRUCT_OK ids={} ops={}",
            module.types_global_values.len(),
            module.functions.iter().map(|f| f.instructions.len()).sum::<usize>() + module.types_global_values.len()
          );
        }
        Err(e) => println!("cargo:warning=RT_SPV_STRUCT_ERR: {}", e),
      }
    }"""
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('loader structural check wired')
else:
    print('anchor missing')
