
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 截图 = 未翻转 framebuffer！用户屏幕翻转显示 → 用户看到 = 截图镜像！
# version 行（用户 y=1440 底部）→ 截图顶部 y≈146
print('=== version line zone y 138-170 x 700-1900 ===')
for y in range(138, 170, 2):
    line = ''
    for x in range(700, 1900, 3):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 160: ch = 'W'
        elif l > 90: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print(line)
print('=== title zone y 1030-1160 x 700-1900 (white?) ===')
for y in range(1030, 1160, 3):
    line = ''
    for x in range(700, 1900, 3):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 200: ch = 'W'
        elif p[0] > 150 and p[1] < 100: ch = 'R'
        elif p[0] > 150 and p[1] > 120 and p[2] < 110: ch = 'Y'
        elif l > 100: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print(line)
