# -*- coding: utf-8 -*-
import urllib.request
h = {'User-Agent': 'Mozilla/5.0'}
for u in ['https://www.pureref.com/downloads/PureRef-2.0.3-x64.exe', 'https://www.pureref.com/download.php?file=PureRef-2.0.3-x64.exe', 'https://www.pureref.com/downloads/PureRef-2.0.3-WIN64.exe']:
    try:
        r = urllib.request.urlopen(urllib.request.Request(u, headers=h, method='HEAD'), timeout=15)
        print('HIT', u, r.status, r.headers.get('Content-Length'))
    except Exception as e:
        print('miss', u.split('/')[-1][:36], str(e)[:40])
