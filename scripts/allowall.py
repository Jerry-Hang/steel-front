# -*- coding: utf-8 -*-
import io, re

# 记录各符号所在文件（从 warn16 采集、人工补全）
targets = [
    ('src/engine/assets.rs', 'parse_obj', '规划特性保留：OBJ 解析器（备用导入）'),
    ('src/engine/city.rs', 'RELIEF_STEP', '规划特性保留：浮雕地面细节步长（备用）'),
    ('src/engine/city.rs', 'ico', '规划特性保留：二十面体生成器（备用）'),
    ('src/engine/city.rs', 'slab', '规划特性保留：板式建筑生成器（备用）'),
    ('src/engine/city.rs', 'parking_lot', '规划特性保留：停车场地块（备用）'),
    ('src/engine/city.rs', 'map_obstacles', '规划特性保留：障碍地图查询（备用）'),
    ('src/engine/city.rs', 'player_speed', '规划特性保留：玩家速度查询（备用）'),
    ('src/engine/city.rs', 'from_tag', '规划特性保留：形状标签解析（备用）'),
    ('src/engine/city.rs', 'inscribed_radius_factor', '规划特性保留：内切半径系数（备用）'),
    ('src/engine/renderer.rs', 'GROUND_DETAIL_SIZE', '规划特性保留：地面细节纹理尺寸（备用）'),
    ('src/engine/renderer.rs', 'GROUND_DETAIL_METRES', '规划特性保留：地面细节纹米制（备用）'),
    ('src/engine/renderer.rs', 'periodic_grid_hash', '规划特性保留：周期网格哈希（备用）'),
    ('src/engine/renderer.rs', 'periodic_noise', '规划特性保留：周期噪声（备用）'),
    ('src/engine/renderer.rs', 'generate_ground_detail_texture', '规划特性保留：地面细节纹理生成（备用）'),
    ('src/engine/renderer.rs', 'generate_default_ground_detail_texture', '规划特性保留：默认地面细节纹理（备用）'),
    ('src/engine/ray_tracer.rs', 'PT_SUN_COLOR', '规划特性保留：PT 太阳色常量（shader 已内置，备用）'),
    ('src/engine/ray_tracer.rs', 'PT_AMBIENT_COLOR', '规划特性保留：PT 环境光常量（备用）'),
    ('src/engine/ray_tracer.rs', 'PT_AMBIENT_INTENSITY', '规划特性保留：PT 环境光强度（备用）'),
]
for path, sym, note in targets:
    s = io.open(path, encoding='utf-8').read()
    if '#[allow(dead_code)] // ' + sym in s:
        continue
    # 判断符号类型：const/fn（普通）/方法（fn 在 impl 内）：规则：搜行含 sym 且以 fn/const/static 开头
    lines = s.split('\n')
    changed = False
    for i, ln in enumerate(lines):
        if ln.lstrip().startswith('pub fn ' + sym + ' ') or ln.lstrip().startswith('fn ' + sym + ' ') or ln.lstrip().startswith('const ' + sym + ':') or ln.lstrip().startswith('static ' + sym + ':'):
            # 若前一行为 allow 则跳过；否则插
            if i > 0 and '#[allow(dead_code)]' in lines[i-1]:
                changed = True
                break
            lines.insert(i, '#[allow(dead_code)] // ' + note)
            changed = True
            break
    if changed:
        io.open(path, 'w', encoding='utf-8', newline='\n').write('\n'.join(lines))
        print('ok', path, sym)
    else:
        print('MISS', path, sym)
