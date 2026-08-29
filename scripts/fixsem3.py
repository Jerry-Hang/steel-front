# -*- coding: utf-8 -*-
import io
s = io.open('scripts/sem3.py', encoding='utf-8').read()
s = s.replace("        expectptr('Proceed.RQ', w[0], 'rayquery')", "        expectptr('Proceed.RQ', w[2], 'rayquery')")
s = s.replace("        expectptr('GetType.RQ', w[1], 'rayquery')", "        expectptr('GetType.RQ', w[2], 'rayquery')")
s = s.replace("        expect('GetType.X', w[2], 'int')", "        expect('GetType.X', w[3], 'int')")
io.open('scripts/sem3.py', 'w', encoding='utf-8', newline='').write(s)
print('fixed offsets')
