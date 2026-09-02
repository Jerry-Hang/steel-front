# -*- coding: utf-8 -*-
import io
s = io.open('README.md', encoding='utf-8').read()
s = s.replace('| **i7-12700K+ / Ryzen 9 9950X** | RTX 4080 Super / **RX 9070 XT** | 16 GB | 32 GB |', '| **Intel Core Ultra 7 270K Plus / Ryzen 9 9950X** | RTX 4080 Super / **RX 9070 XT** | 16 GB | 32 GB |')
io.open('README.md', 'w', encoding='utf-8', newline='\n').write(s)
print('270K Plus fixed')
