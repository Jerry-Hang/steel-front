# -*- coding: utf-8 -*-
import io, re

def add_one(path, pat, note):
    s = io.open(path, encoding='utf-8').read()
    if '#[allow(dead_code)] // ' + note in s:
        return
    m = re.search(pat, s, re.M)
    if not m:
        print('MISS', pat[:30]); return
    s = s[:m.start()] + '#[allow(dead_code)] // ' + note + '\n' + s[m.start():]
    io.open(path, 'w', encoding='utf-8', newline='\n').write(s)
    print('OK', pat[:26])

add_one('src/engine/geom.rs', r'\npub const fn from_tag\(', '规划：形状标签解析备用')
add_one('src/engine/geom.rs', r'\npub const fn inscribed_radius_factor\(', '规划：内切半径系数备用')
add_one('src/engine/procedural.rs', r'\npub const GROUND_DETAIL_SIZE\b', '规划：地面细节尺寸备用')
add_one('src/engine/procedural.rs', r'\npub const GROUND_DETAIL_METRES\b', '规划：地面细节米制备用')
add_one('src/engine/procedural.rs', r'\npub fn generate_ground_detail_texture\(', '规划：地面细节纹理生成备用')
add_one('src/engine/procedural.rs', r'\npub fn generate_default_ground_detail_texture\(', '规划：默认地面细节纹理备用')
