# -*- coding: utf-8 -*-
import urllib.request, re, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0', 'Referer': 'https://armorpaint.org/download'}

def links(u, pat):
    try:
        c = urllib.request.urlopen(urllib.request.Request(u, headers=h), timeout=20).read().decode(errors='replace')
        return re.findall(pat, c)[:8], len(c)
    except Exception as e:
        return ['ERR ' + str(e)[:60]], 0

a, an = links('https://armorpaint.org/download', r'href="([^"]+)"')
print('AP page:', an, 'links:', [x for x in a if 'zip' in x.lower() or 'win' in x.lower() or 'download' in x.lower()][:6])

p, pn = links('https://silentinstallhq.com/pureref-install-and-uninstall-powershell/', r'https?://[^"\' ]+pureref[^"\' ]*\.exe')
print('PR guide:', p[:3])

m, mn = links('http://boundingboxsoftware.com/materialize/downloads.php', r'(?:href="|\b)([^"]*Materialize[^"]*)"')
print('MA page:', mn, ':', m[:5])
