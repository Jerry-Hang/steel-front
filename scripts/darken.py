# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
s = s.replace("                            let shade = 0.35 + 0.20 * ndl;\n                            [\n                                (raw[0] * 1.2 * shade).min(1.0),", "                            let shade = 0.30 + 0.20 * ndl;\n                            [\n                                (raw[0] * 0.5 * shade).min(1.0),")
s = s.replace("                            (raw[1] * 1.2 * shade).min(1.0),\n                            (raw[2] * 1.2 * shade).min(1.0),", "                            (raw[1] * 0.5 * shade).min(1.0),\n                            (raw[2] * 0.5 * shade).min(1.0),")
# 烘焙分支 shade
s = s.replace("                            let shade = 0.35 + 0.20 * ndl;\n                            [\n                                (raw[0] * 0.5 * shade).min(1.0),\n                                (raw[1] * 0.5 * shade).min(1.0),\n                                (raw[2] * 0.5 * shade).min(1.0),\n                            ]", "                            let shade = 0.30 + 0.20 * ndl;\n                            [\n                                (raw[0] * 0.5 * shade).min(1.0),\n                                (raw[1] * 0.5 * shade).min(1.0),\n                                (raw[2] * 0.5 * shade).min(1.0),\n                            ]")
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('darkened')
