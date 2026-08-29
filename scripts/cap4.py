# -*- coding: utf-8 -*-
import urllib.request, json
h = {'User-Agent': 'Mozilla/5.0'}
url = 'https://raw.githubusercontent.com/KhronosGroup/SPIRV-Headers/main/include/spirv/unified1/spirv.core.grammar.json'
try:
    d = json.loads(urllib.request.urlopen(urllib.request.Request(url, headers=h), timeout=25).read().decode())
    out = []
    for i in d['capabilities']:
        s = json.dumps(i)
        if 'RayQuery' in s:
            out.append(s[:220])
    io = open('screenshots/caps.txt', 'w', encoding='utf-8')
    io.write('\n'.join(out))
    io.close()
    print('saved', len(out))
except Exception as e:
    print('ERR', e)
