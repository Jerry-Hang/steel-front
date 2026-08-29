# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
s = io.open(p, encoding='utf-8').read()
# 1) tlas 变量 → storage buffer u64 指针类型
s = s.replace('    let t_p_acc = nid(&mut i);', '    let t_p_acc = nid(&mut i);')
# 2) 增加：u64 类型 + 转换指令
s = s.replace('    emit(&mut w, 32, &[t_p_acc, 0, t_accel]);', '    emit(&mut w, 32, &[t_p_acc, 12, t_u64]);')
s = s.replace('    let t_accel = nid(&mut i);', '    let t_accel = nid(&mut i);\n    let t_u64 = nid(&mut i);')
s = s.replace('    emit(&mut w, 22, &[t_f32, 32]);', '    emit(&mut w, 22, &[t_f32, 32]);')
s = s.replace('    emit(&mut w, 21, &[t_u32, 32, 0]);', '    emit(&mut w, 21, &[t_u32, 32, 0]);')
# 3) u64 类型发射（21 之后：追加 OpTypeInt 64 0）
s = s.replace('    emit(&mut w, 23, &[t_v3u, t_u32, 3]);', '''    emit(&mut w, 21, &[t_u64, 64, 0]);
    emit(&mut w, 23, &[t_v3u, t_u32, 3]);''')
# 4) 函数内：load u64 + convert
s = s.replace('''    emit(&mut w, 61, &[tlas_l, t_accel, g_tlas]);''', '''    emit(&mut w, 61, &[tlas_l, t_u64, g_tlas]);
    emit(&mut w, 4447, &[tlas_c, t_accel, tlas_l]);''')
s = s.replace('    let tlas_l = nid(&mut i);', '    let tlas_l = nid(&mut i);\n    let tlas_c = nid(&mut i);')
# 5) init 使用转换结果
s = s.replace('emit(&mut w, 4473, &[rq, tlas_l, c0, c255, vzero, c001f, dir, c1000f]);', 'emit(&mut w, 4473, &[rq, tlas_c, c0, c255, vzero, c001f, dir, c1000f]);')
# 6) 描述符集装饰不变（binding 0 仍为 tlas 变量——但现在是 storage buffer u64）
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('convert path installed')
