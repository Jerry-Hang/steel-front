# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("dq.p_queue_priorities = &prio;", "dq.p_queue_priorities = prio.as_ptr();")
# ext_ptrs enabled_extension_names 期望 &[*const c_char]? ash 0.38 DeviceCreateInfo::enabled_extension_names(&[*const i8]) 需要 &[*const u8]… 报错即转换
s = s.replace("let ext_ptrs: Vec<*const u8> = ext_names.iter().map(|e| e.as_ptr() as *const u8).collect();", "let ext_ptrs: Vec<*const std::ffi::c_char> = ext_names.iter().map(|e| e.as_ptr() as *const std::ffi::c_char).collect();")
s = s.replace("alloc.set_layouts(&[dsl]), None)?[0];", "alloc.set_layouts(&[dsl])).map_err(|e| format!(\"{e:?}\"))?[0];")
s = s.replace("let cmd = dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(cpool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1), None).map_err(|e| format!(\"{e:?}\"))?[0];", "let cmd = dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(cpool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1)).map_err(|e| format!(\"{e:?}\"))?[0];")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('main ash fixes')
