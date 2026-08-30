# -*- coding: utf-8 -*-
import io
# rt_impl: use Instant
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("use ash::vk;", "use ash::vk;\nuse std::time::Instant;")
# fn rt_test -> pub fn
s = s.replace("fn rt_test(dev: &ash::Device", "pub fn rt_test(dev: &ash::Device")
# 批量 ? → map_err（针对 create_buffer/map_memory/allocate 等失败?）
# 简单方案：外层大 try? 用 String——逐个替换常见
import re
for pat, rep in [
  ("dev.create_buffer(&vk::BufferCreateInfo::default().size(verts.len() as u64)", "dev.create_buffer(&vk::BufferCreateInfo::default().size(verts.len() as u64)"),
]:
    s = s.replace(pat, rep)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('rt_impl fixed')
# main.rs: rt_test pub
p2 = 'src/main.rs'
s2 = io.open(p2, encoding='utf-8').read()
s2 = s2.replace("mod rt_impl;", "mod rt_impl;\nuse rt_impl::rt_test;")
s2 = s2.replace("fn rt_test(dev: &ash::Device, queue: &vk::Queue, phys: vk::PhysicalDevice, _entry: &ash::Entry) -> Result<f64, String> {\n    // 复用主项目已验证 RT spv（1M 射线 x 200 迭代，递归压测）\n    let spv = include_bytes!(\"../pt_ref3.spv\");\n    let _ = spv;\n    Ok(0.0) // 占位：完整 RT 压测在下一步接入（扩展现有 AS/管线）\n}\n", "")
io.open(p2, 'w', encoding='utf-8', newline='').write(s2)
print('main fixed')
