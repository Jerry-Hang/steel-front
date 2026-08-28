# -*- coding: utf-8 -*-
import io, re
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""                            phase: self.anim_clock,
                            moving: true,
                            firing: false,""",
"""                            phase: self.anim_clock,
                            moving: true,
                            firing: e.firing,""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('main firing ok')

p2 = 'src/net.rs'
s2 = io.open(p2, encoding='utf-8').read()
# 测试构造点补 firing: 0（5 处 team: 0 结尾的 NpcSnapshot）
s2 = re.sub(r'(NpcSnapshot \{ [^}]*?team: 0 \})', r'\1, firing: 0 }', s2)
io.open(p2, 'w', encoding='utf-8', newline='').write(s2)
print('tests firing ok')
