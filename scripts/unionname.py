# -*- coding: utf-8 -*-
import re
base = r'C:\Users\Jerry-Huang\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\ash-0.38.0+1.3.281\src\vk.rs'
s = open(base, encoding='utf-8', errors='replace').read()
m = re.search(r'pub union AccelerationStructureGeometryDataKHR', s)
if m:
    print(s[m.start():m.start()+400])
else:
    print('union not in vk.rs')
