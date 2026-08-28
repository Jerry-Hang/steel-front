# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = vec3<f32>(step(0.5, color.r), 0.0, 0.0); // 探针", "output.color = color * inst.tint.rgb;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('restored' if 'step(0.5, color.r)' not in s else 'fail')
