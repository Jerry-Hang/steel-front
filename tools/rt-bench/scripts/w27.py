# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("dev.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&[dsl]), None).map_err(|e| format!(\"ds: {:?}\", e))?[0];", "dev.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&[dsl])).map_err(|e| format!(\"ds: {:?}\", e))?[0];")
s = s.replace("let cmd = dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(cpool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1), None).map_err(|e| format!(\"cb: {:?}\", e))?[0];", "let cmd = dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(cpool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1)).map_err(|e| format!(\"cb: {:?}\", e))?[0];")
s = s.replace("dev.begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default(), None).map_err(|e| format!(\"begin: {:?}\", e))?;", "dev.begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default()).map_err(|e| format!(\"begin: {:?}\", e))?;")
s = s.replace("dev.end_command_buffer(cmd, None);", "dev.end_command_buffer(cmd);")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('main sigs fixed')
