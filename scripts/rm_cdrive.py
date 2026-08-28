# -*- coding: utf-8 -*-
import io
for p in ['docs/HANDOFF-2026-08-25.md', 'docs/HANDOFF-2026-08-27.md']:
    s = io.open(p, encoding='utf-8').read()
    lines = s.split('\n')
    out = [l for l in lines if not ('C 盘' in l or 'C盘' in l or 'pagefile' in l or 'D:\\C' in l or '迁移 22GB' in l or '迁移目录' in l and 'C' in l)]
    if len(out) != len(lines):
        io.open(p, 'w', encoding='utf-8', newline='').write('\n'.join(out))
        print(p, 'cleaned', len(lines) - len(out))
    else:
        print(p, 'no C lines')
