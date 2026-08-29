# -*- coding: utf-8 -*-
import struct, urllib.request, json
# 取 op 名称表
h = {'User-Agent': 'Mozilla/5.0'}
url = 'https://raw.githubusercontent.com/KhronosGroup/SPIRV-Headers/main/include/spirv/unified1/spirv.core.grammar.json'
try:
    d = json.loads(urllib.request.urlopen(urllib.request.Request(url, headers=h), timeout=25).read().decode())
    names = {i['opcode']: i['opname'] for i in d.get('instructions', [])}
    io = open('screenshots/opnames.json', 'w')
    import json as j
    io.write(j.dumps(names))
    io.close()
    b = open(r'D:\Rust\steel-front\assets\rt_bench.spv', 'rb').read() if False else None
    print('table saved', len(names))
except Exception as e:
    print('ERR', e)
