# -*- coding: utf-8 -*-
import io
# renderer 回退
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("uv: [v.color[0], v.color[1]],", "uv: v.uv,")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
# build.rs 回退
p2 = 'build.rs'
s2 = io.open(p2, encoding='utf-8').read()
s2 = s2.replace("output.color = color; // 实验：attr 直通（去 tint 乘）", "output.color = color * inst.tint.rgb;")
io.open(p2, 'w', encoding='utf-8', newline='').write(s2)
print('reverted')
