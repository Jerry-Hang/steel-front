# -*- coding: utf-8 -*-
import urllib.request, re, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}

c = urllib.request.urlopen(urllib.request.Request('https://armorpaint.org/download', headers=h), timeout=20).read().decode(errors='replace')
print('AP raw:', re.findall(r'action="([^"]+)"|href="([^"]+)"', c)[:6])

m = urllib.request.urlopen(urllib.request.Request('http://boundingboxsoftware.com/materialize/downloads.php', headers=h), timeout=20).read().decode(errors='replace')
print('MA links:', re.findall(r'href="([^"]+)"', m)[:12])

p = urllib.request.urlopen(urllib.request.Request('https://silentinstallhq.com/pureref-install-and-uninstall-powershell/', headers=h), timeout=20).read().decode(errors='replace')
print('PR exe:', re.findall(r'[a-zA-Z]+://[^\"\' ]+pure?ref[^\"\' ]+', p)[:4])
