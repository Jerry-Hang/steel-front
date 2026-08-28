# -*- coding: utf-8 -*-
import urllib.request, json, re, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}

bl = urllib.request.urlopen(urllib.request.Request('https://download.blender.org/release/Blender5.2/', headers=h), timeout=20).read().decode()
files = re.findall(r'href="(blender-5\.2\.1-windows-x64\.([a-z]+))"', bl)
print('blender521:', files)

ap = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/repos/armory3d/armorpaint/releases/latest', headers=h), timeout=20).read().decode())
for a in ap.get('assets', []):
    print('ap:', a['name'], '|', a['size'], '|', a['browser_download_url'])

# Materialize 官方下载页
try:
    mp = urllib.request.urlopen(urllib.request.Request('http://boundingboxsoftware.com/materialize/', headers=h), timeout=20).read().decode(errors='replace')
    links = re.findall(r'href="([^"]*Materialize[^"]*(?:zip|exe|msi))"', mp, re.I)
    print('mat links:', links[:4])
except Exception as e:
    print('mat err', str(e)[:80])

# PureRef 下载页
try:
    pp = urllib.request.urlopen(urllib.request.Request('https://www.pureref.com/download.php', headers=h), timeout=20).read().decode(errors='replace')
    links2 = re.findall(r'href="([^"]+\.exe)"', pp)
    print('pureref links:', links2[:4])
except Exception as e:
    print('pureref err', str(e)[:80])
