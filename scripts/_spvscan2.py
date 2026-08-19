import struct
def scan(path):
    data = open(path, 'rb').read()
    words = struct.unpack('<%dI' % (len(data)//4), data)
    i = 5
    ops = {}
    fneg = 0
    while i < len(words):
        w = words[i]
        opcode = w & 0xFFFF
        wc = w >> 16
        ops[opcode] = ops.get(opcode, 0) + 1
        if opcode == 103: fneg += 1
        i += wc
    print(path.split('/')[-1], 'OpFNegate(103):', fneg)
    return fneg
a = scan('D:/Rust/steel-front/assets/triangle.vert.spv')
b = scan('D:/Rust/steel-front/assets/mesh.spv')
c = scan('D:/Rust/steel-front/assets/shadow.vert.spv')
print('vertex:', a, 'mesh:', b, 'shadow:', c)