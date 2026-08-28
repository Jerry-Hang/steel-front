# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""                .offset(std::mem::offset_of!(Vertex, color) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, uv) as u32),""",
"""                .offset(std::mem::offset_of!(Vertex, uv) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, color) as u32),""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('swapped' if 'Vertex, uv) as u32),\n            // location 2' in s else 'FAIL')
