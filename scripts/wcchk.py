# -*- coding: utf-8 -*-
import struct, json, glob, urllib.request
names = json.load(open('screenshots/opnames.json'))
# grammar 操作数（尽力缓存）
try:
    g = json.load(open('screenshots/grammar.json'))
except Exception:
    h = {'User-Agent': 'Mozilla/5.0'}
    g = json.loads(urllib.request.urlopen(urllib.request.Request('https://raw.githubusercontent.com/KhronosGroup/SPIRV-Headers/main/include/spirv/unified1/spirv.core.grammar.json', headers=h), timeout=20).read().decode())
    json.dump(g, open('screenshots/grammar.json', 'w'))
opcount = {}
basecount = {}
for i in g['instructions']:
    ops = i.get('operands', [])
    base = 0
    var = False
    for o in ops:
        k = o.get('kind', '')
        if k == 'IdRef' or k == 'IdResult' or k == 'IdResultType':
            base += 1
        elif 'Literal' in k or k in ('PairLiteralIntegerIdRef', 'PairIdRefLiteralInteger', 'PairLiteralIntegerIdRef'):
            base += 1
        else:
            var = True
            base += 1
    opcount[i['opcode']] = (base, var)
b = open(glob.glob('target/release/build/steel-front-*/out/rt_bench.spv')[0], 'rb').read()
i = 5
lines = []
n = 0
while i < len(b)//4:
    insn = struct.unpack('<I', b[i*4:i*4+4])[0]
    wc = insn >> 16
    op = insn & 0xFFFF
    base, var = opcount.get(op, (0, False))
    if wc - 1 < base:
        lines.append('SHORT #%d op=%d wc=%d base=%d' % (n, op, wc, base))
    elif var and wc - 1 > base:
        lines.append('EXTRA? #%d op=%d wc=%d base=%d var' % (n, op, wc, base))
    elif not var and wc - 1 > base:
        lines.append('EXTRA #%d op=%d(%s) wc=%d base=%d' % (n, op, names.get(str(op), '?'), wc, base))
    i += wc
    n += 1
print(chr(10).join(lines[:15]))
print('checked', n)
