from PIL import Image
from collections import Counter
def bands(name):
    im = Image.open('D:/Rust/steel-front/' + name + '.png').convert('RGB')
    w, h = im.size
    px = im.load()
    rows = {}
    for y in range(0, h, 8):
        n = 0
        for x in range(0, w, 8):
            p = px[x, y]
            if p[0]+p[1]+p[2] > 450: n += 1
        if n > 0: rows[y] = n
    if rows:
        ys = sorted(rows)
        b = []
        cur = [ys[0], ys[0]]
        for y in ys[1:]:
            if y - cur[1] > 40: b.append(tuple(cur)); cur = [y, y]
            else: cur[1] = y
        b.append(tuple(cur))
        print(name, 'bands:', b)
    else:
        print(name, 'NO bright rows')
bands('menu2_shot')
bands('game2_shot')
# HP bar green in game2: top vs bottom
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def green_count(y0, y1):
    n = 0; ys = []
    for y in range(y0, y1):
        c = 0
        for x in range(48, 768, 2):
            p = px[x, y]
            if p[1] > 120 and p[1] > p[0] + 30 and p[1] > p[2] + 30: c += 1
        if c > 3: ys.append((y, c))
        n += c
    return n, ys[:4]
n1, y1 = green_count(20, 200)
n2, y2 = green_count(1400, 1600)
print('HP green top:', n1, y1)
print('HP green bottom:', n2, y2)