from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
for label, t, tol in [('walnut', (189,156,108), 28), ('steel', (172,179,186), 20)]:
    locs = []
    for y in range(0, h, 2):
        for x in range(0, w, 2):
            p = px[x, y]
            if abs(p[0]-t[0])<tol and abs(p[1]-t[1])<tol and abs(p[2]-t[2])<tol:
                locs.append((x, y))
    if locs:
        xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
        print(label, 'n=', len(locs), 'bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    else:
        print(label, 'none')
top_sky = 0; bot_sky = 0
for y in range(100, 500, 4):
    for x in range(0, w, 8):
        p = px[x, y]
        if p[2] > p[0] + 5 and p[0] < 70: top_sky += 1
for y in range(1200, 1550, 4):
    for x in range(0, w, 8):
        p = px[x, y]
        if p[2] > p[0] + 5 and p[0] < 70: bot_sky += 1
print('bluish top:', top_sky, ' bluish bottom:', bot_sky)