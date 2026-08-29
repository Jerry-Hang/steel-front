# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = s.replace(".create_as_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, count as u64)", ".create_device_local_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; count as usize], \"pt-as\")")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('asbuf via device_local')
