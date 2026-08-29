# -*- coding: utf-8 -*-
import io
s = io.open('build_spv_rt.rs', encoding='utf-8').read()
s = s.replace("""    emit(&mut w, 248, &[l_hit]);""", """    emit(&mut w, 248, &[l_hit]);""")
s = s.replace("""    emit(&mut w, 32, &[p_hit, t_p_u32, 12, t_u32]);      // 临时指针类型不能这么用——改用直接 Store（下面修正）",
    emit(&mut w, 62, &[g_hits, one]);                    // OpStore %g_hits %one（写 1 到第 0 行——基准近似）",""", """    emit(&mut w, 65, &[p_hit, t_p_u32, g_hits, gx]);      // OpAccessChain hits[gid.x]",
    emit(&mut w, 62, &[p_hit, one]);                     // OpStore hits[gid.x]=1",""")
io.open('build_spv_rt.rs', 'w', encoding='utf-8', newline='').write(s)
print('fixed', 'p_hit, t_p_u32' in s and 'OpStore %g_hits' not in s)
