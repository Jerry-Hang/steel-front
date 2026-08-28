# -*- coding: utf-8 -*-
import json, struct, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
b = open('D:/Rust/steel-front/assets/guns/ak12_baked.glb', 'rb').read()
jl = struct.unpack('<I', b[12:16])[0]
j = json.loads(b[20:20+jl])
bin_start = 20 + jl
blen = struct.unpack('<I', b[bin_start:bin_start+4])[0]
bin = b[bin_start+8:bin_start+8+blen]
a = j['accessors'][3]
bv = j['bufferViews'][a['bufferView']]
off = bv.get('byteOffset', 0) + a.get('byteOffset', 0)
vals = [struct.unpack_from('<f', bin, off + i*4)[0] for i in range(12)]
print('COLOR_0 first 4 verts:', vals)
# 全局统计
import random
random.seed(1)
samples = [struct.unpack_from('<f', bin, off + i*4)[0] for i in random.sample(range(a['count']*3), 300)]
mn = min(samples); mx = max(samples)
print('sample min/max:', mn, mx)