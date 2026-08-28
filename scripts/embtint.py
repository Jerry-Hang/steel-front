# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = color * inst.tint.rgb;", "output.color = inst.tint.rgb * 0.25; // 嵌入tint")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('embed tint' if '嵌入tint' in s else 'FAIL')
