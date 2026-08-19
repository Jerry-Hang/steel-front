from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# 定位顶部绿色像素
locs = []
for y in range(0, int(h*0.35), 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[1] > 100 and p[1] > p[0] + 25 and p[1] > p[2] + 25:
            locs.append((x, y, p))
print('top green px:', len(locs))
if locs:
    xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
    print('bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    print('samples:', locs[:8])
else:
    print('none')
# 底部绿色像素
locs2 = []
for y in range(int(h*0.65), h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[1] > 100 and p[1] > p[0] + 25 and p[1] > p[2] + 25:
            locs2.append((x, y, p))
print('bottom green px:', len(locs2))
if locs2:
    xs = [l[0] for l in locs2]; ys = [l[1] for l in locs2]
    print('bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    print('samples:', locs2[:8])
# 定位底部亮灰 (5,5,5)=(160,160,160) 像素
locs3 = []
for y in range(int(h*0.5), h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if abs(p[0]-160)<20 and abs(p[1]-160)<20 and abs(p[2]-160)<20:
            locs3.append((x, y))
print('bottom light-gray px:', len(locs3))
if locs3:
    xs = [l[0] for l in locs3]; ys = [l[1] for l in locs3]
    print('bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))