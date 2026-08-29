# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""    emit(&mut w, 65, &[p_hit, t_p_u32, g_hits, gx]);
    emit(&mut w, 62, &[p_hit, one]);
    emit(&mut w, 248, &[l_skip]);""", """    emit(&mut w, 65, &[p_hit, t_p_u32, g_hits, gx]);
    emit(&mut w, 62, &[p_hit, one]);
    emit(&mut w, 249, &[l_skip]);
    emit(&mut w, 248, &[l_skip]);""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('hit skip branch added')
