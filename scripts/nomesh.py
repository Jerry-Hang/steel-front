# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
old = "            mesh_enabled: mesh_shader_available,"
new = "            mesh_enabled: false, // 临时分离测试"
if old not in s:
    print('anchor missing')
else:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('mesh disabled')
