# -*- coding: utf-8 -*-
import json, struct, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
b = open(r'C:\Users\Jerry-Huang\Downloads\ak12_-_3d_model_assault_rifle.glb', 'rb').read()
print('magic', b[:4], 'ver', struct.unpack('<I', b[4:8])[0])
jl = struct.unpack('<I', b[12:16])[0]
j = json.loads(b[20:20+jl])
print('meshes:', len(j.get('meshes', [])))
for m in j.get('meshes', []):
    for p in m.get('primitives', []):
        print('  prim attrs:', list(p.get('attributes', {}).keys()), 'indices:', p.get('indices'), 'material:', p.get('material'))
print('materials:', len(j.get('materials', [])))
for mt in j.get('materials', []):
    print('  mat:', mt.get('name'), '| pbr:', json.dumps(mt.get('pbrMetallicRoughness', {}))[:120])
print('images:', len(j.get('images', [])))
for im in j.get('images', []):
    print('  img:', im.get('name'), im.get('mimeType'), 'bv:', im.get('bufferView'))
print('nodes:', len(j.get('nodes', [])))
for n in j.get('nodes', [])[:4]:
    print('  node:', n.get('name'), n.get('translation'), n.get('rotation'), n.get('scale'))
print('scenes:', j.get('scenes'))
print('accessors count:', len(j.get('accessors', [])))
