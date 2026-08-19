
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 用户看到 = 截图垂直翻转。开始菜单（用户视角）:
# 标题 STEEL FRONT 用户 y=480 -> 截图 y = 1600-480-56 = 1064
# 面板用户 (760,420)-(1800,760) -> 截图 y = 840-1180
# PRESS ANY KEY 用户 y=880 -> 截图 y=706
# version 用户 y=1440 -> 截图 y=146
# 找白色文字像素（标题应白色）
white = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 200 and p[1] > 200 and p[2] > 200:
            white.append((x, y))
print('white pixels:', len(white))
if white:
    xs = [p[0] for p in white]; ys = [p[1] for p in white]
    print('x:', min(xs), '-', max(xs))
    print('y:', min(ys), '-', max(ys))
    # 直方图：哪些 y 带
    from collections import Counter
    yc = Counter(y//32 for y in ys)
    print('y bands (32px):', sorted(yc.items()))
# 标题 STEEL FRONT 应该在截图 y 1064-1120, x 760-1800 区域
zone = 0
for y in range(1064, 1120):
    for x in range(760, 1800):
        p = px[x, y]
        if p[0] > 200 and p[1] > 200 and p[2] > 200:
            zone += 1
print('white in title zone (y1064-1120 x760-1800):', zone)
