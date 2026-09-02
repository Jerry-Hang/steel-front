# -*- coding: utf-8 -*-
import io, re

def add_one(path, pat, note):
    s = io.open(path, encoding='utf-8').read()
    if any('#[allow(dead_code)] // ' + note in ln for ln in s.split('\n')):
        return
    m = re.search(pat, s, re.M)
    if not m:
        print('MISS', path, pat[:30])
        return
    ins = '#[allow(dead_code)] // ' + note + '\n'
    s = s[:m.start()] + ins + s[m.start():]
    io.open(path, 'w', encoding='utf-8', newline='\n').write(s)
    print('OK', path, pat[:24])

add_one('src/engine/game.rs', r'\n\s*pub fn map_obstacles\(', '规划：障碍查询API备用')
add_one('src/engine/game.rs', r'\n\s*pub fn player_speed\(', '规划：玩家速度查询备用')
add_one('src/engine/geom.rs', r'\n\s*pub fn from_tag\(', '规划：形状标签解析备用')
add_one('src/engine/geom.rs', r'\n\s*pub fn inscribed_radius_factor\(', '规划：内切半径系数备用')
add_one('src/engine/procedural.rs', r'\nconst GROUND_DETAIL_SIZE\b', '规划：地面细节尺寸备用')
add_one('src/engine/procedural.rs', r'\nconst GROUND_DETAIL_METRES\b', '规划：地面细节米制备用')
add_one('src/engine/procedural.rs', r'\nfn periodic_grid_hash\(', '规划：周期网格哈希备用')
add_one('src/engine/procedural.rs', r'\nfn periodic_noise\(', '规划：周期噪声备用')
add_one('src/engine/procedural.rs', r'\nfn generate_ground_detail_texture\(', '规划：地面细节纹理生成备用')
add_one('src/engine/procedural.rs', r'\nfn generate_default_ground_detail_texture\(', '规划：默认地面细节纹理备用')
add_one('src/engine/assets.rs', r'\npub fn parse_obj\(', '规划：OBJ 解析器备用')
