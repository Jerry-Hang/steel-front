# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
anchor = """        // 索引上传（独立映射窗口，用一次性的暂存：直接再 map 索引内存）"""
add = """        // 2026-08-28 终极可见性修复：unmap → remap（host-coherent 亦可能被驱动缓存延迟可见）
        unsafe {
            self.device.unmap_memory(self.gun_vertex_buffer_memory);
            self.gun_mapped = self
                .device
                .map_memory(
                    self.gun_vertex_buffer_memory,
                    0,
                    self.gun_buffer_capacity_verts as u64 * std::mem::size_of::<Vertex>() as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .expect("枪模顶点缓冲重映射失败")
        }
""" + anchor
if anchor in s:
    s = s.replace(anchor, add, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('unmap-remap installed')
else:
    print('anchor missing')
