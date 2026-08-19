from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def match(p, t, tol): return all(abs(p[i]-t[i]) <= tol for i in range(3))
greens = []; walnuts = []; steels = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[1] > 150 and p[1] > p[0] + 60 and p[1] > p[2] + 60:
            greens.append((x, y, p))
        if match(p, (189, 156, 108), 30):
            walnuts.append((x, y, p))
        if match(p, (172, 179, 186), 25):
            steels.append((x, y, p))
def bbox(locs, label):
    if locs:
        xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
        print(label, 'n=', len(locs), 'bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    else:
        print(label, 'none')
bbox(greens, 'green ')
bbox(walnuts, 'walnut')
bbox(steels, 'steel ')