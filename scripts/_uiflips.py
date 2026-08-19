
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
# 垂直翻转（用户屏幕补偿）后的采样坐标
flip = im.transpose(Image.FLIP_TOP_BOTTOM)
px = flip.load()
def lum(p): return p[0]+p[1]+p[2]
print('FLIPPED ui_start:')
# 开始菜单布局（设计空间 -> 像素 x2）:
# 容器 (380,210,520,170) 圆角24*2=48; 标题 y=240; 副标题 284; ops 308
pts = [
    ('panel center', 1280, 590),
    ('outside left', 200, 590),
    ('corner cut TL (760,420)', 768, 428),
    ('inside near TL (830,480)', 830, 480),
    ('corner cut BR (1800,760)', 1792, 752),
    ('title area', 1280, 500),
    ('subtitle', 1280, 568+14),
    ('ops', 1280, 608+7),
    ('hint 0.55h', 1280, 880),
]
for label, x, y in pts:
    p = px[x, y]
    print('  %-26s (%5d,%4d) lum=%3d rgb=%s' % (label, x, y, p[0]+p[1]+p[2], p))
# 垂直扫描找面板边界（翻转后）
print('vertical scan x=1280:')
prev = None
for y in range(0, h, 8):
    l = lum(px[1280, y])
    if prev is None or abs(l - prev) > 50:
        print('  y=%4d lum=%d rgb=%s' % (y, l, px[1280, y]))
        prev = l
