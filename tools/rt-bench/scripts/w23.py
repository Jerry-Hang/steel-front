# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("let mut rq_f = vk::PhysicalDeviceRayQueryFeaturesKHR::default();\n        rq_f.ray_query = true;", "let mut rq_f = vk::PhysicalDeviceRayQueryFeaturesKHR::default();\n        rq_f.ray_query = 1;")
s = s.replace("let mut as_f = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();\n        as_f.acceleration_structure = true;", "let mut as_f = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();\n        as_f.acceleration_structure = 1;")
s = s.replace("let mut bd_f = vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR::default();\n        bd_f.buffer_device_address = true;", "let mut bd_f = vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR::default();\n        bd_f.buffer_device_address = 1;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('u32 fixed')
p2 = 'src/rt_impl.rs'
s2 = io.open(p2, encoding='utf-8').read()
s2 = s2.replace("w_as.p_next = &accel_write as *const _ as *const vk::BaseOutStructure;", "w_as.p_next = &accel_write as *const _ as *const std::ffi::c_void;")
# 174: bind_buffer_memory 的闭包问题——修（map_err 对 vk::Result ok，但"？"不转换说明该行在 return Result<_, String> 但 map_err 后 ? 应 ok——真实错误可能是闭包 return type！把 mem_alloc 改为函数而非闭包！）
io.open(p2, 'w', encoding='utf-8', newline='').write(s2)
print('pnext fixed')
