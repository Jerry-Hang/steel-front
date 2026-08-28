# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = color * inst.tint.rgb;", "output.color = vec3<f32>(0.15, 0.16, 0.19); // 全const深灰")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('const-grey installed' if '全const深灰' in s else 'FAIL')
