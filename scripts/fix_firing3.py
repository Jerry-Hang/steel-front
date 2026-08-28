# -*- coding: utf-8 -*-
import io
p = 'src/net.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("}, firing: 0 }", ", firing: 0 }")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fixed')
