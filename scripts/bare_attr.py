# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = color * inst.tint.rgb;", "output.color = color; // 裸 attr 直通")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('bare attr' if 'output.color = color; // 裸 attr' in s else 'FAIL')
