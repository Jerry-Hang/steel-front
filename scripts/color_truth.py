# -*- coding: utf-8 -*-
import json, struct, sys, os
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
base = 'D:/Rust/steel-front/assets/guns/'
for name in ['ak12.glb', 'ak12_baked.glb']:
    b = open(base + name, 'rb').read()
    jl = struct.unpack('<I', b[12:16])[0]
    j = json.loads(b[20:20+jl])
    print('===', name)
    for m in j.get('materials', []):
        pbr = m.get('pbrMetallicRoughness', {})
        print(' mat:', m.get('name'), pbr.get('baseColorFactor'), 'metal', pbr.get('metallicFactor'))
    for i, a in enumerate(j.get('accessors', [])):
        mn = a.get('min')
        if mn and len(mn) >= 3 and -0.01 < mn[0] < 0.6:
            print(' acc', i, a.get('type'), 'min', a.get('min'), 'max', a.get('max'))