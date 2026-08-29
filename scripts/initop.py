# -*- coding: utf-8 -*-
import urllib.request, json
h = {'User-Agent': 'Mozilla/5.0'}
url = 'https://raw.githubusercontent.com/KhronosGroup/SPIRV-Headers/main/include/spirv/unified1/spirv.core.grammar.json'
try:
    d = json.loads(urllib.request.urlopen(urllib.request.Request(url, headers=h), timeout=20).read().decode())
    for i in d['instructions']:
        if i['opname'] == 'OpRayQueryInitializeKHR':
            for op in i['operands']:
                print(op)
            break
except Exception as e:
    print('ERR', e)
