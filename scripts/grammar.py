# -*- coding: utf-8 -*-
import urllib.request, json
h = {'User-Agent': 'Mozilla/5.0'}
url = 'https://raw.githubusercontent.com/KhronosGroup/SPIRV-Headers/main/include/spirv/unified1/spirv.core.grammar.json'
d = json.loads(urllib.request.urlopen(urllib.request.Request(url, headers=h), timeout=25).read().decode())
ops = {i['opname']: i['opcode'] for i in d['instructions']}
for k in sorted(ops):
    if 'RayQuery' in k or 'AccelerationStructure' in k and k.startswith('OpType'):
        print(k, '=', ops[k])
