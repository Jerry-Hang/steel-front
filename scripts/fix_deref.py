# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("|| *id == 0)", "|| **id == 0)")
s = s.replace("tint: if *id == 0", "tint: if **id == 0")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fixed')
