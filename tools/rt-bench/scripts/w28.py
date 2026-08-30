# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
# 104: queues 数组借用 prio —— 把 prio 提到函数级（在 unsafe 块外）
s = s.replace("        let prio = [1.0f32];", "        let prio = [1.0f32];\n        let _ = &prio;")
# fp_test/alloc_mem 签名加 mem_props；调用处
s = s.replace("fn fp_test(dev: &ash::Device, queue: &vk::Queue, phys: vk::PhysicalDevice, spv: &[u8], items: u32) -> Result<f64, String> {", "fn fp_test(dev: &ash::Device, queue: &vk::Queue, phys: vk::PhysicalDevice, inst: &ash::Instance, spv: &[u8], items: u32) -> Result<f64, String> {")
s = s.replace("let buf = create_buffer(dev, phys, size, vk::BufferUsageFlags::STORAGE_BUFFER)?;", "let buf = create_buffer(dev, phys, size, vk::BufferUsageFlags::STORAGE_BUFFER)?;")
s = s.replace("let mem = alloc_mem(dev, phys, buf, std::mem::size_of::<u64>() as u64, 0)?;", "let mem = alloc_mem(dev, inst, buf, 0)?;")
s = s.replace("fn alloc_mem(dev: &ash::Device, phys: vk::PhysicalDevice, buf: vk::Buffer, alignment: u64, _extra: u32) -> Result<vk::DeviceMemory, String> {", "fn alloc_mem(dev: &ash::Device, inst: &ash::Instance, buf: vk::Buffer, alignment: u64) -> Result<vk::DeviceMemory, String> {")
s = s.replace("let mprops = dev.get_physical_device_memory_properties(phys);", "let mprops = inst.get_physical_device_memory_properties(phys);")
# fp_test 调用处（加 inst）
s = s.replace("match fp_test(&device, &queue, phys, spv, 1 << 18) {", "match fp_test(&device, &queue, phys, &inst, spv, 1 << 18) {")
# alloc_mem 中 alignment 未用则加 let _
s = s.replace("let req = dev.get_buffer_memory_requirements(buf);", "let _align = alignment;\n        let req = dev.get_buffer_memory_requirements(buf);")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fp_test signatures fixed')
