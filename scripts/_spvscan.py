import struct
def scan(path):
    data = open(path, 'rb').read()
    words = struct.unpack('<%dI' % (len(data)//4), data)
    magic, ver, gen, bound, schema = words[:5]
    # walk instructions from word 5
    i = 5
    fneg = 0; pos_builtin = 0
    ops = {}
    while i < len(words):
        w = words[i]
        opcode = w & 0xFFFF
        wc = w >> 16
        ops[opcode] = ops.get(opcode, 0) + 1
        if opcode == 127:  # OpFNegate
            fneg += 1
        i += wc
    print(path.split('\\')[-1], 'fnegate:', fneg, 'total ops:', sum(ops.values()))
    return fneg
a = scan('D:/Rust/steel-front/assets/triangle.vert.spv')
b = scan('D:/Rust/steel-front/assets/mesh.spv')
print('vertex flip:', a, ' mesh flip:', b)