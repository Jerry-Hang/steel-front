
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 截图 = 未翻转 framebuffer！用户屏幕翻转显示 → 用户看到的画面 = 截图的垂直镜像！
# 开始菜单（用户看到）：标题 STEEL FRONT 在 y=480；面板 (760,420)-(1800,760)；PRESS ANY KEY y=880；version y=1440
# 截图（未翻转）镜像位置：标题 y=1064；面板 y=840-1180；PRESS y=706；version y=146
# 关键验证：截图顶部 y 130-175 有没有 version 行（灰色 153）
print('=== version zone y 130-175 x 900-1700 ===')
for y in range(130, 175, 2):
    line = ''
    for x in range(900, 1700, 3):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 150: ch = 'W'
        elif l > 90: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print(line)
# PRESS ANY KEY zone y 700-720
print('=== PRESS zone y 700-720 x 900-1700 ===')
for y in range(700, 720, 2):
    line = ''
    for x in range(900, 1700, 3):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 150: ch = 'W'
        elif p[0] > 150 and p[1] > 120 and p[2] < 110: ch = 'Y'
        elif l > 90: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print(line)
