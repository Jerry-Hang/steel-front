
# -*- coding: utf-8 -*-
import re
lines = open('data/battle_log.txt', encoding='utf-8', errors='replace').read().splitlines()
samples = []
for l in lines:
    m = re.search(r'npcs=(\d+) .*?red=(\d+) blue=(\d+)', l)
    if m:
        samples.append((int(m.group(1)), int(m.group(2)), int(m.group(3))))
start = 357
seg = samples[start:start + 116]
for i in range(0, len(seg), 4):
    t = seg[i]
    print(i, 'red', t[1], 'blue', t[2])
