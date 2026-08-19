from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# YELLOW debug text: r>200, g>200, b<120
yellows = []; cyans = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 200 and p[1] > 200 and p[2] < 120:
            yellows.append((x, y))
        if p[2] > 200 and p[0] < 120 and p[1] > 180:
            cyans.append((x, y))
def bbox(locs, label):
    if locs:
        xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
        print(label, 'n=', len(locs), 'bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    else:
        print(label, 'none')
bbox(yellows, 'yellow text')
bbox(cyans, 'cyan text')
# also WHITE text (HP/AMMO): all > 230
whites = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 230 and p[1] > 230 and p[2] > 230:
            whites.append((x, y))
bbox(whites, 'white text')