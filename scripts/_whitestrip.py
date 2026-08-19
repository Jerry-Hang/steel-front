
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 扫描 y=156 行找白色条纹 x 范围
for label, y in [('y156', 156), ('y300', 300)]:
    xs = []
    for x in range(0, w):
        p = px[x, y]
        if p[0] > 150 and p[1] > 150 and p[2] > 150:
            xs.append(x)
    if xs:
        print(label, 'white x range:', min(xs), '-', max(xs), 'count:', len(xs))
    else:
        print(label, 'no white at x scan')
# 全局找灰色 (153,153,153) version 行文字
gray = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if abs(p[0]-153) < 20 and abs(p[1]-153) < 20 and abs(p[2]-153) < 20:
            gray.append((x, y))
if gray:
    xs = [p[0] for p in gray]; ys = [p[1] for p in gray]
    print('gray153:', len(gray), 'x', min(xs), '-', max(xs), 'y', min(ys), '-', max(ys))
else:
    print('gray153: NONE')
