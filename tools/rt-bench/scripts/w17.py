# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("let vp = dev.map_memory(vmem, 0, verts.len() as u64, vk::MemoryMapFlags::empty()?;", "let vp = dev.map_memory(vmem, 0, verts.len() as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!(\"{e:?}\"))?;")
s = s.replace("let ip = dev.map_memory(imem, 0, idxs.len() as u64 * 4, vk::MemoryMapFlags::empty()?;", "let ip = dev.map_memory(imem, 0, idxs.len() as u64 * 4, vk::MemoryMapFlags::empty()).map_err(|e| format!(\"{e:?}\"))?;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fixed again')
