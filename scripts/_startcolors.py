
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 搜索 version 行灰色 (153,153,153) 与标题白色 (255,255,255)
for label, pred in [
    ('gray153 version', lambda p: abs(p[0]-153)<25 and abs(p[1]-153)<25 and abs(p[2]-153)<25),
    ('white255 title', lambda p: p[0]>235 and p[1]>235 and p[2]>235),
    ('cyan subtitle', lambda p: p[2]>150 and p[1]>120 and p[0]<90),
    ('yellow hint', lambda p: p[0]>180 and p[1]>140 and p[2]<110),
]:
    pts = []
    for y in range(0, h, 3):
        for x in range(0, w, 3):
            if pred(px[x, y]):
                pts.append((x, y))
    if pts:
        xs = [p[0] for p in pts]; ys = [p[1] for p in pts]
        print(label, 'count', len(pts), 'x', min(xs), '-', max(xs), 'y', min(ys), '-', max(ys))
    else:
        print(label, 'NONE')
