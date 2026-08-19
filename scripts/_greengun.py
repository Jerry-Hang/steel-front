from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
greens = []
for y in range(0, h, 1):
    for x in range(0, w, 1):
        p = px[x, y]
        if p[1] > 200 and p[0] < 80 and p[2] < 80:
            greens.append((x, y))
print('pure green px:', len(greens))
if greens:
    xs = [g[0] for g in greens]; ys = [g[1] for g in greens]
    print('green bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    print('center:', (sum(xs)//len(xs), sum(ys)//len(ys)))