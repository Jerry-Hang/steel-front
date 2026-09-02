# -*- coding: utf-8 -*-
import io
p = 'README.md'
s = io.open(p, encoding='utf-8').read()
s += P2
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('p2 appended')
