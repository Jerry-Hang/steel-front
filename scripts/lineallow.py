# -*- coding: utf-8 -*-
import io, re

insertions = [
    ('src/engine/assets.rs', 24, '规划特性保留：OBJ 解析器（备用导入）'),
    ('src/engine/city.rs', 45, '规划特性保留：浮雕步长常量（备用）'),
    ('src/engine/city.rs', 164, '规划特性保留：二十面体生成器（备用）'),
    ('src/engine/city.rs', 206, '规划特性保留：板式建筑生成器（备用）'),
    ('src/engine/city.rs', 795, '规划特性保留：停车场地块（备用）'),
    ('src/engine/game.rs', 1660, '规划特性保留：障碍/速度查询 API（备用）'),
    ('src/engine/geom.rs', 62, '规划特性保留：形状标签解析（备用）'),
    ('src/engine/procedural.rs', 414, '规划特性保留：地面细节尺寸（备用）'),
    ('src/engine/procedural.rs', 418, '规划特性保留：地面细节米制（备用）'),
    ('src/engine/procedural.rs', 424, '规划特性保留：周期网格哈希（备用）'),
    ('src/engine/procedural.rs', 431, '规划特性保留：周期噪声（备用）'),
    ('src/engine/procedural.rs', 462, '规划特性保留：地面细节纹理生成（备用）'),
    ('src/engine/procedural.rs', 489, '规划特性保留：默认地面细节纹理（备用）'),
    ('src/engine/ray_tracer.rs', 78, '规划特性保留：PT 太阳色常量（备用）'),
    ('src/engine/ray_tracer.rs', 80, '规划特性保留：PT 环境光颜色（备用）'),
    ('src/engine/ray_tracer.rs', 81, '规划特性保留：PT 环境光强度（备用）'),
]
# 插入（倒序行号，避免行号漂移）
insertions.sort(key=lambda x: -x[1])
for path, line, note in insertions:
    ls = io.open(path, encoding='utf-8').read().split('\n')
    idx = line - 1
    if ls[idx].lstrip().startswith('#['):
        continue
    ls.insert(idx, '#[allow(dead_code)] // ' + note)
    io.open(path, 'w', encoding='utf-8', newline='\n').write('\n'.join(ls))
    print('ok', path, line)
# frame_size 删（4600 renderer）——找并删除方法块
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = re.sub(r'    /// 当前交换链尺寸 \(宽, 高\)——PT 原生分辨率取它\n    pub fn frame_size\(&self\) -> \(u32, u32\) \{\n        \(self\.swapchain_extent\.width, self\.swapchain_extent\.height\)\n    \}\n\n', '', s)
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='\n').write(s)
print('frame_size deleted')
