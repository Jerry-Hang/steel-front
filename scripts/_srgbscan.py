from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def match(p, t, tol): return all(abs(p[i]-t[i]) <= tol for i in range(3))
# sRGB conversions: linear^0.4545
def srgb(c): return tuple(round(255 * (v ** 0.4545)) for v in c)
walnut = srgb((0.52, 0.34, 0.15))
walnut_dark = srgb((0.38, 0.24, 0.10))
steel = srgb((0.42, 0.46, 0.50))
dark = srgb((0.22, 0.22, 0.24))
print('targets:', walnut, walnut_dark, steel, dark)
for label, t, tol in [('walnut', walnut, 30), ('walnut_dark', walnut_dark, 30), ('steel', steel, 25), ('dark', dark, 20)]:
    locs = []
    for y in range(h//2, h, 2):
        for x in range(0, w, 2):
            p = px[x, y]
            if match(p, t, tol): locs.append((x, y, p))
    if locs:
        xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
        print(label, 'n=', len(locs), 'bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    else:
        print(label, 'none')