# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
old = """        if let Some(first) = verts.first() {
            log::info!("gun: 写入顶点首色 {:?}", first.color);
        }"""
new = """        if let Some(first) = verts.first() {
            let vp = self.gun_mapped as *const Vertex;
            log::info!(
                "gun: 写入首色 {:?} / 映射@color {:?} @uv {:?}",
                first.color,
                unsafe { (*vp).color },
                unsafe { (*vp).uv }
            );
        }"""
if old not in s:
    print('anchor missing')
else:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('readback installed')
