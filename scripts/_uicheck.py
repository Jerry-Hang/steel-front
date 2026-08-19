
from PIL import Image
for name in ['ui_start']:
    im = Image.open('D:/Rust/steel-front/%s.png' % name).convert('RGB')
    w, h = im.size
    px = im.load()
    print('###', name, w, h)
    # 采样关键点（用户屏幕坐标 = 截图镜像，但布局在引擎侧；直接检查截图内容）
    pts = [
        ('center', 1280, 800),
        ('center red?', 1280, 900),
        ('tl', 100, 100),
        ('top center', 1280, 80),
        ('bottom center', 1280, 1520),
        ('left mid', 100, 800),
    ]
    for label, x, y in pts:
        p = px[x, y]
        print('  %-16s (%4d,%4d) = (%3d,%3d,%3d)' % (label, x, y, p[0], p[1], p[2]))
    # 全图是否接近遮罩暗化: 平均亮度
    tot = 0; n = 0
    for y in range(0, h, 16):
        for x in range(0, w, 16):
            p = px[x, y]
            tot += (p[0]+p[1]+p[2])/3; n += 1
    print('avg luma:', tot/n)
