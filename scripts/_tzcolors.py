
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
px = im.load()
# title zone 右侧 (x 1200-1900, y 1039-1110) 颜色分布
from collections import Counter
c = Counter()
for y in range(1039, 1110):
    for x in range(1200, 1900):
        p = px[x, y]
        c[(p[0]//64, p[1]//64, p[2]//64)] += 1
print('total', sum(c.values()))
for k, v in c.most_common(8):
    print('  bucket', k, '~ rgb', (k[0]*64+32, k[1]*64+32, k[2]*64+32), 'count', v)
# 检查 title zone 右侧是否真的是文字：找规则间隔的亮像素
print('--- column scan y=1080, x 1200-1900 ---')
line = ''
for x in range(1200, 1900, 4):
    p = px[x, 1080]
    l = (p[0]+p[1]+p[2])/3
    line += 'W' if l > 180 else ('R' if p[0] > 150 and p[1] < 110 else ('Y' if p[0]>150 and p[1]>120 and p[2]<110 else ('.' if l > 60 else ' ')))
print(line)
