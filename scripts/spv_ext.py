# -*- coding: utf-8 -*-
import struct, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
b = open(r'D:\Rust\steel-front\assets\triangle.vert.spv', 'rb').read()
i = 5
exts = {}
while i < len(b)//4:
    insn = struct.unpack('<I', b[i*4:i*4+4])[0]
    wc = insn >> 16
    op = insn & 0xFFFF
    words = [struct.unpack('<I', b[(i+j)*4:(i+j+1)*4])[0] for j in range(wc)]
    if op == 12:
        ext = words[1]
        extins = words[2]
        exts[(ext, extins)] = exts.get((ext, extins), 0) + 1
    i += wc
print('ext insts:', exts)
# GLSL.std.450: 45=FMul 46=... 48=Step
