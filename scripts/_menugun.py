from PIL import Image
im = Image.open('D:/Rust/steel-front/menu_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def match(p, t, tol): return all(abs(p[i]-t[i]) <= tol for i in range(3))
for target, tol, label in [
    ((133, 87, 38), 60, 'walnut'),
    ((97, 61, 26), 50, 'walnut_dark'),
    ((107, 117, 128), 30, 'steel'),
]:
    locs = []
    for y in range(h//2, h, 3):
        for x in range(0, w, 3):
            p = px[x, y]
            if match(p, target, tol): locs.append((x, y))
    if locs:
        xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
        print(label, 'n=', len(locs), 'bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    else:
        print(label, 'none')