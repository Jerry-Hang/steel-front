# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("let mem = dev.allocate_memory(&ai, None)).map_err(|e| format!(\"{e:?}\"))?;", "let mem = dev.allocate_memory(&ai, None).map_err(|e| format!(\"{e:?}\"))?;")
s = s.replace("dev.bind_buffer_memory(buf, mem, 0)).map_err(|e| format!(\"{e:?}\"))?;", "dev.bind_buffer_memory(buf, mem, 0).map_err(|e| format!(\"{e:?}\"))?;")
s = s.replace("let vp = dev.map_memory(vmem, 0, verts.len() as u64, vk::MemoryMapFlags::empty())).map_err(|e| format!(\"{e:?}\"))?;", "let vp = dev.map_memory(vmem, 0, verts.len() as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!(\"{e:?}\"))?;")
s = s.replace("let ip = dev.map_memory(imem, 0, idxs.len() as u64 * 4, vk::MemoryMapFlags::empty())).map_err(|e| format!(\"{e:?}\"))?;", "let ip = dev.map_memory(imem, 0, idxs.len() as u64 * 4, vk::MemoryMapFlags::empty()).map_err(|e| format!(\"{e:?}\"))?;")
s = s.replace("dev.destroy_buffer(b, None);", "dev.destroy_buffer(b, None);")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('bad parens fixed')
