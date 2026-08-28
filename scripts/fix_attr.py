# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
old = """                .offset(std::mem::size_of::<[f32; 3]>() as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset((std::mem::size_of::<[f32; 3]>() * 2) as u32),"""
new = """                .offset(std::mem::offset_of!(Vertex, color) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, uv) as u32),"""
if old not in s:
    print('anchor miss')
else:
    s = s.replace(old, new, 1)
    # loc0 也改
    s2 = s.replace("""                .offset(0),
            // location 1: color vec3""", """                .offset(std::mem::offset_of!(Vertex, pos) as u32),
            // location 1: color vec3""", 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s2)
    print('offset_of applied' if s2 != s else 'loc0 ok, loc1 applied')
