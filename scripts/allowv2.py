# -*- coding: utf-8 -*-
import io, re

def add_allow(path, patterns, note):
    s = io.open(path, encoding='utf-8').read()
    for pat in patterns:
        # 避免重复
        if '#[allow(dead_code)] // ' + note in s:
            continue
        # 找声明行
        m = re.search(pat, s)
        if not m:
            print('MISS', path, pat)
            continue
        # 插入前一行（若未 allow）
        line_start = m.start()
        before = s[:line_start]
        if before.rstrip().endswith('#[allow(dead_code)]') or '#[allow(dead_code)]' in before[-60:]:
            continue
        s = s[:line_start] + '#[allow(dead_code)] // ' + note + '\n' + s[line_start:]
    io.open(path, 'w', encoding='utf-8', newline='\n').write(s)
    print('patched', path)

add_allow('src/engine/city.rs', [r'\nconst RELIEF_STEP\b', r'\n    fn ico\(', r'\n    fn slab\(', r'\nfn parking_lot\('], '规划特性保留')
# 注意：note 里全部写中文会重复插入（我上面的逻辑每次都会插——改为 per-symbol note impossible 简化）
