
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
im = im.transpose(Image.FLIP_TOP_BOTTOM)
px = im.load()
# 用户看到左上角 y 0-120 x 0-500 的 B/Y 文字精细查看（step 2）
x0, y0, x1, y1 = 0, 0, 500, 120
for y in range(y0, y1, 2):
    line = ''
    for x in range(x0, x1, 2):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 200: ch = 'W'
        elif p[0] > 150 and p[1] < 100: ch = 'R'
        elif p[0] > 150 and p[1] > 120 and p[2] < 110: ch = 'Y'
        elif p[2] > p[0]+20 and p[0] < 130: ch = 'B'
        elif l > 100: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print(line)
