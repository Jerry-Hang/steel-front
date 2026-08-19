from PIL import Image
im = Image.open('D:/Rust/steel-front/show_me_gun_p3.png').convert('RGB')
w, h = im.size
px = im.load()
locs = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if (p[0] > 110 and 70 < p[1] < 200 and p[2] < 150 and p[0] > p[2] + 35) or (abs(p[0]-p[1]) < 30 and abs(p[1]-p[2]) < 30 and 90 < p[0] < 200):
            locs.append((x, y))
if locs:
    xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
    print('gun px:', len(locs))
    print('bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    print('bbox w/h:', max(xs)-min(xs), max(ys)-min(ys))
    # 重心
    print('center:', (sum(xs)//len(xs), sum(ys)//len(ys)))