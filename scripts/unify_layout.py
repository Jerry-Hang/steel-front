# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# 1) Vertex 结构：{pos, uv, color}
s = s.replace("""struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
    uv: [f32; 2],
}""", """struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 3],
}""")
# 2) attr：pos@0, color@offset(color)=24, uv@offset(uv)=12
s = s.replace("""                .offset(std::mem::offset_of!(Vertex, uv) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, color) as u32),""",
"""                .offset(std::mem::offset_of!(Vertex, color) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, uv) as u32),""")
# 3) 转换写 {pos, uv, color}
s = s.replace("""                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    color: v.color,
                    uv: v.uv,
                };""",
"""                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    uv: v.uv,
                    color: v.color,
                };""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('unified old layout:', 'uv: v.uv,\n                    color: v.color' in s)
