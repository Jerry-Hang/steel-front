# -*- coding: utf-8 -*-
import urllib.request, json, re, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}

# PureRef plural /downloads/
for u in ['https://www.pureref.com/downloads/PureRef-2.1.1-x64.exe', 'https://www.pureref.com/download/PureRef-2.1.1-x64.exe', 'https://www.pureref.com/download.php']:
    try:
        r = urllib.request.urlopen(urllib.request.Request(u, headers=h, method='HEAD'), timeout=15)
        print('PR HIT:', u, r.status, r.headers.get('Content-Length'))
    except Exception as e:
        print('PR miss:', u.split('/')[-1][:30], str(e)[:40])
        
# ArmorPaint 21.10 资产
rels = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/repos/armory3d/armorpaint/releases?per_page=6', headers=h), timeout=20).read().decode())
for r in rels:
    for a in r.get('assets', []):
        if 'windows' in a['name'].lower() or a['name'].lower().endswith('.zip'):
            print('AP:', r['tag_name'], a['name'], int(a['size'])//1048576, 'MB')

# Materialize download 目录猜测
for pat in ['Materialize_v1.94.zip', 'Materialize_v1.91.zip', 'Materialize-x64.zip', 'Materialize-1.95.zip']:
    u = 'http://boundingboxsoftware.com/materialize/download/' + pat
    try:
        r = urllib.request.urlopen(urllib.request.Request(u, headers=h, method='HEAD'), timeout=15)
        print('MA HIT:', u, r.status, r.headers.get('Content-Length'))
    except Exception as e:
        pass
