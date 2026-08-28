# -*- coding: utf-8 -*-
import urllib.request, json, re, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}

# PureRef 已知直链模式
for v in ['2.1.1', '2.1.0', '2.0.0']:
    u = f'https://www.pureref.com/releases/PureRef-{v}-x64.exe'
    req = urllib.request.Request(u, headers=h, method='HEAD')
    try:
        r = urllib.request.urlopen(req, timeout=15)
        print('PR OK:', u, r.headers.get('Content-Length', '?'))
        break
    except Exception as e:
        print('PR miss:', v, str(e)[:60])

# Materialize GitHub 搜索（带中文输出正确编码）
try:
    q = urllib.parse.quote('materialize normal map')
    s = json.loads(urllib.request.urlopen(urllib.request.Request(f'https://api.github.com/search/repositories?q={q}', headers=h), timeout=20).read().decode())
    for it in s.get('items', [])[:6]:
        print('MA candidate:', it['full_name'])
except Exception as e:
    print('MA search err', str(e)[:60])

# ArmorPaint 官网页面
try:
    ap = urllib.request.urlopen(urllib.request.Request('https://armorpaint.org/download', headers=h), timeout=20).read().decode(errors='replace')
    lk = re.findall(r'(https?://[^"\' ]+\.zip)', ap)
    print('AP links:', lk[:3])
except Exception as e:
    print('AP err', str(e)[:60])
