from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def rc(x0, y0, x1, y1, step=2):
    c = Counter()
    for y in range(y0, y1, step):
        for x in range(x0, x1, step):
            p = px[x, y]
            c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    return c.most_common(4)
# HP bar now expected at (48,1552) 720x44
print('HP bar   :', rc(48, 1550, 780, 1600))
# debug text at (20,88) scale 2.6
print('debug txt:', rc(15, 80, 500, 130))
# crosshair at center (1280,800)
print('center   :', rc(1240, 760, 1320, 840))
n = 0
for y in range(h//2-60, h//2+60):
    for x in range(w//2-60, w//2+60):
        p = px[x, y]
        if p[0]+p[1]+p[2] > 450: n += 1
print('center bright:', n)
# corner should no longer have crosshair
n2 = 0
for y in range(h-160, h):
    for x in range(w-160, w):
        p = px[x, y]
        if p[0]+p[1]+p[2] > 450: n2 += 1
print('corner bright:', n2)