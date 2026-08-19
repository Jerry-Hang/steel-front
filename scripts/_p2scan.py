from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def rc(x0, y0, x1, y1, step=3):
    c = Counter()
    for y in range(y0, y1, step):
        for x in range(x0, x1, step):
            p = px[x, y]
            c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    return c.most_common(4)
print('HP bar  :', rc(24, 1550, 390, 1580))
print('debug   :', rc(8, 40, 420, 84))
print('center  :', rc(w//2-40, h//2-40, w//2+40, h//2+40))
print('menu band:', rc(0, 496, 2560, 664))
# bright near center
n = 0
for y in range(h//2-60, h//2+60):
    for x in range(w//2-60, w//2+60):
        p = px[x, y]
        if p[0]+p[1]+p[2] > 450: n += 1
print('center bright:', n)