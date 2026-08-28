# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
old_vs = """    if (instance_index >= EMISSIVE_INSTANCE_BASE) {
        // 自发光实体：fade > 1 作为 emissive 信号（片元直出颜色，跳过光照/贴图混合）
        output.flat_flag = 1.0;
        output.fade = 2.0;"""
new_vs = """    // 自发光区间（EMISSIVE_BASE .. +64）；枪槽（+64 后一槽）在此区间之外（2026-08-27 修复：
    // 原 >= 使枪槽被当作自发光 → flag=1+fade=2 → 走光照路径被太阳光×7.7 刷成纯白）
    if (instance_index >= EMISSIVE_INSTANCE_BASE && instance_index < EMISSIVE_INSTANCE_BASE + 64u) {
        // 自发光实体：fade > 1 作为 emissive 信号（片元直出颜色，跳过光照/贴图混合）
        output.flat_flag = 1.0;
        output.fade = 2.0;"""
if old_vs not in s:
    print('VS ANCHOR NOT FOUND')
else:
    s = s.replace(old_vs, new_vs, 1)
    print('vs fixed')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
