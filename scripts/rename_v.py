# -*- coding: utf-8 -*-
import io
p = 'src/net.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("pub const PROTOCOL_VERSION: u16 = 2;", "pub const SESSION_VERSION: u16 = 2;")
s = s.replace("if version != PROTOCOL_VERSION {", "if version != SESSION_VERSION {")
s = s.replace("服务器 {PROTOCOL_VERSION}）", "服务器 {SESSION_VERSION}）")
s = s.replace("version: PROTOCOL_VERSION,", "version: SESSION_VERSION,")
s = s.replace("version: PROTOCOL_VERSION,", "version: SESSION_VERSION,")
s = s.replace("version: PROTOCOL_VERSION,name:", "version: SESSION_VERSION,name:")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('renamed')
