
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 截图 = 未翻转 framebuffer！用户屏幕翻转显示 → 用户看到的画面 = 截图的垂直镜像！
# 开始菜单（用户看到）：标题 STEEL FRONT 在用户 y=480（0.30h）
# 截图（未翻转）镜像位置：标题在截图 y = 1600-480-56 = 1064 附近
# 面板用户 (760,420)-(1800,760) → 截图 y = 1600-760=840 到 1600-420=1180
# PRESS ANY KEY 用户 y=880 → 截图 y=706
# version 用户 y=1440 → 截图 y=146（顶部）
pts = [
    ('corner', 50, 50),
    ('center', 1280, 800),
    ('panel center fb', 1280, 1010),
    ('title fb1064', 1280, 1080),
    ('version fb146', 1280, 150),
    ('PRESS fb706', 1280, 710),
]
for label, x, y in pts:
    p = px[x, y]
    print('%-20s (%4d,%4d) = (%3d,%3d,%3d)' % (label, x, y, p[0], p[1], p[2]))
from collections import Counter
c = Counter()
for y in range(0, h, 8):
    for x in range(0, w, 8):
        p = px[x, y]
        c[(p[0]//64, p[1]//64, p[2]//64)] += 1
print('top colors:')
for k, v in c.most_common(6):
    print('  ~rgb', (k[0]*64+32, k[1]*64+32, k[2]*64+32), 'count', v)
