# -*- coding: utf-8 -*-
import urllib.request, json, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}
try:
    r = json.loads(urllib.request.urlopen(urllib.request.Request('https://api.github.com/repos/rodzill4/material-maker/releases/latest', headers=h), timeout=25).read().decode())
    print('tag:', r.get('tag_name'))
    for a in r.get('assets', []):
        print('  ', a['name'], int(a['size'])//1048576, 'MB')
        print('   ', a['browser_download_url'])
except Exception as e:
    print('ERR', str(e)[:120])
