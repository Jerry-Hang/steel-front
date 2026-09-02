# -*- coding: utf-8 -*-
import io
s = io.open(r'assets\rt\pt_panorama.glsl', encoding='utf-8').read()
s = s.replace("uint SPP = uint(round(mix(16.0, 64.0, move)));", "uint SPP = uint(round(mix(48.0, 160.0, move)));")
io.open(r'assets\rt\pt_panorama.glsl', 'w', encoding='utf-8', newline='\n').write(s)
print('SPP 48/160')
p = 'src/main.rs'
s2 = io.open(p, encoding='utf-8').read()
s2 = s2.replace("bounces: 6,", "bounces: 8,")
io.open(p, 'w', encoding='utf-8', newline='\n').write(s2)
print('bounces 8')
