# -*- coding: utf-8 -*-
import urllib.request
h = {'User-Agent': 'Mozilla/5.0', 'Referer': 'https://www.pureref.com/download.php'}
r = urllib.request.urlopen(urllib.request.Request('https://www.pureref.com/download.php?file=PureRef-2.0.3-x64.exe', headers=h), timeout=120)
d = r.read()
open(r'D:\3D_Work\PureRef-2.0.3-x64.exe', 'wb').write(d)
print('len', len(d), 'head', d[:2], 'type', r.headers.get('Content-Type'))
