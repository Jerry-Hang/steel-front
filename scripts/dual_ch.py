# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = color * inst.tint.rgb;", "output.color = vec3<f32>(step(0.5, position.y), step(0.5, color.g), 0.0); // 双通道二值")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('dual-channel' if '双通道二值' in s else 'FAIL')
