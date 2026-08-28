# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = color * inst.tint.rgb;\n    output.uv = uv;", "output.color = vec3<f32>(step(0.5, color.r), 0.0, 0.0); // 探针\n    output.uv = uv;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('probe in' if 'step(0.5, color.r)' in s else 'FAIL')
