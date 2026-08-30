# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("fn create_device(entry: &ash::Instance, phys: vk::PhysicalDevice) -> Result<(ash::Device, vk::Queue), String> {\n    unsafe {", "fn create_device(entry: &ash::Instance, phys: vk::PhysicalDevice) -> Result<(ash::Device, vk::Queue), String> {\n    let prio = [1.0f32];\n    unsafe {")
s = s.replace("        let prio = [1.0f32];\n    unsafe {\n        let ext_names = [", "    unsafe {\n        let ext_names = [")
# 230 段：把整段 civil_from_days 的重叠修复（u64 → i64 一致化）
old_civil = """    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };"""
new_civil = """    let doye = (z - era * 146097).rem_euclid(146097);
    let yoe = (doye - doye / 1460 + doye / 36524 - doye / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doye - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };"""
if old_civil in s:
    s = s.replace(old_civil, new_civil, 1)
    print('civil fixed')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
