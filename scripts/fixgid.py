# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('    emit(&mut w, 32, &[t_p_v3u, 7, t_v3u]);', '    emit(&mut w, 32, &[t_p_v3u, 1, t_v3u]);')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('gid storage -> Input(1)')
