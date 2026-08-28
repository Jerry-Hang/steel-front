# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = color * inst.tint.rgb;", "output.color = inst.tint.rgb * 0.25; // 仪器：tint真值")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('tint-instrument installed' if 'inst.tint.rgb * 0.25' in s else 'FAIL')
