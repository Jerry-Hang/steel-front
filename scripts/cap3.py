# -*- coding: utf-8 -*-
import urllib.request, json
h = {'User-Agent': 'Mozilla/5.0'}
url = 'https://raw.githubusercontent.com/KhronosGroup/SPIRV-Headers/main/include/spirv/unified1/spirv.core.grammar.json'
d = json.loads(urllib.request.urlopen(urllib.request.Request(url, headers=h), timeout=25).read().decode())
print(json.dumps(d['capabilities'][:3])[:400])
# RayQuery 直接
for i in d['capabilities']:
    s = json.dumps(i)
    if 'RayQuery' in s:
        print(s[:200])
