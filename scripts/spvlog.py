# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
anchor = '        let vs_spirv = load_spirv("assets/triangle.vert.spv")?;'
if anchor in s:
    s = s.replace(anchor, anchor + '\n        log::info!("shader: triangle.vert.spv 字节数 {}", vs_spirv.len());', 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('spv size log installed')
else:
    print('anchor missing')
