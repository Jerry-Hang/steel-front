import struct
def scan(path):
    data = open(path, 'rb').read()
    words = struct.unpack('<%dI' % (len(data)//4), data)
    i = 5
    ops = []
    while i < len(words):
        w = words[i]
        opcode = w & 0xFFFF
        wc = w >> 16
        ops.append((opcode, words[i+1:i+wc]))
        i += wc
    fneg = sum(1 for o, _ in ops if o == 103)
    fmul = sum(1 for o, a in ops if o == 109)
    fsub = sum(1 for o, a in ops if o == 107)
    print(path.split('/')[-1], 'FNegate:', fneg, 'FMul:', fmul, 'FSub:', fsub, 'total:', len(ops))
for p in ['D:/Rust/steel-front/assets/hud.vert.spv', 'D:/Rust/steel-front/assets/triangle.vert.spv']:
    scan(p)