# -*- coding: utf-8 -*-
import urllib.request, json, re, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}

ap = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/repos/armory3d/armorpaint/releases/latest', headers=h), timeout=20).read().decode())
for a in ap.get('assets', []):
    print('AP:', a['name'], int(a['size'])//1048576, 'MB')

try:
    pp = urllib.request.urlopen(urllib.request.Request('https://www.pureref.com/releases/', headers=h), timeout=20).read().decode(errors='replace')
    fl = re.findall(r'href="([^"]+\.exe)"', pp)
    print('PR releases:', fl[:4])
except Exception as e:
    print('PR err', str(e)[:80])

# Materialize 官方页直接找（全链接）
try:
    mp = urllib.request.urlopen(urllib.request.Request('http://boundingboxsoftware.com/materialize/', headers=h), timeout=20).read().decode(errors='replace')
    links = re.findall(r'href="([^"]+)"', mp)
    dl = [l for l in links if 'zip' in l.lower() or 'download' in l.lower() or 'exe' in l.lower()]
    print('MA raw:', dl[:6])
except Exception as e:
    print('MA err', str(e)[:80])
