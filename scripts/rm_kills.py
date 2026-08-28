# -*- coding: utf-8 -*-
import io, re
p = 'src/engine/ai_command.rs'
s = io.open(p, encoding='utf-8').read()
# 删 kills: 0,（5 处，含其前 contact 行保留）
lines = s.split('\n')
out = []
removed = 0
for ln in lines:
    if re.match(r'^(\s*)kills: 0,\s*$', ln):
        removed += 1
        continue
    out.append(ln)
print('removed', removed)
s = '\n'.join(out)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
