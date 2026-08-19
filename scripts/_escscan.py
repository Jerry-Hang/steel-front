
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_esc.png').convert('RGB')
w, h = im.size
px = im.load()
print('size', w, h)
# 找深色面板: 面板色 (0.07,0.10,0.14) alpha 0.88 叠加遮罩 -> 非常暗的蓝灰
# 扫描网格，找出颜色均匀且暗的区域
def is_panel(p):
    r, g, b = p
    return r < 70 and g < 80 and b < 90 and abs(r-g) < 25 and abs(b-r) < 30
rows = []
for y in range(0, h, 32):
    line = ''
    for x in range(0, w, 32):
        p = px[x, y]
        if is_panel(p):
            line += '#'
        elif p[0] > 200 and p[1] > 200 and p[2] > 200:
            line += 'W'
        elif p[2] > p[0] + 30:
            line += 'B'
        elif p[0] > 150 and p[1] < 100:
            line += 'R'
        elif p[0] > 150 and p[1] > 120 and p[2] < 100:
            line += 'Y'
        else:
            line += '.'
    rows.append(line)
for i, line in enumerate(rows):
    print('%3d %s' % (i*32, line))
