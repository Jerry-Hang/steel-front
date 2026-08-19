
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 截图 = 未翻转 framebuffer。用户看到 = 垂直镜像。
# 开始菜单标题 STEEL FRONT 用户 y=480 -> 截图 y = 1600-480-56 = 1064
# title zone y 1030-1160 放大 ASCII（step 2）辨认
def ascii_region(x0, y0, x1, y1, step=2):
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
print('=== title zone y 1050-1140 x 900-1900 ===')
ascii_region(900, 1050, 1900, 1140, 2)
