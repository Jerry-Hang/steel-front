# -*- coding: utf-8 -*-
import re
p = r'C:\Users\Jerry-Huang\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\ash-0.38.0+1.3.281\src\extensions_generated.rs'
s = open(p, encoding='utf-8', errors='replace').read()
for m in re.finditer(r'pub mod acceleration_structure \{', s):
    seg = s[m.start():m.start()+1200]
    if 'Device' in seg:
        print(seg[:900])
        break
