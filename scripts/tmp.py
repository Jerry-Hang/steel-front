# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# t_p_v3u 从 Function 7 → Input 1（builtin 变量用）
old = "    let t_p_v3u = nid(&mut i);\n    let t_rq = nid(&mut i);"
# 这个在 build_spv_rt.rs 里！先看那边
