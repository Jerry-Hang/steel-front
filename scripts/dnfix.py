# -*- coding: utf-8 -*-
import io
s = io.open(r'assets\rt\denoise.comp', encoding='utf-8').read()
s = s.replace('    outlet:;\n', '')
io.open(r'assets\rt\denoise.comp', 'w', encoding='utf-8', newline='\n').write(s)
print('fixed label')
