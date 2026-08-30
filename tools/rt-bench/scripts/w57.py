# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
# ① 顶部加 RtResult
s = s.replace("use ash::vk;\nuse std::time::Instant;", """use ash::vk;
use std::time::Instant;

pub struct RtResult {
    pub best_mrays: f64,
    pub median_mrays: f64,
    pub total_hits: u64,
    pub rounds_mrays: Vec<f64>,
}""")
# ② rt_test 签名改为带 rays/iters/rounds
s = s.replace("pub fn rt_test(\n    dev: &ash::Device,\n    queue: &vk::Queue,\n    instance: &ash::Instance,\n    phys: vk::PhysicalDevice,\n) -> Result<f64, String> {",
"""pub fn rt_test(
    dev: &ash::Device,
    queue: &vk::Queue,
    instance: &ash::Instance,
    phys: vk::PhysicalDevice,
    rays: u32,
    iters: u32,
    rounds: u32,
) -> Result<RtResult, String> {""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('sig updated')
