# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = vec3<f32>(0.15, 0.16, 0.19); // 全const深灰", "output.color = color; // attr直通最终")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('attr direct final' if 'attr直通最终' in s else 'FAIL')
