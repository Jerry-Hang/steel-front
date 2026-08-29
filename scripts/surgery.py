# -*- coding: utf-8 -*-
import io
p = 'build_spv_rt.rs'
lines = io.open(p, encoding='utf-8').read().split('\n')
out = []
skip = False
for l in lines:
    if 'fn-internal-pointer-orphan' in l or ('&[p_hit, t_p_u32, 12, t_u32]' in l and 'fn' not in l and 'emit(&mut w, 32' in l):
        skip = True
    if skip:
        # 跳过该 emit 行
        if l.strip().endswith(';') or l.strip().endswith(';'):
            skip = False
            continue
        skip = False
        continue
    out.append(l)
s = '\n'.join(out)
# 孤儿类型发射的精确删除（python 直接字符串级）
s = s.replace('''    emit(&mut w, 32, &[p_hit, t_p_u32, 12, t_u32]);      // 临时指针类型不能这么用——改用直接 Store（下面修正）''', '')
s = s.replace('''    emit(&mut w, 32, &[p_hit, t_p_u32, 12, t_u32]);''', '')
# 循环区整块重写（definitive）
import re
old_loop = '''    emit(&mut w, 248, &[loop_h]);
    emit(&mut w, 246, &[merge, latch, 0]);               // OpLoopMerge merge latch None
    emit(&mut w, 4477, &[cont, t_bool, rq]);             // proceed
    emit(&mut w, 250, &[cont, latch, merge]);            // branchconditional
    emit(&mut w, 248, &[latch]);
    emit(&mut w, 249, &[loop_h]);                         // latch: branch loop_h'''
new_loop = '''    emit(&mut w, 248, &[loop_h]);
    emit(&mut w, 246, &[merge, latch, 0]);
    emit(&mut w, 4477, &[cont, t_bool, rq]);
    emit(&mut w, 250, &[cont, latch, merge]);
    emit(&mut w, 248, &[latch]);
    emit(&mut w, 249, &[loop_h]);'''
if old_loop in s:
    s = s.replace(old_loop, new_loop, 1)
    print('loop region exact matched')
else:
    # 原始格式（无 latch block）检查
    if 'emit(&mut w, 250, &[cont, loop_h, merge]);' in s:
        s = s.replace('''    emit(&mut w, 248, &[loop_h]);
    emit(&mut w, 246, &[merge, latch, 0]);               // OpLoopMerge merge latch None
    emit(&mut w, 4477, &[cont, t_bool, rq]);             // proceed
    emit(&mut w, 250, &[cont, latch, merge]);            // branchconditional
    emit(&mut w, 248, &[latch]);
    emit(&mut w, 249, &[loop_h]);                         // latch: branch loop_h''', new_loop)
        print('fallback replaced')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('surgery done; orphan-check:', 'p_hit, t_p_u32' not in s)
