# -*- coding: utf-8 -*-
import io, re
p = 'src/net.rs'
s = io.open(p, encoding='utf-8').read()
# 替换 NpcSnapshot { ... hp: X } 不带 team 的
s2 = re.sub(r'(NpcSnapshot \{ id: [^}]*?hp: [0-9.\-]+ \})', r'\1, team: 0 }', s) if False else s
# 手动处理已知 4 个（id/pos/facing/hp 形式）
repls = [
  ("NpcSnapshot { id: 10, pos: [1.0, 0.0, 1.0], facing: 0.25, hp: 100.0 }", "NpcSnapshot { id: 10, pos: [1.0, 0.0, 1.0], facing: 0.25, hp: 100.0, team: 0 }"),
  ("NpcSnapshot { id: 11, pos: [-2.0, 0.0, 4.0], facing: -1.0, hp: 50.0 }", "NpcSnapshot { id: 11, pos: [-2.0, 0.0, 4.0], facing: -1.0, hp: 50.0, team: 0 }"),
  ("NpcSnapshot { id, pos: [0.0; 3], facing: 0.0, hp: 1.0 }", "NpcSnapshot { id, pos: [0.0; 3], facing: 0.0, hp: 1.0, team: 0 }"),
  ("NpcSnapshot { id: 10, pos: [1.0, 0.0, 1.0], facing: 0.25, hp: 100.0 }", "NpcSnapshot { id: 10, pos: [1.0, 0.0, 1.0], facing: 0.25, hp: 100.0, team: 0 }"),
  ("NpcSnapshot { id: 11, pos: [2.0, 0.0, 2.0], facing: -0.5, hp: 40.0 }", "NpcSnapshot { id: 11, pos: [2.0, 0.0, 2.0], facing: -0.5, hp: 40.0, team: 0 }"),
]
for a, b in repls:
    s = s.replace(a, b, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('tests patched')
