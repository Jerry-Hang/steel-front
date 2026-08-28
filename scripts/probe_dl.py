# -*- coding: utf-8 -*-
import urllib.request, json, re
h = {'User-Agent': 'Mozilla/5.0'}

bl = urllib.request.urlopen(urllib.request.Request('https://download.blender.org/release/', headers=h), timeout=20).read().decode()
dirs = re.findall(r'href="(Blender[0-9.]+/)"', bl)
print('blender dirs tail:', dirs[-8:])

ap = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/repos/armory3d/armorpaint/releases/latest', headers=h), timeout=20).read().decode())
print('armorpaint tag:', ap.get('tag_name'))
for a in ap.get('assets', []):
    n = a['name'].lower()
    if 'windows' in n or n.endswith('.zip') or n.endswith('.exe'):
        print('  asset:', a['name'], '|', a['browser_download_url'])
