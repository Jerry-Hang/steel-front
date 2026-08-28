# -*- coding: utf-8 -*-
import urllib.request, re, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
h = {'User-Agent': 'Mozilla/5.0'}
c = urllib.request.urlopen(urllib.request.Request('https://rodzill4.itch.io/material-maker', headers=h), timeout=25).read().decode(errors='replace')
print('len', len(c))
print('upload_id:', re.findall(r'data-upload_id="?([0-9]+)', c)[:3])
print('build_id:', re.findall(r'data-build_id="?([0-9]+)', c)[:3])
print('download urls:', re.findall(r'(https?://[^"\' ]*download[^"\' ]*)', c)[:4])
print('file names:', re.findall(r'(?<=>)([^<>"]*\.(?:zip|exe|7z|rar))', c)[:8])
print('want:', re.findall(r'data-want[^>]*', c)[:2])
