# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('emit(&mut w, 46, &[vzero, t_v3f, c0f, c0f, c0f]);', 'emit(&mut w, 44, &[vzero, t_v3f, c0f, c0f, c0f]);')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('composite = 44')
