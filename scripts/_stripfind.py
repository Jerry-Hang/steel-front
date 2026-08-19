
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 找 y=152-160 和 y=296-304 的白色像素 x 范围
for y0, y1, label in [(150, 162, 'strip A y152'), (294, 306, 'strip B y296'), (760, 880, 'cyan zone')]:
    xs = []
    for y in range(y0, y1):
        for x in range(0, w):
            p = px[x, y]
            if p[0] > 150 and p[1] > 150 and p[2] > 150:
                xs.append(x)
    if xs:
        print(label, 'x range:', min(xs), '-', max(xs), 'count:', len(xs))
    else:
        print(label, 'no white')
# 青色带 y 760-880 的位置
cyan_x = []
for y in range(760, 880):
    for x in range(0, w):
        p = px[x, y]
        if p[2] > 180 and p[1] > 140 and p[0] < 120:
            cyan_x.append(x)
if cyan_x:
    print('cyan x range:', min(cyan_x), '-', max(cyan_x), 'count:', len(cyan_x))
# 红色 y 1088 区域 x 范围
red_x = []
for y in range(1060, 1120):
    for x in range(0, w):
        p = px[x, y]
        if p[0] > 150 and p[1] < 100:
            red_x.append(x)
if red_x:
    print('red x range:', min(red_x), '-', max(red_x), 'count:', len(red_x))
