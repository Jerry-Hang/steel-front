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
    # OpFMul (109) with a constant that is negative float
    fmul_neg = 0
    for o, a in ops:
        if o == 109:
            # a = (result_type, result_id, op1, op2); op2 might be constant id - resolve later
            fmul_neg += 1
    fsub_zero = sum(1 for o, a in ops if o == 107)
    print(path.split('/')[-1], 'FNegate:', fneg, 'FMul:', fmul_neg, 'FSub:', fsub_zero)
for p in ['D:/Rust/steel-front/assets/triangle.vert.spv', 'D:/Rust/steel-front/assets/mesh.spv', 'D:/Rust/steel-front/assets/shadow.vert.spv']:
    scan(p)