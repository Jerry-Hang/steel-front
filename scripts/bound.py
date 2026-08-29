# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("    w[3] = w.len() as u32;", "    w[3] = 44; // bound = 最大 ID(43) + 1")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('bound fixed')
