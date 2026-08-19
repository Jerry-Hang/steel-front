from PIL import Image
im = Image.open('D:/Rust/steel-front/show_me_gun_v4.png').convert('RGB')
w, h = im.size
px = im.load()
locs = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 110 and 70 < p[1] < 200 and p[2] < 150 and p[0] > p[2] + 35:
            locs.append((x, y))
if locs:
    xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
    print('walnut n:', len(locs), 'bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    # 按行分布
    from collections import Counter
    yc = Counter(y//100*100 for y in ys)
    print('y分布:', sorted(yc.items()))
    xc = Counter(x//200*200 for x in xs)
    print('x分布:', sorted(xc.items()))