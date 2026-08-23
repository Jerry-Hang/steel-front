
# -*- coding: utf-8 -*-
import struct, zlib
p = 'screenshots/2-3.png'
d = open(p, 'rb').read()
pos = 8
w = h = None
idat = b''
while pos < len(d):
    ln = struct.unpack('>I', d[pos:pos+4])[0]
    typ = d[pos+4:pos+8]
    data = d[pos+8:pos+8+ln]
    if typ == b'IHDR':
        w, h, bit, color = struct.unpack('>IIBB', data[:10])
    elif typ == b'IDAT':
        idat += data
    pos += 12 + ln
raw = zlib.decompress(idat)
def paeth(a, b, c):
    q = a + b - c
    pa, pb, pc = abs(q-a), abs(q-b), abs(q-c)
    if pa <= pb and pa <= pc: return a
    if pb <= pc: return b
    return c
i = 0
rows = []
prev = bytearray(w * 4)
for y in range(h):
    ft = raw[i]; i += 1
    row = bytearray(raw[i:i+w*4]); i += w*4
    if ft == 1:
        for x in range(4, len(row)): row[x] = (row[x] + row[x-4]) & 255
    elif ft == 2:
        for x in range(len(row)): row[x] = (row[x] + prev[x]) & 255
    elif ft == 3:
        for x in range(len(row)):
            a = row[x-4] if x >= 4 else 0
            row[x] = (row[x] + ((a + prev[x]) >> 1)) & 255
    elif ft == 4:
        for x in range(len(row)):
            a = row[x-4] if x >= 4 else 0
            b = prev[x]
            c = prev[x-4] if x >= 4 else 0
            row[x] = (row[x] + paeth(a, b, c)) & 255
    rows.append(bytes(row))
    prev = row
# 找绿色范围（G 明显高于 R/B 且不太暗）
minx, maxx, miny, maxy = w, 0, h, 0
cnt = 0
for y in range(0, h, 4):
    r = rows[y]
    for x in range(0, w, 4):
        o = x * 4
        R, G, B = r[o], r[o+1], r[o+2]
        if G > R + 18 and G > B + 28 and G > 80:
            if x < minx: minx = x
            if x > maxx: maxx = x
            if y < miny: miny = y
            if y > maxy: maxy = y
            cnt += 1
print('green bbox', minx, maxx, miny, maxy, 'count', cnt)
