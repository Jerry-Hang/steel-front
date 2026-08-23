
# -*- coding: utf-8 -*-
import re
lines = open('data/battle_log.txt', encoding='utf-8', errors='replace').read().splitlines()
samples = []
for l in lines:
    m = re.search(r'red=(\d+) blue=(\d+) ra=(\d+) ba=(\d+)', l)
    if m:
        samples.append((int(m.group(1)), int(m.group(2)), int(m.group(3)), int(m.group(4))))
print('samples', len(samples))
# 取最后一个 red>=128 开局后的 160s
start = -1
for i in range(len(samples) - 1, -1, -1):
    if samples[i][0] >= 128 and samples[i][1] >= 127:
        start = i
        break
for i in range(0, min(160, len(samples) - start), 4):
    t = samples[start + i]
    print(i, 'red', t[0], 'blue', t[1], 'ra', t[2], 'ba', t[3])
