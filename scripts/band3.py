# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = color; // 裸 attr 直通", "output.color = vec3<f32>(step(0.3, color.r), step(0.6, color.r), step(0.9, color.r)); // 三档编码")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('3-band installed' if '三档编码' in s else 'FAIL')
