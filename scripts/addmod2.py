# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("mod engine;", "/// 构建期内嵌着色器（build.rs 生成 OUT_DIR/shaders.rs）\npub mod shaders {\n    include!(concat!(env!(\"OUT_DIR\"), \"/shaders.rs\"));\n}\n\nmod engine;", 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('module inserted:', 'pub mod shaders' in s)
