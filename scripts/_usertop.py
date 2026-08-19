
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
im = im.transpose(Image.FLIP_TOP_BOTTOM)  # 用户视角
px = im.load()
# 用户看到左上角 (x 0-300, y 0-120) —— 是什么文字？
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
print('=== user top-left y 0-120 x 0-500 ===')
ascii_region(0, 0, 500, 120, 3)
print('=== user title zone y 440-600 x 700-1900 ===')
ascii_region(700, 440, 1900, 600, 6)
