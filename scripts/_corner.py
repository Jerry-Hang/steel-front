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
# debug text expected at (20,88) scaled: yellow/cyan text
print('(20,88) area  :', rc(15, 80, 420, 125))
# crosshair expected near (2560,1600): white+red at bottom-right corner
print('bottom-right  :', rc(w-70, h-70, w, h))
# also check full bottom-right quadrant for white cross arms
bright = 0
for y in range(h-160, h):
    for x in range(w-160, w):
        p = px[x, y]
        if p[0]+p[1]+p[2] > 450: bright += 1
print('corner bright px:', bright)