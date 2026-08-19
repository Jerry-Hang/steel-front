
from PIL import Image
im = Image.open('D:/Rust/steel-front/screenshots/steel_front_1786966687.png').convert('RGB')
w, h = im.size
print('size', w, 'x', h)
px = im.load()
# 采样关键点：开始菜单应有 遮罩(暗) + 圆角面板(深蓝黑) + 白色标题 + PRESS ANY KEY(黄)
pts = [
    ('corner', 50, 50),
    ('center', 1280, 800),
    ('panel center', 1280, 590),
    ('title zone', 1280, 500),
    ('subtitle', 1280, 570),
    ('hint 0.55h', 1280, 880),
    ('bottom-left', 200, 1500),
]
for label, x, y in pts:
    p = px[x, y]
    print('  %-16s (%4d,%4d) = (%3d,%3d,%3d)' % (label, x, y, p[0], p[1], p[2]))
# 全图颜色分布
from collections import Counter
c = Counter()
for y in range(0, h, 8):
    for x in range(0, w, 8):
        p = px[x, y]
        c[(p[0]//64, p[1]//64, p[2]//64)] += 1
print('top colors:')
for k, v in c.most_common(6):
    print('  ~rgb', (k[0]*64+32, k[1]*64+32, k[2]*64+32), 'count', v)
