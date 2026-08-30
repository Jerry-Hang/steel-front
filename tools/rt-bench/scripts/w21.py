# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
# 306: end_command_buffer(cmd, None) → (cmd)
s = s.replace("dev.end_command_buffer(cmd, None);", "dev.end_command_buffer(cmd);")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('end fixed')
p2 = 'src/main.rs'
s2 = io.open(p2, encoding='utf-8').read()
# 创建真 instance
old = """    let (device, queue) = create_device(&entry, phys).expect("设备创建失败");"""
new = """    let inst = create_instance(&entry).expect("Instance 创建失败");
    let (device, queue) = create_device(&inst, phys).expect("设备创建失败");"""
if old in s2:
    s2 = s2.replace(old, new, 1)
    print('instance wired')
# create_instance 函数 + create_device 签名改 instance
fn_add = """
fn create_instance(entry: &ash::Entry) -> Result<ash::Instance, String> {
    unsafe {
        let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3);
        let ci = vk::InstanceCreateInfo::default().application_info(&app);
        entry.create_instance(&ci, None).map_err(|e| format!("instance: {:?}", e))
    }
}
"""
if 'fn create_instance' not in s2:
    s2 = s2.replace("fn pick_gpu", fn_add + "\nfn pick_gpu", 1)
s2 = s2.replace("fn create_device(entry: &ash::Entry, phys: vk::PhysicalDevice)", "fn create_device(entry: &ash::Instance, phys: vk::PhysicalDevice)")
# rt_test 调用（用 inst）
s2 = s2.replace("let rt_val = rt_test(&device, &queue, &entry, unsafe { entry.get_physical_device_memory_properties(phys) }).unwrap_or(0.0);", "let rt_val = rt_test(&device, &queue, &inst, unsafe { inst.get_physical_device_memory_properties(phys) }).unwrap_or(0.0);")
io.open(p2, 'w', encoding='utf-8', newline='').write(s2)
print('main instance wired')
