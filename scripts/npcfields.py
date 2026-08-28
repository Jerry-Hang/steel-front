# -*- coding: utf-8 -*-
import io, re
p = 'src/engine/game.rs'
s = io.open(p, encoding='utf-8').read()
# 每个 "direct_z: 0.0," 之后补攻击字段（仅构造字面量处，避免重复）
pat = re.compile(r'(direct_z: 0\.0,)\n')
s, n = pat.subn(r'\1\n                attack_timer: 0.0,\n                reposition: None,', s)
print('patched', n)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
