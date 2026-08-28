# -*- coding: utf-8 -*-
import urllib.request, json, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}
repos = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/orgs/armory3d/repos?per_page=100', headers=h), timeout=25).read().decode())
for r in repos:
    print(r['name'], '|', r.get('description') or '')