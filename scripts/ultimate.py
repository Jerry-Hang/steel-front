# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# Vertex：{pos, uv, color}
s = s.replace("""struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
    uv: [f32; 2],
}""", """struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 3],
}""")
# 转换 {pos, uv, color}
s = s.replace("""                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    color: v.color,
                    uv: v.uv,
                };""", """                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    uv: v.uv,
                    color: v.color,
                };""")
# attrs loc1 = offset_of!(Vertex, color)（=24）loc2 = offset_of!(Vertex, uv)（=12）——顺序本就按字段名
# （offset_of 自动给出 color=24 / uv=12 —— loc1 已绑定 Vertex.color 字段 ✓）
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ultimate layout {pos,uv,color} with offset_of attrs')
