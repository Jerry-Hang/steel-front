# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("        return vec4<f32>(input.color * input.fade, 1.0);", "        return vec4<f32>(input.color, 1.0); // 除 fade 测试")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('no-fade installed' if 'return vec4<f32>(input.color, 1.0); // 除 fade' in s else 'FAIL')
