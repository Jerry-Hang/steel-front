# -*- coding: utf-8 -*-
import json, struct, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
b = open(r'D:\Rust\steel-front\assets\guns\ak12_baked.glb', 'rb').read()
jl = struct.unpack('<I', b[12:16])[0]
j = json.loads(b[20:20+jl])
bin_start = 20 + jl
blen = struct.unpack('<I', b[bin_start:bin_start+4])[0]
bin = b[bin_start+8:bin_start+8+blen]
# mesh 0/1 的 COLOR_0 accessor
for mi, m in enumerate(j.get('meshes', [])):
    for p in m.get('primitives', []):
        col_acc = p.get('attributes', {}).get('COLOR_0')
        if col_acc is not None:
            a = j['accessors'][col_acc]
            bv = j['bufferViews'][a['bufferView']]
            off = bv.get('byteOffset', 0) + a.get('byteOffset', 0)
            vals = [struct.unpack_from('<f', bin, off + i*4)[0] for i in range(9)]
            print('mesh', mi, 'COLOR_0 type', a.get('type'), 'comp', a.get('componentType'), 'first vals', vals)
