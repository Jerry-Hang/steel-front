
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
px = im.load()
# title zone 左侧 x 700-1200, y 1039-1110 —— 检查暗色图案是否文字
def ascii_region(x0, y0, x1, y1, step=3):
    for y in range(y0, y1, step):
        line = ''
        for x in range(x0, x1, step):
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
print('=== title zone left x 700-1250, y 1039-1110 ===')
ascii_region(700, 1039, 1250, 1110, 3)
