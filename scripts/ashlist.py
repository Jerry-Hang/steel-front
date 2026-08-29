# -*- coding: utf-8 -*-
import urllib.request, json
h = {'User-Agent': 'Mozilla/5.0'}
try:
    d = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/repos/ash-rs/ash/contents/examples/src', headers=h), timeout=20).read().decode())
    print([x['name'] for x in d])
except Exception as e:
    print('ERR', str(e)[:100])
