# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = inst.tint.rgb * 0.25; // 嵌入tint", "output.color = color; // 嵌入attr直通")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('embed attr direct' if '嵌入attr直通' in s else 'FAIL')
