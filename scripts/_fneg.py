import struct
def scan(path):
    data = open(path, 'rb').read()
    words = struct.unpack('<%dI' % (len(data)//4), data)
    i = 5
    n127 = 0; ops = {}
    while i < len(words):
        w = words[i]
        opcode = w & 0xFFFF
        wc = w >> 16
        ops[opcode] = ops.get(opcode, 0) + 1
        if opcode == 127: n127 += 1
        i += wc
    print(path.split('/')[-1], 'FNegate(127):', n127)
for p in ['triangle.vert.spv', 'mesh.spv', 'hud.vert.spv', 'shadow.vert.spv']:
    scan('D:/Rust/steel-front/assets/' + p)