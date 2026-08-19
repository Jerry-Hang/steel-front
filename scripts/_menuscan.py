from PIL import Image
from collections import Counter
im = Image.open(r'D:\Rust\steel-front\menu_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def rc(x0, y0, x1, y1, step=3):
    c = Counter()
    for y in range(y0, y1, step):
        for x in range(x0, x1, step):
            p = px[x, y]
            c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    return c.most_common(4)
# menu title likely center-top area; also whole-image bright text count
print('center  :', rc(w//2-400, h//2-200, w//2+400, h//2+200))
print('top     :', rc(0, 0, w, 300))
print('bottom  :', rc(0, h-300, w, h))
# bright pixel count whole image (text is white/bright on dark)
bright = 0
for y in range(0, h, 4):
    for x in range(0, w, 4):
        p = px[x, y]
        if p[0]+p[1]+p[2] > 450: bright += 1
print('bright px whole:', bright)
# where are bright pixels located?
locs = []
for y in range(0, h, 4):
    for x in range(0, w, 4):
        p = px[x, y]
        if p[0]+p[1]+p[2] > 450:
            locs.append((x, y, p))
print('first 10 bright:', locs[:10])
if locs:
    xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
    print('bright bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)), 'count:', len(locs))