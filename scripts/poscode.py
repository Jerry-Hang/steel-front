# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = color; // attr直通最终", "output.color = vec3<f32>(fract(position.x * 7.0), 0.0, 0.0); // pos-x编码")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('pos-code installed' if 'pos-x编码' in s else 'FAIL')
