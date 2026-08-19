
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 截图 = framebuffer 未翻转！用户看到 = 截图垂直镜像！
# 开始菜单（用户看到）标题 STEEL FRONT 在 y=480 -> 截图 y = 1600-480-56 = 1064
# 面板用户 (760,420)-(1800,760) -> 截图 y = 840-1180
# PRESS ANY KEY 用户 y=880 -> 截图 y=706
# version 用户 y=1440 -> 截图 y=146（顶部）
# 让我扫描截图 y 700-720（PRESS ANY KEY 位置）和 y 144-160（version 位置）
for label, y0, y1 in [('PRESS-ANY-KEY zone y700-720', 700, 720), ('version zone y140-170', 140, 170)]:
    print('===', label, '===')
    for y in range(y0, y1, 2):
        line = ''
        for x in range(600, 2000, 4):
            p = px[x, y]
            l = (p[0]+p[1]+p[2])/3
            ch = ' '
            if l > 180: ch = 'W'
            elif p[0] > 150 and p[1] < 100: ch = 'R'
            elif p[0] > 150 and p[1] > 120 and p[2] < 110: ch = 'Y'
            elif l > 100: ch = '.'
            elif l > 50: ch = ':'
            else: ch = ' '
            line += ch
        print(line)
