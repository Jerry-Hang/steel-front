from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# gun expected bottom-center-right: anchor (0.28,-0.30,-0.62) view space; gun parts spread
# scan wide bottom area for walnut brown (132,87,38)*255-ish and steel (107,117,128)
def match(p, target, tol=40):
    return all(abs(p[i]-target[i]) <= tol for i in range(3))
walnut = (133, 87, 38)
steel = (107, 117, 128)
wn = sn = 0
for y in range(h-420, h-40, 2):
    for x in range(w//2-500, w//2+700, 2):
        p = px[x, y]
        if match(p, walnut, 45): wn += 1
        elif match(p, steel, 35): sn += 1
print('walnut px:', wn, ' steel px:', sn)
# where are walnut pixels?
locs = []
for y in range(h-420, h-40, 3):
    for x in range(w//2-500, w//2+700, 3):
        p = px[x, y]
        if match(p, walnut, 45): locs.append((x, y))
if locs:
    xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
    print('walnut bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)), 'n=', len(locs))
else:
    print('no walnut found')