# -*- coding: utf-8 -*-
import urllib.request, re
h = {'User-Agent': 'Mozilla/5.0'}
c = urllib.request.urlopen(urllib.request.Request('https://www.pureref.com/download.php?os=windows', headers=h), timeout=25).read().decode(errors='replace')
m = re.search(r'([^\'\"]*WIN64\.exe)', c)
print('full:', m.group(1) if m else 'none')
print('context:', c[m.start()-80:m.end()+10].replace(chr(10), ' ') if m else '')
