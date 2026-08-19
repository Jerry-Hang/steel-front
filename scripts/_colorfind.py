
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 全图搜索白色像素 (标题 STEEL FRONT 白色 scale 4 = 255,255,255 附近)
white = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 220 and p[1] > 220 and p[2] > 220:
            white.append((x, y))
print('white pixels:', len(white))
if white:
    xs = [p[0] for p in white]; ys = [p[1] for p in white]
    print('x range:', min(xs), '-', max(xs))
    print('y range:', min(ys), '-', max(ys))
# 黄色像素 (PRESS ANY KEY 黄色 0.95,0.8,0.2 = 242,204,51)
yellow = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 180 and p[1] > 140 and p[2] < 110:
            yellow.append((x, y))
print('yellow pixels:', len(yellow))
if yellow:
    xs = [p[0] for p in yellow]; ys = [p[1] for p in yellow]
    print('x range:', min(xs), '-', max(xs))
    print('y range:', min(ys), '-', max(ys))
# 青色像素 (副标题 CYAN 0.2,0.8,0.9 = 51,204,229)
cyan = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[2] > 160 and p[1] > 140 and p[0] < 100:
            cyan.append((x, y))
print('cyan pixels:', len(cyan))
if cyan:
    xs = [p[0] for p in cyan]; ys = [p[1] for p in cyan]
    print('x range:', min(xs), '-', max(xs))
    print('y range:', min(ys), '-', max(ys))
