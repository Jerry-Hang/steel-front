
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 垂直扫描 x=1280: 记录相邻行亮度突变
def lum(p): return p[0]+p[1]+p[2]
print('vertical scan x=1280 (y: lum)')
prev = None
for y in range(0, h, 8):
    l = lum(px[1280, y])
    if prev is None or abs(l - prev) > 60:
        print('  y=%4d lum=%d rgb=%s' % (y, l, px[1280, y]))
        prev = l
print('horizontal scan y=590:')
prev = None
for x in range(0, w, 8):
    l = lum(px[x, 590])
    if prev is None or abs(l - prev) > 60:
        print('  x=%4d lum=%d rgb=%s' % (x, l, px[x, 590]))
        prev = l
# 全图亮度分布
from collections import Counter
c = Counter()
for y in range(0, h, 16):
    for x in range(0, w, 16):
        l = (px[x,y][0]+px[x,y][1]+px[x,y][2])//3
        c[l//32] += 1
print('luma buckets:', sorted(c.items()))
