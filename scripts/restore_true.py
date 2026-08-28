# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = vec3<f32>(step(0.3, color.r), step(0.6, color.r), step(0.9, color.r)); // 三档编码", "output.color = color * inst.tint.rgb;")
# fs 恢复（去掉顶部的无条件 return）
s = s.replace("    return vec4<f32>(input.color, 1.0); // 终极直通\n    if (input.fade <= 0.02) {", "    if (input.fade <= 0.02) {")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('vs restored:', 'color * inst.tint.rgb;' in s, '| fs restored:', '终极直通' not in s)
