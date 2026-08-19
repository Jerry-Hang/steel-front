from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# enemy red (249,98,83) tight + green cube (0,255,0) tight
reds = []; greens = []
for y in range(0, h, 1):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 200 and 60 < p[1] < 130 and 50 < p[2] < 110:
            reds.append((x, y))
        if p[1] > 200 and p[0] < 80 and p[2] < 80:
            greens.append((x, y))
def bbox(locs, label):
    if locs:
        xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
        print(label, 'n=', len(locs), 'bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    else:
        print(label, 'none')
bbox(reds, 'enemy-red')
bbox(greens, 'gun-green')