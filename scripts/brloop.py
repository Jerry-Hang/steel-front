# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""    emit(&mut w, 4473, &[rq, tlas_l, c0, c255, vzero, c001f, dir, c1000f]);
    emit(&mut w, 248, &[loop_h]);""", """    emit(&mut w, 4473, &[rq, tlas_l, c0, c255, vzero, c001f, dir, c1000f]);
    emit(&mut w, 249, &[loop_h]);
    emit(&mut w, 248, &[loop_h]);""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('branch to loop added')
