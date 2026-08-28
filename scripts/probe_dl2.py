# -*- coding: utf-8 -*-
import urllib.request, json, re
h = {'User-Agent': 'Mozilla/5.0'}

bl = urllib.request.urlopen(urllib.request.Request('https://download.blender.org/release/Blender5.2/', headers=h), timeout=20).read().decode()
files = re.findall(r'href="([^"]+\.(?:msi|zip))"', bl)
print('blender52:', files[:6])

ap = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/repos/armory3d/armorpaint/releases/latest', headers=h), timeout=20).read().decode())
for a in ap.get('assets', []):
    print('ap asset:', a['name'], '|', a['browser_download_url'])

# Materialize 正确仓库（搜索）
try:
    s = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/search/repositories?q=Materialize+software', headers=h), timeout=20).read().decode())
    for it in s.get('items', [])[:5]:
        print('mat candidate:', it['full_name'], it.get('description', '')[:60])
except Exception as e:
    print('mat search err', e)
