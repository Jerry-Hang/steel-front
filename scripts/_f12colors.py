
from PIL import Image
im = Image.open('D:/Rust/steel-front/screenshots/steel_front_1786966687.png').convert('RGB')
w, h = im.size
px = im.load()
# 搜索 version 行灰色 (153,153,153) 和黄色 (242,204,51) 和白色 (255,255,255)
for label, pred in [
    ('gray153', lambda p: abs(p[0]-153)<25 and abs(p[1]-153)<25 and abs(p[2]-153)<25),
    ('yellow', lambda p: p[0]>180 and p[1]>140 and p[2]<110),
    ('white', lambda p: p[0]>220 and p[1]>220 and p[2]>220),
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
# 蓝色面板色 (0.05,0.06,0.10,0.72) 叠加遮罩后 ≈ (13,15,26)*0.72 + 世界*0.28*0.28 ... 搜深蓝黑
dark = []
for y in range(0, h, 4):
    for x in range(0, w, 4):
        p = px[x, y]
        if p[0] < 40 and p[1] < 45 and p[2] < 60:
            dark.append((x, y))
if dark:
    xs = [p[0] for p in dark]; ys = [p[1] for p in dark]
    print('dark-blue count', len(dark), 'x', min(xs), '-', max(xs), 'y', min(ys), '-', max(ys))
else:
    print('dark-blue NONE')
