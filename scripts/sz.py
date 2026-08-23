# -*- coding: utf-8 -*-
import struct, os
def png_size(p):
    with open(p, 'rb') as f:
        f.read(16)
        w, h = struct.unpack('>II', f.read(8))
    return w, h
for t in ['1-1.png', '1-2.png', '1-3.png', 'AK-12M.png']:
    p = 'D:/Rust/steel-front/screenshots/' + t
    print(t, png_size(p), os.path.getsize(p))