# -*- coding: utf-8 -*-
import urllib.request, re
h = {'User-Agent': 'Mozilla/5.0'}
for u in ['https://armorpaint.org/download.php', 'https://armorpaint.org/download.php?os=win64', 'https://armorpaint.org/downloads/ArmorPaint-23.08-win64.zip']:
    try:
        r = urllib.request.urlopen(urllib.request.Request(u, headers=h, method='HEAD'), timeout=15)
        ct = r.headers.get('Content-Type', '')
        print(u, '->', r.status, ct, r.headers.get('Content-Length'))
    except Exception as e:
        print(u.split('/')[-1][:44], 'miss', str(e)[:40])
