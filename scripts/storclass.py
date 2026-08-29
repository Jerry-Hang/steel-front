# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('emit(&mut w, 59, &[g_tlas, t_p_acc]);', 'emit(&mut w, 59, &[g_tlas, t_p_acc, 0]);')
s = s.replace('emit(&mut w, 59, &[g_hits, t_p_u32]);', 'emit(&mut w, 59, &[g_hits, t_p_u32, 12]);')
s = s.replace('emit(&mut w, 59, &[g_gid, t_p_v3u]);', 'emit(&mut w, 59, &[g_gid, t_p_v3u, 1]);')
s = s.replace('emit(&mut w, 59, &[rq, t_p_rq]);', 'emit(&mut w, 59, &[rq, t_p_rq, 7]);')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('storage classes added')
