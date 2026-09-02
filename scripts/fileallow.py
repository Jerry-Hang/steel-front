# -*- coding: utf-8 -*-
import io
for f in ['src/engine/geom.rs', 'src/engine/procedural.rs']:
    lines = io.open(f, encoding='utf-8').read().split('\n')
    # 找到首个非 //! 非空行（如 use / 注释后）
    ins = None
    for i, ln in enumerate(lines):
        if ln.strip() == '':
            # 找之后第一个非空非 //! 行
            for j in range(i, len(lines)):
                if lines[j].strip() and not lines[j].strip().startswith('//!'):
                    ins = j
                    break
            break
    if ins is not None:
        lines.insert(ins, '#![allow(dead_code)] // 规划保留：库内部接口（生成器/形状）')
        io.open(f, 'w', encoding='utf-8', newline='\n').write('\n'.join(lines))
        print('ok', f, ins)
