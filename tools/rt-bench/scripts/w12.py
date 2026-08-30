# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
# 2) instance: rt_test 签名加 instance &ash::Instance；vk_asext 用 entry
s2 = s.replace("pub fn rt_test(dev: &ash::Device, queue: &vk::Queue, mem_props: vk::PhysicalDeviceMemoryProperties) -> Result<f64, String> {",
"pub fn rt_test(dev: &ash::Device, queue: &vk::Queue, instance: &ash::Instance, mem_props: vk::PhysicalDeviceMemoryProperties) -> Result<f64, String> {")
s2 = s2.replace("let vk_asext = ash::khr::acceleration_structure::Device::new(&ash::Instance::default(), dev);", "let vk_asext = ash::khr::acceleration_structure::Device::new(instance, dev);")
# 3) mem_props 在闭包中引用问题：闭包捕获 mem_props（Copy 类型 ok）
s = s2
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('instance param fixed')
# main.rs 调用
p2 = 'src/main.rs'
s3 = io.open(p2, encoding='utf-8').read()
s3 = s3.replace("let rt_val = rt_test(&device, &queue, unsafe { device.get_physical_device_memory_properties(phys) }).unwrap_or(0.0);", "let rt_val = rt_test(&device, &queue, &entry, unsafe { entry.get_physical_device_memory_properties(phys) }).unwrap_or(0.0);")
io.open(p2, 'w', encoding='utf-8', newline='').write(s3)
print('main call fixed 2')
