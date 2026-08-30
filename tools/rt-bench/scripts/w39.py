# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
# fp_test: 创建 buf 后写随机初值（host map）
old = """        let buf = create_buffer(dev, phys, size, vk::BufferUsageFlags::STORAGE_BUFFER)?;
        let mem = alloc_mem(dev, inst, phys, buf, 0)?;"""
new = """        let buf = create_buffer(dev, phys, size, vk::BufferUsageFlags::STORAGE_BUFFER)?;
        let mem = alloc_mem(dev, inst, phys, buf, 0)?;
        // 随机初值 → 防循环常量折叠（结果必须依赖输入！）
        if let Ok(p) = dev.map_memory(mem, 0, size, vk::MemoryMapFlags::empty()) {
            let mut seed = 0x9E3779B9u32;
            let n = size as usize / 4;
            let arr = std::slice::from_raw_parts_mut(p as *mut u32, n);
            for v in arr.iter_mut() { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); *v = seed; }
            dev.unmap_memory(mem);
        }"""
if old in s:
    s = s.replace(old, new, 1)
    print('fp random init')
# alloc_mem 用 host-visible（需要 map！）——alloc_mem 是 device-local 现在！改为 host-visible
s = s.replace("if req.memory_type_bits & (1 << i) != 0 && t.property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL) {", "if req.memory_type_bits & (1 << i) != 0 && t.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT) {")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fp_test mapped hostbuf')
