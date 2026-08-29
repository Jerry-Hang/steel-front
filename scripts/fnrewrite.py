# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
i0 = s.find('    // ---- 函数 main ----')
i1 = s.find('    // 头部 word 数 + id 数')
assert i0 > 0 and i1 > i0, (i0, i1)
new_fn = '''    // ---- 函数 main ----
    emit(&mut w, 54, &[f_main, t_void, 0, t_fn]);
    emit(&mut w, 248, &[lbl_entry]);
    emit(&mut w, 59, &[rq, t_p_rq]);
    emit(&mut w, 61, &[gv, t_v3u, g_gid]);
    emit(&mut w, 186, &[gx, t_u32, gv, 0]);
    emit(&mut w, 186, &[gy, t_u32, gv, 1]);
    emit(&mut w, 111, &[fx, t_f32, gx]);
    emit(&mut w, 111, &[fy, t_f32, gy]);
    emit(&mut w, 80, &[dir, t_v3f, fx, fy, c001f]);
    emit(&mut w, 61, &[tlas_l, t_accel, g_tlas]);
    emit(&mut w, 4473, &[rq, tlas_l, c0, c255, vzero, c001f, dir, c1000f]);
    emit(&mut w, 248, &[loop_h]);
    emit(&mut w, 246, &[merge, latch, 0]);
    emit(&mut w, 4477, &[cont, t_bool, rq]);
    emit(&mut w, 250, &[cont, latch, merge]);
    emit(&mut w, 248, &[latch]);
    emit(&mut w, 249, &[loop_h]);
    emit(&mut w, 248, &[merge]);
    emit(&mut w, 4479, &[ityp, t_u32, rq, c0]);
    emit(&mut w, 171, &[ishit, t_bool, ityp, c0]);
    emit(&mut w, 250, &[ishit, l_hit, l_skip]);
    emit(&mut w, 248, &[l_hit]);
    emit(&mut w, 65, &[p_hit, t_p_u32, g_hits, gx]);
    emit(&mut w, 62, &[p_hit, one]);
    emit(&mut w, 248, &[l_skip]);
    emit(&mut w, 253, &[]);
    emit(&mut w, 56, &[]);

'''
# p_hit 类型发射去掉——l_hit 里用 g_hits+access chain 需要 p_hit 是结果 id（已声明）
s = s[:i0] + new_fn + s[i1:]
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('function body rewritten')
