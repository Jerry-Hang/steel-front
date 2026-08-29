# -*- coding: utf-8 -*-
import urllib.request, json
h = {'User-Agent': 'Mozilla/5.0'}
url = 'https://raw.githubusercontent.com/KhronosGroup/SPIRV-Headers/main/include/spirv/unified1/spirv.core.grammar.json'
d = json.loads(urllib.request.urlopen(urllib.request.Request(url, headers=h), timeout=25).read().decode())
# capabilities + types
for attr in ['capabilities', 'types', 'instructions']:
    for i in d.get(attr, []):
        n = i.get('opname') or i.get('type') or i.get('capname')
        if n and ('RayQuery' in n or 'RayTracing' in n or 'Acceleration' in n):
            print(attr[:3], n, '=', i.get('value', i.get('opcode')))
