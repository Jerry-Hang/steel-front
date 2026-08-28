# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, uv) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, color) as u32),""",
"""                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, color) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, uv) as u32),""")
s = s.replace("""                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    color: v.color,
                    uv: [v.color[0], v.color[1]],
                };""", """                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    color: v.color,
                    uv: v.uv,
                };""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('reverted to clean (color@12, uv@24)')
