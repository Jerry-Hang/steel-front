# -*- coding: utf-8 -*-
import io
lines = io.open('src/main.rs', encoding='utf-8').read().split('\n')
# 找：第二个 load_gun_glb 起始（"导入枪模：优先"）到 first_person_gun_mesh 前
start = None
for i, l in enumerate(lines):
    if '导入枪模：优先' in l and start is None:
        # 第一个（我刚插的 TEMP 之前？——找 TEMP 标记之后的那个）
        if any('TEMP' in x for x in lines[max(0,i-5):i]):
            start = i
if start is None:
    print('not found')
else:
    end = None
    for i in range(start, min(len(lines), start+130)):
        if 'fn first_person_gun_mesh' in lines[i]:
            end = i
            break
    if end:
        del lines[start:end]
        io.open('src/main.rs', 'w', encoding='utf-8', newline='').write('\n'.join(lines))
        print('deleted', start, 'to', end)
    else:
        print('end not found')
