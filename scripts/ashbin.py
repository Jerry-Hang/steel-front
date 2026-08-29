# -*- coding: utf-8 -*-
import urllib.request, json
h = {'User-Agent': 'Mozilla/5.0'}
d = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/repos/ash-rs/ash/contents/ash-examples/src/bin', headers=h), timeout=20).read().decode())
print([x['name'] for x in d])
