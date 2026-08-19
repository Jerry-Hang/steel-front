from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def green_count(x0, y0, x1, y1):
    n = 0; rows = {}
    for y in range(y0, y1):
        c = 0
        for x in range(x0, x1, 2):
            p = px[x, y]
            if p[1] > 120 and p[1] > p[0] + 30 and p[1] > p[2] + 30:
                c += 1
        if c > 3: rows[y] = c
        n += c
    return n, rows
# HP bar x-range: 48..768 (design 24..384 x2)
n1, r1 = green_count(48, 20, 768, 200)
n2, r2 = green_count(48, 1400, 768, 1600)
print('green top  (48-768, y20-200):', n1, 'rows:', list(r1.items())[:5])
print('green bottom(48-768, y1400-1600):', n2, 'rows:', list(r2.items())[:5])
# where are the widest green rows overall?
best = []
for y in range(0, h, 2):
    c = 0
    for x in range(0, 1200, 3):
        p = px[x, y]
        if p[1] > 120 and p[1] > p[0] + 30 and p[1] > p[2] + 30:
            c += 1
    if c > 20: best.append((y, c))
print('wide green rows:', best[:10])