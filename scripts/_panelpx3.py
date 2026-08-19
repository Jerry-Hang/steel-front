
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 面板用户 (760,420)-(1800,760) -> 截图镜像 y = 1600-760=840 到 1600-420=1180
# 面板中心截图 (1280, 1010)
pts = [
    ('panel center', 1280, 1010),
    ('panel TL inner', 820, 880),
    ('panel BR inner', 1740, 1140),
    ('panel mid-left', 800, 1010),
    ('panel mid-right', 1780, 1010),
    ('outside panel top', 1280, 800),
    ('outside panel bottom', 1280, 1220),
]
for label, x, y in pts:
    p = px[x, y]
    print('%-22s (%4d,%4d) = (%3d,%3d,%3d)' % (label, x, y, p[0], p[1], p[2]))
