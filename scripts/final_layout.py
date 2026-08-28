# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# Vertex 结构：{pos, color, uv}
s = s.replace("""struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 3],
}""", """struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
    uv: [f32; 2],
}""")
# attrs：loc1=color@12, loc2=uv@24（offset_of 代管）
s = s.replace("""                .offset(std::mem::offset_of!(Vertex, pos) as u32),""", """                .offset(std::mem::offset_of!(Vertex, pos) as u32),""")
# 转换：{pos, color, uv}
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
print('layout {pos,color,uv} restored; attrs follow offset_of (color@12,uv@24)')
