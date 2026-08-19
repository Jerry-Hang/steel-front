from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# near-white pixels: all channels > 230
pts = []
for y in range(0, h):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 230 and p[1] > 230 and p[2] > 230:
            pts.append((x, y))
print('near-white count:', len(pts))
if pts:
    # cluster into blobs (simple: print first 20 and bounding box)
    xs = [p[0] for p in pts]; ys = [p[1] for p in pts]
    print('bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    print('first 20:', pts[:20])
else:
    # check max brightness anywhere
    mx = (0, None)
    for y in range(0, h, 4):
        for x in range(0, w, 4):
            p = px[x, y]
            s = p[0]+p[1]+p[2]
            if s > mx[0]: mx = (s, (x, y, p))
    print('max brightness:', mx)