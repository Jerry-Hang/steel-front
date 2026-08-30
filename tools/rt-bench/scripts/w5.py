# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
# 1) 签名带 mem_props
s = s.replace("pub fn rt_test(dev: &ash::Device, queue: &vk::Queue, phys: vk::PhysicalDevice, _entry: &ash::Entry) -> Result<f64, String> {",
"pub fn rt_test(dev: &ash::Device, queue: &vk::Queue, mem_props: vk::PhysicalDeviceMemoryProperties) -> Result<f64, String> {")
# 2) mprops 引用
s = s.replace("let mprops = dev.get_physical_device_memory_properties(phys);", "let mprops = mem_props;")
s = s.replace("let mprops = dev.get_physical_device_memory_properties(phys);", "let mprops = mem_props;")
s = s.replace("let mprops = dev.get_physical_device_memory_properties(phys);", "let mprops = mem_props;")
s = s.replace("let mprops = dev.get_physical_device_memory_properties(phys);", "let mprops = mem_props;")
s = s.replace("let mprops = dev.get_physical_device_memory_properties(phys);", "let mprops = mem_props;")
# 3) 所有 dev.xxx? 自动 map_err —— 用 helper 函数接收 vk::Device + Result 转换：手动批量替换常见模式
import re
def add_me(m):
    return m.group(0)[:-1] + ").map_err(|e| format!(\"{:?}\", e))?"
# 逐行处理 create_buffer/allocate_memory/bind_buffer_memory/map_memory/create_shader_module 等
lines = s.split('\n')
out = []
for ln in lines:
    if ln.rstrip().endswith('?;') or ln.rstrip().endswith('?):'):
        # 常见 ash 调用
        if any(k in ln for k in ['create_buffer(', 'allocate_memory(', 'bind_buffer_memory(', 'map_memory(', 'create_shader_module(', 'create_descriptor_', 'create_pipeline_layout(', 'create_compute_pipelines(', 'create_command_pool(', 'allocate_command_buffers(', 'allocate_descriptor_sets(', 'queue_submit(', 'queue_wait_idle(', 'create_pipeline(', 'create_acceleration_structure(', 'begin_command_buffer(', 'end_command_buffer(']):
            # 简单替换：在结尾 ? 前加 map_err
            ln2 = ln.replace('?;', ').map_err(|e| format!("{e:?}"))?;').replace('?):', ').map_err(|e| format!("{e:?}"))?):')
            out.append(ln2)
        else:
            out.append(ln)
    else:
        out.append(ln)
s = '\n'.join(out)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('rt_impl patched')
