from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# bright green: g > 150, g > r + 60, g > b + 60
greens = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[1] > 150 and p[1] > p[0] + 60 and p[1] > p[2] + 60:
            greens.append((x, y, p))
print('green px:', len(greens))
if greens:
    xs = [g[0] for g in greens]; ys = [g[1] for g in greens]
    print('green bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
    print('samples:', greens[:4])