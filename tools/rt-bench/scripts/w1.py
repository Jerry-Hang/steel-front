# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
# 删掉旧占位 rt_test
old_rt = """// ---------- RT 压测 ----------
fn rt_test(dev: &ash::Device, queue: &vk::Queue, phys: vk::PhysicalDevice, entry: &ash::Entry) -> Result<f64, String> {
    // 复用主项目已验证 RT spv（1M 射线 x 200 迭代，递归压测）
    let spv = include_bytes!("../pt_ref3.spv");
    let _ = spv;
    Ok(0.0) // 占位：完整 RT 压测在下一步接入（扩展现有 AS/管线）
}
"""
if old_rt in s:
    s = s.replace(old_rt, "", 1)
    print('old rt removed')
# 模块引用
s = s.replace("use std::time::Instant;", "use std::time::Instant;\nmod rt_impl;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('module wired')
