# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = vec3<f32>(fract(position.x * 7.0), 0.0, 0.0); // pos-x编码", "output.color = color * inst.tint.rgb;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('vs true restored:', 'color * inst.tint.rgb;' in s and 'pos-x编码' not in s)
