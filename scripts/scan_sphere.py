
import re, os, glob
files = sorted(glob.glob(r'D:\Rust\steel-front\src\engine\guns\*.rs'))
for f in files:
    if f.endswith('mod.rs'):
        continue
    lines = open(f, encoding='utf-8').read().split('\n')
    for i, line in enumerate(lines):
        if 'sphere(' in line:
            # 找该行及前一行的矩阵（部件格式: (矩阵, sphere(...), 颜色)）
            ctx = line.strip()
            has_scale = 'from_scale' in ctx
            has_rz = 'rz()' in ctx
            # 打印所有 sphere 部件行
            print(f'{os.path.basename(f)}:{i+1} scale={has_scale} | {ctx[:110]}')
