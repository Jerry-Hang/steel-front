# -*- coding: utf-8 -*-
import io
s = io.open(r'assets\rt\pt_panorama.glsl', encoding='utf-8').read()
old = "    float win = mix(64.0f, 1.0f, move);"
new = "    float win = mix(64.0f, 1.0f, move);\n    if (move > 0.5) { win = 1.0; } // 移动>0.5 强制窗口=1（零历史=零叠影）"
if old in s:
    s = s.replace(old, new, 1)
    io.open(r'assets\rt\pt_panorama.glsl', 'w', encoding='utf-8', newline='\n').write(s)
    print('shader win=1 force')
else:
    print('miss glsl')
