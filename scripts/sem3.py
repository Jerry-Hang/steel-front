# -*- coding: utf-8 -*-
import struct, json, glob
names = json.load(open('screenshots/opnames.json'))
b = open(glob.glob('target/release/build/steel-front-*/out/rt_bench.spv')[0], 'rb').read()
insts = []
i = 5
while i < len(b)//4:
    insn = struct.unpack('<I', b[i*4:i*4+4])[0]
    words = [struct.unpack('<I', b[(i+j)*4:(i+j+1)*4])[0] for j in range(insn >> 16)]
    insts.append((insn & 0xFFFF, words[1:]))
    i += insn >> 16
types = {}
consts = {}
vars_ = {}
results = {}
for op, w in insts:
    if op == 19: types[w[0]] = ('void',)
    elif op == 20: types[w[0]] = ('bool',)
    elif op == 21: types[w[0]] = ('int',)
    elif op == 22: types[w[0]] = ('float',)
    elif op == 23: types[w[0]] = ('vec', w[2])
    elif op == 5341: types[w[0]] = ('accel',)
    elif op == 4472: types[w[0]] = ('rayquery',)
    elif op == 32: types[w[0]] = ('ptr', w[1], w[2])
    elif op == 43: consts[w[0]] = (w[1], 'const')
    elif op == 46: consts[w[0]] = (w[1], 'composite')
    elif op == 59: vars_[w[0]] = (w[1], 'var')
    elif op == 61: results[w[0]] = w[1]
    elif op == 186: results[w[0]] = w[1]
    elif op == 111: results[w[0]] = w[1]
    elif op == 80: results[w[0]] = w[1]
    elif op == 171: results[w[0]] = w[1]
    elif op == 65: results[w[0]] = w[1]
    elif op == 4477: results[w[1]] = w[0]
    elif op == 4479: results[w[1]] = w[0]

def tid(vid):
    if vid in results: return results[vid]
    if vid in consts: return consts[vid][0]
    if vid in vars_: return vars_[vid][0]
    return None
def kind(vid):
    t = tid(vid)
    if t is None: return 'UNDEF'
    k = types.get(t)
    if k is None: return 'UNK' + str(t)
    return k[0]
def pointee(vid):
    t = tid(vid)
    if t is not None and types.get(t) and types[t][0] == 'ptr':
        return types[t][2]
    return None
issues = []
def expect(label, vid, want):
    k = kind(vid)
    if k == 'UNDEF': issues.append(label + ': %' + str(vid) + ' 未定义')
    elif k != want: issues.append(label + ': %' + str(vid) + ' 种类=' + k + ' 期望=' + want)
def expectptr(label, vid, pk):
    k = kind(vid)
    if k == 'UNDEF': issues.append(label + ': %' + str(vid) + ' 未定义'); return
    if k != 'ptr': issues.append(label + ': %' + str(vid) + ' 非指针'); return
    p = pointee(vid)
    tk = types.get(p) if p is not None else None
    if tk is None or tk[0] != pk:
        issues.append(label + ': %' + str(vid) + ' 指向=' + (str(tk) if tk else '?') + ' 期望=' + pk)
for op, w in insts:
    if op == 4473:
        expectptr('Init.RQ', w[0], 'rayquery')
        expect('Init.Accel', w[1], 'accel')
        expect('Init.Flags', w[2], 'int')
        expect('Init.Mask', w[3], 'int')
        expect('Init.Origin', w[4], 'vec')
        expect('Init.TMin', w[5], 'float')
        expect('Init.Dir', w[6], 'vec')
        expect('Init.TMax', w[7], 'float')
    elif op == 4477:
        expectptr('Proceed.RQ', w[2], 'rayquery')
    elif op == 4479:
        expectptr('GetType.RQ', w[2], 'rayquery')
        expect('GetType.X', w[3], 'int')
print('ISSUES:', len(issues))
for x in issues[:20]: print(' -', x)
