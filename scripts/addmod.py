# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
if 'mod shaders' in s:
    print('already')
else:
    # 在 crate 第一个 mod 附近插入（找首个 'mod ' 声明）
    mark = "#![cfg(windows)]\n"
    if mark in s:
        s = s.replace(mark, mark + "\n/// 构建期内嵌着色器（build.rs 生成 OUT_DIR/shaders.rs）\npub mod shaders { include!(concat!(env!(\"OUT_DIR\"), \"/shaders.rs\")); }\n", 1)
        io.open(p, 'w', encoding='utf-8', newline='').write(s)
        print('shaders module added')
    else:
        print('mark missing')
