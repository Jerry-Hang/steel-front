# -*- coding: utf-8 -*-
import urllib.request, json, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}
rels = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/repos/armory3d/armorpaint/releases?per_page=15', headers=h), timeout=25).read().decode())
for r in rels:
    assets = r.get('assets', [])
    if assets:
        print('##', r['tag_name'], len(assets), 'assets')
        for a in assets:
            print('  ', a['name'], int(a['size'])//1048576, 'MB')
    else:
        print('##', r['tag_name'], '(no assets)')
