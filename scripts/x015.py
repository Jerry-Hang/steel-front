# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
s = s.replace("(raw[0] * 0.5 * shade).min(1.0),", "(raw[0] * 0.15 * shade).min(1.0),")
s = s.replace("(raw[1] * 0.5 * shade).min(1.0),", "(raw[1] * 0.15 * shade).min(1.0),")
s = s.replace("(raw[2] * 0.5 * shade).min(1.0),", "(raw[2] * 0.15 * shade).min(1.0),")
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('×0.15')
