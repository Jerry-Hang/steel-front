# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("                        let idn = **id;", "                        let idn = *id;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok')
