# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("        return vec4<f32>(input.color * input.fade, 1.0);", "        return vec4<f32>(input.color, 1.0); // fade=直通测试")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fs fade-passthrough' if 'input.color, 1.0); // fade=直通' in s else 'FAIL')
