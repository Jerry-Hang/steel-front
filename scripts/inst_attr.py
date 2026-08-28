# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = inst.tint.rgb * 0.25; // 仪器：tint真值", "output.color = color * 3.0; // 仪器：attr真值×3")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('attr-instrument' if 'color * 3.0' in s and 'tint.rgb * 0.25' not in s else 'FAIL')
