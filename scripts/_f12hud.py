
from PIL import Image
im = Image.open('D:/Rust/steel-front/screenshots/steel_front_1786966687.png').convert('RGB')
w, h = im.size
px = im.load()
# framebuffer 坐标采样（用户屏幕翻转显示，framebuffer 顶部 = 用户底部）
# version 行: 用户 y=1440 -> fb y=146
# PRESS ANY KEY: 用户 y=880 -> fb y=706
# 标题: 用户 y=480 -> fb y=1064
pts = [
    ('version fb146', 1280, 146),
    ('PRESS fb706', 1280, 706),
    ('title fb1064', 1280, 1064),
    ('panel center fb1010', 1280, 1010),
    ('outside panel fb800', 1280, 800),
    ('outside panel fb1220', 1280, 1220),
    ('npc red?', 1000, 900),
    ('npc red?2', 1200, 700),
]
for label, x, y in pts:
    p = px[x, y]
    print('  %-22s (%4d,%4d) = (%3d,%3d,%3d)' % (label, x, y, p[0], p[1], p[2]))
# 扫描 fb y=140-160 找 version 行文字
line = ''
for x in range(600, 2000, 4):
    p = px[x, 150]
    l = (p[0]+p[1]+p[2])/3
    line += 'W' if l > 160 else ('.' if l > 60 else ' ')
print('fb y150 scan:', line)
# fb y=700-714 PRESS ANY KEY
line = ''
for x in range(600, 2000, 4):
    p = px[x, 710]
    l = (p[0]+p[1]+p[2])/3
    line += 'W' if l > 160 else ('.' if l > 60 else ' ')
print('fb y710 scan:', line)
