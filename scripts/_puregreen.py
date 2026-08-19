from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# PURE green (0,255,0) with tight tolerance
locs = []
for y in range(0, h, 1):
    for x in range(0, w, 1):
        p = px[x, y]
        if p[1] > 200 and p[0] < 80 and p[2] < 80:
            locs.append((x, y))
print('pure green px:', len(locs))
if locs:
    xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
    print('green bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    print('center:', (sum(xs)//len(xs), sum(ys)//len(ys)))