from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# scan FULL screen for walnut/steel
for label, t, tol in [('walnut', (189,156,108), 28), ('steel', (172,179,186), 20), ('walnut_dark', (164,133,90), 25)]:
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