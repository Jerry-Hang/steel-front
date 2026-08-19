from PIL import Image
from collections import Counter
im = Image.open(r'D:\Rust\steel-front\game_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def region_colors(x0, y0, x1, y1, step=3):
    c = Counter()
    for y in range(y0, y1, step):
        for x in range(x0, x1, step):
            p = px[x, y]
            c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    return c.most_common(5)
print('size:', w, h)
print('HP bar region   :', region_colors(24, 1550, 390, 1580))
print('ammo region     :', region_colors(400, 1550, 560, 1580))
print('debug text      :', region_colors(8, 40, 420, 84))
print('center 60x60    :', region_colors(w//2-30, h//2-30, w//2+30, h//2+30))
print('top-right       :', region_colors(w-420, 8, w-8, 80))
print('bottom-right    :', region_colors(w-420, h-80, w-8, h-8))
# exact center pixel
print('center pixel    :', px[w//2, h//2])
# bright pixels near center (crosshair is white/red)
bright = 0
for y in range(h//2-60, h//2+60, 1):
    for x in range(w//2-60, w//2+60, 1):
        p = px[x, y]
        if p[0]+p[1]+p[2] > 450: bright += 1
print('bright px near center:', bright)