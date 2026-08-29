# -*- coding: utf-8 -*-
import re
base = r'C:\Users\Jerry-Huang\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\ash-0.38.0+1.3.281\src\vk\definitions.rs'
s = open(base, encoding='utf-8', errors='replace').read()
for m in re.finditer(r'TRIANGLE_CULL_DISABLE|GEOMETRY_INSTANCE|InstanceFlagsKHR', s):
    print(s[m.start()-20:m.start()+40].replace('\n',''))
    break
m = re.search(r'transform: TransformMatrixKHR[^;]*', s)
print('TransformMatrix:', m.group(0)[:80] if m else 'no')
