# -*- coding: utf-8 -*-
import json, struct, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
b = open(r'D:\Rust\steel-front\assets\guns\ak12.glb', 'rb').read()
jl = struct.unpack('<I', b[12:16])[0]
j = json.loads(b[20:20+jl])
for i, n in enumerate(j.get('nodes', [])):
    print(i, n.get('name', '?'), '| mesh:', n.get('mesh'), '| T:', n.get('translation'), '| R:', n.get('rotation'), '| S:', n.get('scale'))
print('buffers:', j.get('buffers'))
