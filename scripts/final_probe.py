# -*- coding: utf-8 -*-
import urllib.request, json, re, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}
def head(u):
    try:
        r = urllib.request.urlopen(urllib.request.Request(u, headers=h, method='HEAD'), timeout=15)
        return r.status, r.headers.get('Content-Length')
    except Exception as e:
        return None, str(e)[:40]
for u in ['https://www.pureref.com/download.php?os=windows', 'https://armorpaint.org/downloads/ArmorPaint-23.08-windows.zip', 'https://armorpaint.org/downloads/ArmorPaint-21.10-windows.zip', 'http://boundingboxsoftware.com/materialize/download/materialize-1.95.zip', 'http://boundingboxsoftware.com/materialize/download/Materialize-1.95.zip', 'http://boundingboxsoftware.com/materialize/download/Materialize-1.94.zip']:
    st, ln = head(u)
    print(u.split('|')[-1][-45:], '->', st, ln)
