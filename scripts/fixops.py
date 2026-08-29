# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
# OpCapability RayQueryKHR = 4472 ✓ 不变
# emit(&mut w, 4471, ...) initialize → 4473
s = s.replace('emit(&mut w, 4471, &[rq, tlas_l, c0, c255, vzero, c001f, dir, c1000f]);', 'emit(&mut w, 4473, &[rq, tlas_l, c0, c255, vzero, c001f, dir, c1000f]);')
# proceed 4472 → 4477
s = s.replace('emit(&mut w, 4472, &[cont, t_bool, rq]);', 'emit(&mut w, 4477, &[cont, t_bool, rq]);')
# gettype 4476 → 4479
s = s.replace('emit(&mut w, 4476, &[ityp, t_u32, rq, c0]);', 'emit(&mut w, 4479, &[ityp, t_u32, rq, c0]);')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('opcodes corrected:', '4473' in s and '4477' in s and '4479' in s)
