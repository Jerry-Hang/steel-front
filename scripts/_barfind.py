from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# strict HP-fill green: g dominant
locs = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[1] > 140 and p[1] > p[0] + 50 and p[1] > p[2] + 50:
            locs.append((x, y))
print('green fill px:', len(locs))
if locs:
    xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
    print('green bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    # cluster rows
    from collections import Counter
    yc = Counter(ys)
    print('top y clusters:', yc.most_common(6))
# the ammo bar orange: r dominant, g mid, b low
oranges = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 180 and p[1] > 90 and p[1] < 190 and p[2] < 80:
            oranges.append((x, y))
if oranges:
    xs = [l[0] for l in oranges]; ys = [l[1] for l in oranges]
    print('orange bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)), 'n=', len(oranges))