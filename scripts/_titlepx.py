
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
px = im.load()
# 直接采样 title zone 右侧文字像素 (x 1400-1900, y 1088-1156) 的亮色像素
from collections import Counter
c = Counter()
n = 0
for y in range(1088, 1156):
    for x in range(1400, 1900, 2):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        if l > 80:
            c[(p[0]//64, p[1]//64, p[2]//64)] += 1
            n += 1
print('bright pixels:', n)
for k, v in c.most_common(8):
    print('  bucket', k, '-> rgb ~', (k[0]*64+32, k[1]*64+32, k[2]*64+32), 'count', v)
# 采样标题区几个具体点
for label, x, y in [('t1', 1500, 1100), ('t2', 1600, 1100), ('t3', 1700, 1100), ('t4', 1500, 1140)]:
    print(label, (x, y), '=', px[x, y])
