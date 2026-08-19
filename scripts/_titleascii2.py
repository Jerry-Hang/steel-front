
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 引擎画面（截图）中开始菜单是颠倒的：标题在底部！
# 用户看到标题 y=480(设计240*2) -> 引擎 y = 1600-480-56=1064 附近 (scale4 x2 = 56px 高)
# 副标题青色 y=568 -> 引擎 1600-568-17=1015
# PRESS ANY KEY 黄 y=880 -> 引擎 720
# 精细 ASCII: 标题区域引擎坐标 x 700-1900, y 1030-1160
def ascii_region(x0, y0, x1, y1, step=4):
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
print('--- title zone (engine coords, y 1030-1160) ---')
ascii_region(700, 1030, 1900, 1160)
print('--- press any key zone (engine y 690-760) ---')
ascii_region(700, 690, 1900, 760)
