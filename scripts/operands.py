# -*- coding: utf-8 -*-
import urllib.request, json
h = {'User-Agent': 'Mozilla/5.0'}
url = 'https://raw.githubusercontent.com/KhronosGroup/SPIRV-Headers/main/include/spirv/unified1/spirv.core.grammar.json'
d = json.loads(urllib.request.urlopen(urllib.request.Request(url, headers=h), timeout=25).read().decode())
for i in d['instructions']:
    n = i['opname']
    if n in ('OpRayQueryInitializeKHR', 'OpRayQueryProceedKHR', 'OpRayQueryGetIntersectionTypeKHR'):
        print(n, 'opcode', i['opcode'])
        for op in i.get('operands', []):
            print('   ', op.get('kind'), op.get('name'))
