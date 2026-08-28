# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# 在 attrs 创建后打印（找 vertex_input_state 前）
anchor = "        let vertex_bindings = [vertex_binding];"
if anchor in s:
    s = s.replace(anchor, """        log::info!(
            "gun-attr: stride={} pos@{} color@{} uv@{}",
            std::mem::size_of::<Vertex>(),
            std::mem::offset_of!(Vertex, pos),
            std::mem::offset_of!(Vertex, color),
            std::mem::offset_of!(Vertex, uv)
        );
""" + anchor, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('attr log installed')
else:
    print('anchor missing')
