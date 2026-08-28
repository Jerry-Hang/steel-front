# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 3],
}""", """struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
    uv: [f32; 2],
}""")
s = s.replace("""                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    uv: v.uv,
                    color: v.color,
                };""", """                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    color: v.color,
                    uv: v.uv,
                };""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('reverted to {pos,color,uv} (offset_of attrs auto: color@12/uv@24)')
