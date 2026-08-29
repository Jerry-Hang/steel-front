# -*- coding: utf-8 -*-
import struct, json, glob
names = json.load(open('screenshots/opnames.json'))
b = open(glob.glob('target/release/build/steel-front-*/out/rt_bench.spv')[0], 'rb').read()
insts = []
i = 5
while i < len(b)//4:
    insn = struct.unpack('<I', b[i*4:i*4+4])[0]
    words = [struct.unpack('<I', b[(i+j)*4:(i+j+1)*4])[0] for j in range(insn >> 16)]
    insts.append((insn & 0xFFFF, words))
    i += insn >> 16
# 类型/常量/变量表
types = {}  # id -> (kind, extra)
consts = {}  # id -> (type, value)
vars_ = {}  # id -> (ptr_type, storage)
def prob(v):
    if v == 0: return None
    return types.get(v) or consts.get(v) or vars_.get(v)
for op, w in insts:
    if op == 19: types[w[1]] = ('void',)
    elif op == 20: types[w[1]] = ('bool',)
    elif op == 21: types[w[1]] = ('int', w[2], w[3])
    elif op == 22: types[w[1]] = ('float',)
    elif op == 23: types[w[1]] = ('vec', w[2], w[3])
    elif op == 5341: types[w[1]] = ('accel',)
    elif op == 4472: types[w[1]] = ('rayquery',)
    elif op == 32: types[w[1]] = ('ptr', w[2], w[3])
    elif op == 43: consts[w[1]] = (w[2], w[3])
    elif op == 46: consts[w[1]] = (w[2], 'composite')
    elif op == 59: vars_[w[1]] = (w[2], 'var')
issues = []
def chk(label, vid, expect):
    t = prob(vid)
    if t is None:
        issues.append(f'{label}: %{vid} 未定义')
    else:
        base = t[0] if isinstance(t, tuple) else t
        if isinstance(expect, str) and base != expect:
            issues.append(f'{label}: %{vid} 类型={base} 期望={expect}')
            
for op, w in insts:
    n = names.get(op, 'OP_%d' % op)
    if op == 4473:  # InitializeKHR (words: [0]=ins [1]=rq [2]=accel [3]=flags [4]=mask [5]=origin [6]=tmin [7]=dir [8]=tmax)
        chk('Init.RayQuery', w[1], 'ptr')
        chk('Init.Accel', w[2], 'accel')
        chk('Init.Flags', w[3], 'int')
        chk('Init.Mask', w[4], 'int')
        chk('Init.Origin', w[5], 'vec')
        chk('Init.TMin', w[6], 'float')
        chk('Init.Dir', w[7], 'vec')
        chk('Init.TMax', w[8], 'float')
    elif op == 4477:  # Proceed
        chk('Proceed.RayQuery', w[1], 'ptr')
        # result-type 应为 bool
        if types.get(w[1]) and types[w[1]][0] != 'bool':
            issues.append(f'Proceed 结果类型={types[w[1]][0]} 期望=bool')
    elif op == 4479:  # GetIntersectionType
        chk('GetType.RayQuery', w[3], 'ptr')
        chk('GetType.Intersection', w[4], 'int')
        if types.get(w[1]) and types[w[1]][0] != 'int':
            issues.append(f'GetType 结果类型={types[w[1]][0]} 期望=int')
    elif op == 32:
        pass
print('ISSUES:', len(issues))
for x in issues[:20]: print(' -', x)
