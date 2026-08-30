# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("fn pick_gpu(entry: &ash::Entry) -> Option<(vk::PhysicalDevice, vk::PhysicalDeviceProperties)> {", "fn pick_gpu(entry: &ash::Instance) -> Option<(vk::PhysicalDevice, vk::PhysicalDeviceProperties)> {")
# 调用处先 pick 后 create_instance——顺序调换：先 inst 再 phys
s = s.replace("""    let (phys, props) = pick_gpu(&entry).expect("未找到 GPU");""", """    let inst = create_instance(&entry).expect("Instance 创建失败");
    let (phys, props) = pick_gpu(&inst).expect("未找到 GPU");""")
s = s.replace("""    let inst = create_instance(&entry).expect("Instance 创建失败");
    let (device, queue) = create_device(&inst, phys).expect("设备创建失败");""", """    let (device, queue) = create_device(&inst, phys).expect("设备创建失败");""")
# 把 create_device 中的 entry 类型参数 &ash::Instance ✓ 已改
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('pick gpu fixed')
