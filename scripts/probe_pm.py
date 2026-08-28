# -*- coding: utf-8 -*-
import urllib.request, re
h = {'User-Agent': 'Mozilla/5.0'}
c = urllib.request.urlopen(urllib.request.Request('https://www.pureref.com/download.php', headers=h), timeout=25).read().decode(errors='replace')
print('PR forms:', re.findall(r'<form[^>]*>', c)[:3])
print('PR dl anchors:', re.findall(r'<a[^>]*href="([^"]+)"[^>]*>[^<]*Download[^<]*</a>', c, re.I)[:4])
m = urllib.request.urlopen(urllib.request.Request('http://boundingboxsoftware.com/materialize/downloads.php', headers=h), timeout=25).read().decode(errors='replace')
print('MA hrefs:', re.findall(r'href="([^"]+)"', m)[:20])
