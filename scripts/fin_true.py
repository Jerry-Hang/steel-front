# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = color; // 嵌入attr直通", "output.color = color * inst.tint.rgb;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('final true:', 'color * inst.tint.rgb;' in s and '嵌入attr直通' not in s)
