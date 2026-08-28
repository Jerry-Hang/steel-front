# -*- coding: utf-8 -*-
import json, struct, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
b = open('D:/Rust/steel-front/assets/guns/ak12_baked.glb', 'rb').read()
jl = struct.unpack('<I', b[12:16])[0]
j = json.loads(b[20:20+jl])
for i, a in enumerate(j.get('accessors', [])):
    print(i, a.get('type'), a.get('componentType'), 'count', a.get('count'), 'min', a.get('min'), 'max', a.get('max'))