from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def strip(y, label):
    c = Counter()
    for x in range(0, w, 8):
        p = px[x, y]
        c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    print(label, c.most_common(3))
for y in [100, 300, 500, 700, 900, 1100, 1300, 1500]:
    strip(y, 'y=%d:' % y)
reds = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 200 and 60 < p[1] < 130 and 50 < p[2] < 110:
            reds.append((x, y))
if reds:
    xs = [l[0] for l in reds]; ys = [l[1] for l in reds]
    print('enemy-red bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)), 'n=', len(reds))
else:
    print('enemy-red: none')