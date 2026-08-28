# -*- coding: utf-8 -*-
import json, struct, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
b = open(r'D:\Rust\steel-front\assets\guns\ak12_baked.glb', 'rb').read()
jl = struct.unpack('<I', b[12:16])[0]
j = json.loads(b[20:20+jl])
for i, m in enumerate(j.get('meshes', [])):
    for p in m.get('primitives', []):
        print('mesh', i, 'attrs:', {k: (v, j['accessors'][v]['componentType'], j['accessors'][v]['type'], j['accessors'][v].get('count')) for k, v in p.get('attributes', {}).items()})
        print('  indices acc:', p.get('indices'), j['accessors'][p['indices']]['componentType'] if p.get('indices') else None)
        print('  material:', p.get('material'), 'mode:', p.get('mode'))
print('nodes:', [(n.get('name'), n.get('translation'), n.get('rotation'), n.get('scale')) for n in j.get('nodes', [])[:8]])
print('total buffers:', j.get('buffers'))
