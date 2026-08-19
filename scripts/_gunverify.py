from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# walnut srgb (189,156,108) tight in bottom-center area (gun region)
locs = []
for y in range(900, 1600, 1):
    for x in range(1000, 2500, 1):
        p = px[x, y]
        if abs(p[0]-189)<28 and abs(p[1]-156)<28 and abs(p[2]-108)<28:
            locs.append((x, y))
print('walnut in bottom-right:', len(locs))
if locs:
    xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
    print('bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
# steel srgb (172,179,186)
steels = []
for y in range(900, 1600, 1):
    for x in range(1000, 2500, 1):
        p = px[x, y]
        if abs(p[0]-172)<20 and abs(p[1]-179)<20 and abs(p[2]-186)<20:
            steels.append((x, y))
print('steel in bottom-right:', len(steels))
if steels:
    xs = [l[0] for l in steels]; ys = [l[1] for l in steels]
    print('bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))