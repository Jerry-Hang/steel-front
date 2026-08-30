# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
# 1) vbuf/ibuf 创建 + mem_alloc 闭包 → helper
old1 = """        let vbuf = dev.create_buffer(&vk::BufferCreateInfo::default().size(verts.len() as u64).usage(vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR).sharing_mode(vk::SharingMode::EXCLUSIVE), None)?;"""
new1 = """        let vbuf = create_buf(dev, verts.len() as u64, vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR)?;"""
s = s.replace(old1, new1, 1)
print('vbuf', old1 in s.replace(old1, new1, 1))

# 2) ibuf
old2 = """        let ibuf = dev.create_buffer(&vk::BufferCreateInfo::default().size(idxs.len() as u64 * 4).usage(vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR).sharing_mode(vk::SharingMode::EXCLUSIVE), None)?;"""
new2 = """        let ibuf = create_buf(dev, idxs.len() as u64 * 4, vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR)?;"""
s = s.replace(old2, new2, 1)
print('ibuf done')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
