# -*- coding: utf-8 -*-
import io
p = 'scripts/blender_bake.py'
s = io.open(p, encoding='utf-8').read()
s = s.replace("base = (0.15, 0.16, 0.185, 1.0)", "base = (0.055, 0.056, 0.06, 1.0)")
s = s.replace("base = (0.12, 0.13, 0.15, 1.0)", "base = (0.055, 0.056, 0.06, 1.0)")
s = s.replace("(max(raw[0], 0.08), max(raw[1], 0.09), max(raw[2], 0.11), 1.0)", "(max(raw[0], 0.05), max(raw[1], 0.052), max(raw[2], 0.058), 1.0)")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('neutral base set')
