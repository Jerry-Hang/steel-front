
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
px = im.load()
# title zone 右侧文字 (x 1200-1900, y 1039-1110)：找最亮像素与颜色分布
from collections import Counter
c = Counter()
bright = []
for y in range(1039, 1110):
    for x in range(1200, 1900):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        if l > 60:
            c[(p[0]//48, p[1]//48, p[2]//48)] += 1
        if l > 150:
            bright.append((x, y, p, l))
print('bright(>150) count:', len(bright))
for k, v in c.most_common(6):
    print('  bucket', k, '~ rgb', (k[0]*48+24, k[1]*48+24, k[2]*48+24), 'count', v)
print('sample bright pixels:', bright[:8])
# 左上方 title zone 的红色 RRRR 区域采样
print('red block samples:')
for y in [1050, 1100, 1150]:
    for x in [900, 930, 960]:
        print('  (%d,%d) =' % (x, y), px[x, y])
