# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""        Ok(_) => {
          let module = loader.module();
          println!(
            "cargo:warning=RT_SPV_STRUCT_OK ids={} ops={}",
            module.types_global_values.len(),
            module.functions.iter().map(|f| f.instructions.len()).sum::<usize>() + module.types_global_values.len()
          );
        }""", """        Ok(_) => println!("cargo:warning=RT_SPV_STRUCT_OK"),""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('simplified')
