
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
print('size', w, 'x', h)
px = im.load()
pts = [
    ('corner', 50, 50),
    ('center', 1280, 800),
    ('mid', 1280, 590),
    ('mid2', 1280, 1010),
    ('top', 1280, 146),
    ('bottom', 1280, 1440),
]
for label, x, y in pts:
    p = px[x, y]
    print('  %-10s (%4d,%4d) = (%3d,%3d,%3d)' % (label, x, y, p[0], p[1], p[2]))
from collections import Counter
c = Counter()
for y in range(0, h, 8):
    for x in range(0, w, 8):
        p = px[x, y]
        c[(p[0]//64, p[1]//64, p[2]//64)] += 1
print('top colors:')
for k, v in c.most_common(6):
    print('  ~rgb', (k[0]*64+32, k[1]*64+32, k[2]*64+32), 'count', v)
