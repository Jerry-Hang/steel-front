# -*- coding: utf-8 -*-
import struct, zlib
p = 'screenshots/steel_front_20260821_202523.png'
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
    p = a + b - c
    pa, pb, pc = abs(p-a), abs(p-b), abs(p-c)
    if pa <= pb and pa <= pc: return a
    if pb <= pc: return b
    return c
stride = w * 4 + 1
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
bg = rows[5][5*4:5*4+3]
print('bg', bg)
minx, maxx, miny, maxy = w, 0, h, 0
for y in range(0, h, 3):
    r = rows[y]
    for x in range(0, w, 3):
        o = x*4
        dpx = abs(r[o]-bg[0]) + abs(r[o+1]-bg[1]) + abs(r[o+2]-bg[2])
        if dpx > 30:
            if x < minx: minx = x
            if x > maxx: maxx = x
            if y < miny: miny = y
            if y > maxy: maxy = y
print('bbox', minx, maxx, miny, maxy, 'w', maxx-minx, 'h', maxy-miny)
print('center', (minx+maxx)/2, 'img_center', w/2)
