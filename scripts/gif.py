# -*- coding: utf-8 -*-
import re
s = open(r'C:\Users\Jerry-Huang\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\ash-0.38.0+1.3.281\src\vk\definitions.rs', encoding='utf-8', errors='replace').read()
m = re.search(r'pub struct GeometryInstanceFlagsKHR[^}]*}([^}]*)', s)
print(m.group(1)[:500] if m else 'not found')
