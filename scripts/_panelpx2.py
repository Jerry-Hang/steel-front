
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 引擎坐标 = 用户坐标垂直镜像 (y' = 1600 - y)
# 用户面板 (760,420,1040,340) -> 引擎 (760,840)-(1800,1180)
pts = [
    ('engine panel center', 1280, 1010),
    ('engine panel TL inner', 820, 880),
    ('engine panel BR inner', 1740, 1140),
    ('engine outside left', 300, 1010),
    ('engine outside top', 1280, 700),
    ('engine outside bottom', 1280, 1350),
]
for label, x, y in pts:
    p = px[x, y]
    print('%-24s (%4d,%4d) = (%3d,%3d,%3d)' % (label, x, y, p[0], p[1], p[2]))
