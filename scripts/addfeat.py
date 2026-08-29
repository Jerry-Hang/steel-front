# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# 找 device_create_info 的构建（mesh 的 pNext 挂载点）
anchor = """        let device_create_info = vk::DeviceCreateInfo::default()"""
if anchor in s:
    # 在 device_create_info 定义前后加特性结构体——先看上下文（1102-1125 区域）
    s = s.replace(anchor, """        // 2026-08-29：RT 特性链（rayQuery + accelerationStructure features——扩展启用 ≠ 特性启用！）
        let mut rq_features = vk::PhysicalDeviceRayQueryFeaturesKHR::default();
        rq_features.ray_query = vk::TRUE;
        let mut as_features = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
        as_features.acceleration_structure = vk::TRUE;
        as_features.buffer_device_address = vk::TRUE;
        // 链到 mesh 特性（若无 mesh 则直接挂在 device_create_info.pNext）
""" + anchor, 1)
    # 将特性链挂到 device_create_info（在 ...之后）
    s = s.replace("""            .enabled_extension_names(&device_extensions)""", """            .enabled_extension_names(&device_extensions)""")
    # 找 device_create_info 的 pNext 处理（mesh 特性挂那个字段）
    idx = s.find("let device_create_info = vk::DeviceCreateInfo::default()")
    seg = s[idx:idx+120]
    print('DCI context:', seg.replace('\n',' | ')[:200])
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('feat structs added')
