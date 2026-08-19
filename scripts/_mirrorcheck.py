from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def rc(x0, y0, x1, y1, step=2):
    c = Counter()
    for y in range(y0, y1, step):
        for x in range(x0, x1, step):
            p = px[x, y]
            c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    return c.most_common(5)
# mirrored HP bar would be at top-left (48,48) 720x44
print('top-left (20,30)-(800,100):', rc(20, 30, 800, 100))
# normal HP bar at bottom-left
print('bot-left (20,1530)-(800,1600):', rc(20, 1530, 800, 1600))
# crosshair center
print('center (1240,760)-(1320,840):', rc(1240, 760, 1320, 840))
# green bar fill pixels: bright green anywhere in top-left
n = 0
for y in range(20, 110):
    for x in range(20, 800):
        p = px[x, y]
        if p[1] > 150 and p[1] > p[0] + 40 and p[1] > p[2] + 40:
            n += 1
print('green fill top-left:', n)