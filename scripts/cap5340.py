# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('    emit(&mut w, 17, &[4472]);   // RayQueryKHR', '    emit(&mut w, 17, &[4472]);   // RayQueryKHR\n    emit(&mut w, 17, &[5340]);   // AccelerationStructureKHR')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('cap 5340 added')
