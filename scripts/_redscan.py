from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def match(p, t, tol): return all(abs(p[i]-t[i]) <= tol for i in range(3))
red = (242, 31, 20)
locs = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if match(p, red, 60) and p[0] > p[1] + 60:
            locs.append((x, y, p))
print('red px:', len(locs))
if locs:
    xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
    print('red bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    print('samples:', locs[:5])