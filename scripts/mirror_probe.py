# -*- coding: utf-8 -*-
import urllib.request, sys, time
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
u = 'https://github.com/RodZill4/material-maker/releases/download/1.7/material_maker_1_7_windows.zip'
mirrors = ['https://gh-proxy.com/', 'https://ghfast.top/', 'https://ghproxy.net/', 'https://gh.llkk.cc/', 'https://gh.mihoyo.ovh/']
h = {'User-Agent': 'Mozilla/5.0'}
for m in mirrors:
    t0 = time.time()
    try:
        r = urllib.request.urlopen(urllib.request.Request(m + u, headers=h), timeout=12)
        d = r.read(65536)
        print(m, 'OK', len(d), 'bytes in', round(time.time()-t0, 1), 's')
        break
    except Exception as e:
        print(m, 'fail', str(e)[:50])
