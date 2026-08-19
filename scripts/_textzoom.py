
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 裁剪 y 1039-1110, x 1350-1900 放大 4x 渲染 ASCII（每像素 1 字符）
x0, y0, x1, y1 = 1350, 1039, 1900, 1110
for y in range(y0, y1):
    line = ''
    for x in range(x0, x1):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 150: ch = 'W'
        elif p[0] > 150 and p[1] < 110: ch = 'R'
        elif l > 60: ch = '.'
        else: ch = ':'
        line += ch
    print(line)
