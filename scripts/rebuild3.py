# -*- coding: utf-8 -*-
import io, sys
s = io.open('screenshots/README_old.md', encoding='utf-8').read()
old_row = '| 1080P 最低可玩 | 6 核（i5-10400 / R5 3600 级） | RX 6500 XT / A380 级 | 8GB | 传统顶点管线回退；波次流畅，128v128 掉帧 |'
new_row = '| 1080P 最低可玩 | **4核8线程（Ryzen 3 3300X / Intel Core i3-11100，11 代 i3 级）** | RX 6500 XT / A380 级（ray-query 型 RTX 2060+ 可开全景 PT） | 8GB | 传统顶点管线回退；波次流畅，128v128 掉帧 |'
if old_row in s:
    s = s.replace(old_row, new_row, 1)
else:
    sys.stderr.write('ROW MISS\n')
min_sec = """## 最低配置（2026-09-01 更新）

> 承接下表「1080P 最低可玩」档，2026-09-01 统一修订为以下基线：

| 部件 | 最低要求 | 说明 |
|---|---|---|
| **CPU** | **AMD Ryzen 3 3300X**（4 核 8 线程）或 **Intel Core i3-11100 / i3-1115G4（11 代 i3 级）** | 128v128 大战场需要 4 核+（AI 分池 8 线程更佳） |
| 内存 | 16 GB | 8GB 可玩波次模式 |
| 显卡 | Vulkan 1.3 驱动；**开全景路径追踪需支持 ray-query**（NVIDIA RTX 20/30/40/50 系、AMD RX 6000+ 独显）；无 RT 自动回退传统光栅 | PT 全景 2560×1600 实测显存 ≈ 3 GB |
| 显存 | 8 GB | PT 渲染 + 光栅共存 |
| 存储 | ~2 GB（含 glb 缓存） | SSD 更佳 |
| 系统 | Windows 10 2004+ / Windows 11 | 需 Vulkan 1.3 |

---

"""
if '## 硬件推荐配置' in s:
    s = s.replace('## 硬件推荐配置', min_sec + '## 硬件推荐配置', 1)
else:
    sys.stderr.write('HW MISS\n')
io.open('screenshots/final_readme.md', 'w', encoding='utf-8', newline='\n').write(s)
sys.stderr.write('DONE len=%d\n' % len(s))
