
from PIL import Image
import sys

def px(im, x, y):
    p = im.load()[x, y]
    return (p[0], p[1], p[2])

def show(im, name, points):
    print('==', name)
    for label, x, y in points:
        print('  %-28s (%5d,%4d) = %s' % (label, x, y, px(im, x, y)))

for name in ['ui_start', 'ui_esc', 'ui_settings']:
    im = Image.open('D:/Rust/steel-front/%s.png' % name).convert('RGB')
    w, h = im.size
    print('###', name, w, 'x', h)
    if name == 'ui_start':
        # 容器: 设计 (380,210,520,170) -> 像素 x2 = (760,420,1040,340), 圆角 48px
        show(im, name, [
            ('panel center', 1280, 590),
            ('outside left same-y', 200, 590),
            ('corner cut (should be outside)', 772, 432),
            ('inside near corner (panel)', 820, 470),
            ('below panel', 1280, 900),
        ])
    elif name == 'ui_esc':
        # 面板: (450,280,380,240) -> (900,560,760,480), 圆角 48
        show(im, name, [
            ('panel center', 1280, 800),
            ('outside left', 300, 800),
            ('corner cut', 912, 572),
            ('inside near corner', 960, 620),
            ('below panel', 1280, 1200),
        ])
    else:
        # 容器: (320,52,640,540) -> (640,104,1280,1080), 圆角 48
        show(im, name, [
            ('panel center', 1280, 650),
            ('outside left', 200, 650),
            ('corner cut', 652, 116),
            ('inside near corner', 700, 160),
            ('below panel', 1280, 1300),
        ])
