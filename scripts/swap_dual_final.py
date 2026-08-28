# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# descs：loc1=uv-offset(24), loc2=color-offset(12)——交换
old_desc = """                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, color) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, uv) as u32),"""
new_desc = """                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, uv) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, color) as u32),"""
if old_desc not in s:
    print('desc anchor missing')
else:
    s = s.replace(old_desc, new_desc, 1)
    # 双写
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
    print('swap+dual installed')
