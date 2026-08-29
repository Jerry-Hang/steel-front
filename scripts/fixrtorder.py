# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("emit(&mut w, 4477, &[cont, t_bool, rq]);", "emit(&mut w, 4477, &[t_bool, cont, rq]);")
s = s.replace("emit(&mut w, 4479, &[ityp, t_u32, rq, c0]);", "emit(&mut w, 4479, &[t_u32, ityp, rq, c0]);")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('RT op order fixed')
