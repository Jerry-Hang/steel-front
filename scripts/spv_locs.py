# -*- coding: utf-8 -*-
import struct, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
b = open(r'D:\Rust\steel-front\assets\triangle.vert.spv', 'rb').read()
word = struct.unpack('<I', b[:4])[0]
print('magic ok' if word == 0x07230203 else 'BAD MAGIC')
# 遍历指令流
i = 5  # 头部 5 word
res = []
while i < len(b)//4:
    insn = struct.unpack('<I', b[i*4:i*4+4])[0]
    wc = insn >> 16
    op = insn & 0xFFFF
    words = [ struct.unpack('<I', b[(i+j)*4:(i+j+1)*4])[0] for j in range(wc) ]
    if op == 71:  # OpDecorate
        target, deco = words[1], words[2]
        if deco == 30:  # Location
            res.append((target, words[3]))
    if op == 90:  # OpTypePointer? skip detail
        pass
    i += wc
print('LOCATIONS (target, loc):', res)
