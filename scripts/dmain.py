# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
old = "                if let Err(e) = renderer.init_pt_resident(pt_w, pt_h) {"
new = "                if let Err(e) = renderer.init_pt_resident(pt_w, pt_h) {\n                    log::info!(\"PT-RESIDENT init: {e}\");\n                } else if let Err(e) = renderer.init_pt_denoise(pt_w, pt_h) {\n                    log::info!(\"PT-DENOISE init: {e}\");\n                }"
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('main dn init')
else:
    print('miss')
