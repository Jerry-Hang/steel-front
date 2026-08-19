from PIL import Image
im = Image.open(r'D:\Rust\steel-front\game_shot.png').convert('RGB')
w, h = im.size
print('size:', w, h)
px = im.load()
# sample border strips
def avg(region):
    xs, ys, xe, ye = region
    tot = [0, 0, 0]; n = 0
    for y in range(ys, ye, 4):
        for x in range(xs, xe, 4):
            p = px[x, y]
            tot[0] += p[0]; tot[1] += p[1]; tot[2] += p[2]; n += 1
    return [round(v / n) for v in tot]
print('top strip   :', avg((0, 0, w, 40)))
print('bottom strip:', avg((0, h - 40, w, h)))
print('left strip  :', avg((0, 0, 40, h)))
print('right strip :', avg((w - 40, 0, w, h)))
print('center      :', avg((w // 4, h // 4, 3 * w // 4, 3 * h // 4)))
# find content bbox (pixels not near-black)
minx, miny, maxx, maxy = w, h, 0, 0
for y in range(0, h, 6):
    for x in range(0, w, 6):
        p = px[x, y]
        if p[0] + p[1] + p[2] > 24:
            if x < minx: minx = x
            if x > maxx: maxx = x
            if y < miny: miny = y
            if y > maxy: maxy = y
print('content bbox:', (minx, miny), '-', (maxx, maxy))
print('bbox size:', maxx - minx + 1, 'x', maxy - miny + 1)