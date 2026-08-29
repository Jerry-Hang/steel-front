# -*- coding: utf-8 -*-
import io
s = io.open('build_spv_rt.rs', encoding='utf-8').read()
s = s.replace("    emit(&mut w, 43, &[c_smallf, t_f32, 0);\n    // —— 修正 c_smallf：直接 0.001 复用\n    let c_smallf = c001f;", "    // 方向 z 分量直接用 0.001（c_smallf 与 c001f 复用）\n    let c_smallf = c001f;")
io.open('build_spv_rt.rs', 'w', encoding='utf-8', newline='').write(s)
print('fixed malformed')
