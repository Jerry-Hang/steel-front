# -*- coding: utf-8 -*-
import io, re
p = 'src/net.rs'
s = io.open(p, encoding='utf-8').read()
# 匹配 player_id 与 name 后缺失 version 的 Join 构造（name: X, 或 name: String::new(),）
pat = re.compile(r'(NetworkMessage::Join \{\s*player_id: ([^,]+),\s*name: ([^,}]+),\s*\})')
def fix(m):
    return 'NetworkMessage::Join {\n                player_id: %s,\n                name: %s,\n                version: SESSION_VERSION,\n            }' % (m.group(2), m.group(3))
s2, n = pat.subn(fix, s)
print('fixed', n)
io.open(p, 'w', encoding='utf-8', newline='').write(s2)
