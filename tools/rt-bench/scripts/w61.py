# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
old = """        let hits_items: u32 = rays;
        let hbuf = create_buf(dev, hits_items as u64 * 4, vk::BufferUsageFlags::STORAGE_BUFFER)?;
        let (hmem, _) = mem_alloc_ex(dev, instance, phys, hbuf, hits_items as u64 * 4, false)?;"""
new = """        let hits_items: u32 = rays;
        let hbuf = create_buf(dev, hits_items as u64 * 4, vk::BufferUsageFlags::STORAGE_BUFFER)?;
        let (hmem, _) = mem_alloc_ex(dev, instance, phys, hbuf, hits_items as u64 * 4, true)?; // host-visible 回读！"""
if old in s:
    s = s.replace(old, new, 1)
    print('hits host-visible')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
