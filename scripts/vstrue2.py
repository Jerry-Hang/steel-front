# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = vec3<f32>(step(0.5, position.y), step(0.5, color.g), 0.0); // 双通道二值", "output.color = color * inst.tint.rgb;")
# fs 除fade测试 → 正常 ×fade
s = s.replace("        return vec4<f32>(input.color, 1.0); // 除 fade 测试", "        return vec4<f32>(input.color * input.fade, 1.0);")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('vs/fs true restored:', 'color * inst.tint.rgb;' in s and '双通道二值' not in s)
