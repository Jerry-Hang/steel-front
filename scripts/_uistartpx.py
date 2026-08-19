
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
print('size', w, h)
# 采样设计空间布局关键点（1280x800 设计 x2 = 2560x1600）
# 开始菜单: 容器 (380,210,520,170) -> 像素 (760,420,1040,340), 圆角24*2=48
# 遮罩 0.72 alpha; 面板 0.72 alpha 深蓝黑 (0.05,0.06,0.10)
pts = [
    ('panel center', 1280, 590),
    ('outside left', 200, 590),
    ('corner TL cut', 772, 432),
    ('inside TL', 840, 500),
    ('corner BR cut', 1792, 752),
    ('inside BR', 1700, 700),
    ('title y=480', 1280, 500),
    ('subtitle y=568', 1280, 575),
    ('ops y=608', 1280, 615),
    ('hint y=880', 1280, 880),
    ('ctrl1 y=992', 1280, 992),
    ('version y=1440', 1280, 1440),
]
for label, x, y in pts:
    p = px[x, y]
    print('  %-20s (%4d,%4d) = (%3d,%3d,%3d) lum=%3d' % (label, x, y, p[0], p[1], p[2], p[0]+p[1]+p[2]))
# 垂直扫描 x=1280 找边界
print('vertical scan:')
prev = None
for y in range(0, h, 16):
    p = px[1280, y]
    l = p[0]+p[1]+p[2]
    if prev is None or abs(l-prev) > 80:
        print('  y=%4d lum=%3d rgb=%s' % (y, l, p))
        prev = l
