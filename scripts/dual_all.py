# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    color: v.color,
                    uv: v.uv,
                };""", """                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    color: v.color,
                    uv: [v.color[0], v.color[1]],
                };""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('dual-write all-slot')
