# -*- coding: utf-8 -*-
import urllib.request, re
h = {'User-Agent': 'Mozilla/5.0'}
c = urllib.request.urlopen(urllib.request.Request('https://armorpaint.org/download', headers=h), timeout=25).read().decode(errors='replace')
# 所有 href + onclick + 形似文件的路径
print('hrefs:', re.findall(r'href="([^"]+)"', c))
print('onclick:', re.findall(r'onclick="([^"]+)"', c)[:5])
print('js files:', re.findall(r'src="([^"]+)"', c)[:8])
