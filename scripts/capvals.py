# -*- coding: utf-8 -*-
import urllib.request, json
h = {'User-Agent': 'Mozilla/5.0'}
url = 'https://raw.githubusercontent.com/KhronosGroup/SPIRV-Headers/main/include/spirv/unified1/spirv.core.grammar.json'
d = json.loads(urllib.request.urlopen(urllib.request.Request(url, headers=h), timeout=25).read().decode())
found = []
for i in d['capabilities']:
    if isinstance(i, dict):
        cn = i.get('capname') or ''
        if cn in ('RayQueryKHR', 'AccelerationStructureKHR', 'RayTracingKHR'):
            found.append((cn, i.get('value'), i.get('dependencies')))
print(found)
