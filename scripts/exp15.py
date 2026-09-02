# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
if 'exposure: 0.2,' in s:
    s = s.replace('exposure: 0.2,', 'exposure: 0.15,')
    io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
    print('exposure 0.15')
else:
    print('exposure miss')
