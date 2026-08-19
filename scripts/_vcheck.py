
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 截图 = BitBlt 未翻转 framebuffer！用户屏幕翻转显示 → 用户看到的画面 = 截图的垂直镜像！
# 开始菜单（用户看到）：标题 STEEL FRONT 在用户 y=480（0.30h）
# 截图（未翻转）中标题在 y = 1600-480-56 = 1064 附近
# 面板用户 (760,420)-(1800,760) → 截图 y = 840-1180
# PRESS ANY KEY 用户 y=880 → 截图 y=706
# version 用户 y=1440 → 截图 y=146（顶部）
# 检查：截图顶部 y 146 有没有 version 行（灰色 0.6=153）
for y in [144, 146, 148, 150, 152]:
    line = ''
    for x in range(900, 1700, 4):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 170: ch = 'W'
        elif l > 100: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print(y, line)
# 底部 y 1498-1522 的白色文字（用户看到顶部）
print('--- bottom y 1498-1522 (user sees top) ---')
for y in range(1498, 1524, 2):
    line = ''
    for x in range(0, 900, 4):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 170: ch = 'W'
        elif l > 100: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print(line)
