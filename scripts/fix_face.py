# -*- coding: utf-8 -*-
import io
p = 'src/engine/assets.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""                let p = if vi > 0 {
                    pos[(vi - 1) as usize]
                } else {
                    pos[(pos.len() as i32 + vi) as usize]
                };
                let t = if ti > 0 {
                    uv[(ti - 1) as usize]
                } else {
                    [0.0, 0.0]
                };
                let n = if ni > 0 {
                    nrm[(ni - 1) as usize]
                } else {
                    [0.0, 1.0, 0.0]
                };""",
"""                let p = if vi > 0 {
                    pos[(vi - 1) as usize]
                } else {
                    pos[(pos.len() as i32 + vi) as usize]
                };
                let t = if ti > 0 {
                    uv[(ti - 1) as usize]
                } else {
                    [0.0, 0.0]
                };
                let n = if ni > 0 {
                    nrm[(ni - 1) as usize]
                } else {
                    [0.0, 1.0, 0.0]
                };""")
# 调用点更新
s = s.replace("push_face(&rest, &mut verts, &mut indices)?;", "push_face(&mut remap, &pos, &uv, &nrm, &rest, &mut verts, &mut indices)?;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('callsite fixed')
