# -*- coding: utf-8 -*-
import struct, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
b = open(r'D:\Rust\steel-front\assets\triangle.vert.spv', 'rb').read()
print('spv size', len(b))
i = 5
ops = {}
while i < len(b)//4:
    insn = struct.unpack('<I', b[i*4:i*4+4])[0]
    wc = insn >> 16
    op = insn & 0xFFFF
    ops[op] = ops.get(op, 0) + 1
    i += wc
print('instruction histogram (op: count):', dict(sorted(ops.items())))
print('has FMul(134)?', ops.get(134, 0), '| has Step(OpExtInst? need ext-inst 48):')
# 统计 GLSL.std.450 inst=48（Step!）次数
cnt = 0
i = 5
while i < len(b)//4:
    insn = struct.unpack('<I', b[i*4:i*4+4])[0]
    wc = insn >> 16
    op = insn & 0xFFFF
    words = [struct.unpack('<I', b[(i+j)*4:(i+j+1)*4])[0] for j in range(wc)]
    if op == 12:  # OpExtInst
        if words[1] == 48:  # Step
            cnt += 1
    i += wc
print('Step count:', cnt)
