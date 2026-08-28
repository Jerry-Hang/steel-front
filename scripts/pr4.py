# -*- coding: utf-8 -*-
import urllib.request, re
h = {'User-Agent': 'Mozilla/5.0'}
c = urllib.request.urlopen(urllib.request.Request('https://www.pureref.com/download.php?os=windows', headers=h), timeout=25).read().decode(errors='replace')
# 找 select 块内的 option
opts = re.findall(r'<option value="([^"]+)">', c)
print('options:', opts[:10])
