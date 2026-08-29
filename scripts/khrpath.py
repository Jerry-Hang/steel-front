# -*- coding: utf-8 -*-
import re
base = r'C:\Users\Jerry-Huang\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\ash-0.38.0+1.3.281\src\lib.rs'
s = open(base, encoding='utf-8', errors='replace').read()
for m in re.finditer(r'(extensions_generated|mod khr|pub mod khr|pub use khr)', s):
    print(s[m.start()-40:m.start()+80].replace('\n', ' '))
