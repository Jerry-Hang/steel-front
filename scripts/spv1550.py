# -*- coding: utf-8 -*-
import struct, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
b = open(r'D:\Rust\steel-front\assets\triangle.vert.spv', 'rb').read()
print('size', len(b), 'magic', hex(struct.unpack('<I', b[:4])[0]))
i = 5
ops = {}
while i < len(b)//4:
    insn = struct.unpack('<I', b[i*4:i*4+4])[0]
    wc = insn >> 16
    op = insn & 0xFFFF
    ops[op] = ops.get(op, 0) + 1
    i += wc
print('op histogram:', dict(sorted(ops.items())))
# 具体：找 vs_main 的 OpAccessChain/OpLoad 链（颜色处理）
i = 5
insts = []
while i < len(b)//4:
    insn = struct.unpack('<I', b[i*4:i*4+4])[0]
    wc = insn >> 16
    op = insn & 0xFFFF
    words = [struct.unpack('<I', b[(i+j)*4:(i+j+1)*4])[0] for j in range(wc)]
    insts.append((op, words))
    i += wc
# 打印 OpExtInst（GLSL std450）+ OpLoad + OpStore 概要
for op, words in insts:
    if op in (12, 61, 54, 62, 62):
        print('op', op, 'words', words[:4])
