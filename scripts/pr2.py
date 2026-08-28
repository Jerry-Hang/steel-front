# -*- coding: utf-8 -*-
import urllib.request, re
h = {'User-Agent': 'Mozilla/5.0'}
c = urllib.request.urlopen(urllib.request.Request('https://www.pureref.com/download.php?os=windows', headers=h), timeout=25).read().decode(errors='replace')
print('exe refs:', re.findall(r'[\'\"]?[^\'\"]*\.exe', c)[:8])
print('script urls:', re.findall(r'src="([^"]+\.js)"', c)[:5])
print('href tail:', re.findall(r'href="([^"]+)"', c)[-10:])
