# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = s.replace("geom.p_geometries = [&geo].as_ptr();", "geom.p_geometries = &geo;")
s = s.replace(".create_device_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, count as u64)", ".create_as_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, count as u64)")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('fix p+asbuf')
