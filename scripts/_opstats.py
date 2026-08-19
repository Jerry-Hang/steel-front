import struct
data = open('D:/Rust/steel-front/assets/triangle.vert.spv', 'rb').read()
words = struct.unpack('<%dI' % (len(data)//4), data)
i = 5
ops = {}
while i < len(words):
    w = words[i]
    opcode = w & 0xFFFF
    wc = w >> 16
    ops[opcode] = ops.get(opcode, 0) + 1
    i += wc
print('opcode counts:', sorted(ops.items()))
# check word count integrity
print('total words:', len(words), 'walked:', i)