# -*- coding: utf-8 -*-
import struct, json
names = json.load(open('screenshots/opnames.json'))
b = open('target\\release\\build\\steel-front-2dcbaeb8e9e93c67\\out\\rt_bench.spv', 'rb').read()
out = []
i = 5
while i < len(b)//4:
    insn = struct.unpack('<I', b[i*4:i*4+4])[0]
    wc = insn >> 16
    op = insn & 0xFFFF
    words = [struct.unpack('<I', b[(i+j)*4:(i+j+1)*4])[0] for j in range(wc)]
    nm = names.get(op, f'OP_{op}')
    out.append(f'{nm:42s} ' + ' '.join(str(x) for x in words[1:]))
    i += wc
open('screenshots/dis.txt', 'w', encoding='utf-8').write('\n'.join(out))
print('done')
