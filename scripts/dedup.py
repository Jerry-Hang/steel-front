# -*- coding: utf-8 -*-
import io, re
p = 'src/engine/game.rs'
s = io.open(p, encoding='utf-8').read()
pat = re.compile(r'(\n[ \t]*direct_goal: false,[ \t]*\n[ \t]*direct_x: 0\.0,[ \t]*\n[ \t]*direct_z: 0\.0,[ \t]*){2,}')
s2, n = pat.subn(r'\1', s)
print('dedup', n)
io.open(p, 'w', encoding='utf-8', newline='').write(s2)
