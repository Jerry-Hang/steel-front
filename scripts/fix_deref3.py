# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("(*id >= 100_000 && e.hp > 0.0) || **id == 0", "(**id >= 100_000 && e.hp > 0.0) || **id == 0")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('filter fixed')
