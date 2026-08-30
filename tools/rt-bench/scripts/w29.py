# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("fn alloc_mem(dev: &ash::Device, inst: &ash::Instance, buf: vk::Buffer, alignment: u64) -> Result<vk::DeviceMemory, String> {", "fn alloc_mem(dev: &ash::Device, inst: &ash::Instance, phys: vk::PhysicalDevice, buf: vk::Buffer, alignment: u64) -> Result<vk::DeviceMemory, String> {")
s = s.replace("let mem = alloc_mem(dev, inst, buf, 0)?;", "let mem = alloc_mem(dev, inst, phys, buf, 0)?;")
# 231: update_descriptor_sets 的 WriteDescriptorSet 手工构造（buffer_info builder 可能不存在）
s = s.replace("dev.update_descriptor_sets(&[vk::WriteDescriptorSet::default().dst_set(ds).dst_binding(0).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).buffer_info(&[bai])], &[]);", "let mut w = vk::WriteDescriptorSet::default();\n        w.dst_set = ds;\n        w.dst_binding = 0;\n        w.descriptor_type = vk::DescriptorType::STORAGE_BUFFER;\n        w.descriptor_count = 1;\n        w.p_buffer_info = &bai;\n        dev.update_descriptor_sets(&[w], &[]);")
# 105: prio 借用——将 prio 放 unsafe 外的函数体
s = s.replace("let prio = [1.0f32];\n        let _ = &prio;", "let prio = [1.0f32];")
s = s.replace("unsafe {\n        let ext_names = [", "let prio = [1.0f32];\n    unsafe {\n        let ext_names = [")
s = s.replace("let queues = [dq];", "let queues = [dq];\n        let _ = &prio;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fixed')
