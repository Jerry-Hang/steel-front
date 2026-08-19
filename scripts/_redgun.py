from PIL import Image
im = Image.open('D:/Rust/steel-front/show_me_gun_red.png').convert('RGB')
w, h = im.size
px = im.load()
reds = []
for y in range(0, h, 1):
    for x in range(0, w, 1):
        p = px[x, y]
        if p[0] > 200 and p[1] < 90 and p[2] < 90:
            reds.append((x, y))
print('bright red px:', len(reds))
if reds:
    xs = [l[0] for l in reds]; ys = [l[1] for l in reds]
    print('red bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))