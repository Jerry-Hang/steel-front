
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 截图底部 y 1490-1560, x 0-700 高分辨率（每像素）
x0, y0, x1, y1 = 0, 1498, 700, 1560
for y in range(y0, y1):
    line = ''
    for x in range(x0, x1):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 170: ch = 'W'
        elif p[0] > 150 and p[1] < 110: ch = 'R'
        elif p[0] > 150 and p[1] > 120 and p[2] < 110: ch = 'Y'
        elif l > 90: ch = '.'
        else: ch = ':'
        line += ch
    print(line)
