# -*- coding: utf-8 -*-
import urllib.request, json, re, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}

rels = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/repos/armory3d/armorpaint/releases?per_page=3', headers=h), timeout=20).read().decode())
for r in rels[:2]:
    print('rel:', r['tag_name'], 'assets:', len(r.get('assets', [])))
    for a in r.get('assets', []):
        print('  ', a['name'], int(a['size'])//1048576, 'MB')

try:
    pp = urllib.request.urlopen(urllib.request.Request('https://www.pureref.com/download.php', headers=h), timeout=20).read().decode(errors='replace')
    fl = re.findall(r'(https?://[^"\'; ]+.exe)', pp)
    print('PR exe links:', fl[:4])
except Exception as e:
    print('PR err', str(e)[:80])

try:
    mp = urllib.request.urlopen(urllib.request.Request('http://boundingboxsoftware.com/materialize/downloads.php', headers=h), timeout=20).read().decode(errors='replace')
    fl2 = re.findall(r'(https?://[^"\'; ]+(?:zip|exe|msi))', mp)
    print('MA links:', fl2[:4])
except Exception as e:
    print('MA err', str(e)[:80])
