# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
old = """    let loop_h = nid(&mut i);
    let cont = nid(&mut i);
    let merge = nid(&mut i);"""
new = """    let loop_h = nid(&mut i);
    let cont = nid(&mut i);
    let merge = nid(&mut i);
    let latch = nid(&mut i);"""
if old in s:
    s = s.replace(old, new, 1)
    s = s.replace("""    emit(&mut w, 248, &[loop_h]);
    emit(&mut w, 246, &[merge, loop_h, 0]);              // OpLoopMerge merge loop_h None
    emit(&mut w, 4477, &[cont, t_bool, rq]);             // proceed
    emit(&mut w, 250, &[cont, loop_h, merge]);           // branchconditional""", """    emit(&mut w, 248, &[loop_h]);
    emit(&mut w, 246, &[merge, latch, 0]);               // OpLoopMerge merge latch None
    emit(&mut w, 4477, &[cont, t_bool, rq]);             // proceed
    emit(&mut w, 250, &[cont, latch, merge]);            // branchconditional
    emit(&mut w, 248, &[latch]);
    emit(&mut w, 249, &[loop_h]);                         // latch: branch loop_h""")
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('latch added')
else:
    print('anch missing')
