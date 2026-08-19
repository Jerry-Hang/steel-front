
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# title zone 右侧 (x 1200-1900, y 1039-1110) 放大查看是否为 STEEL FRONT 标题（可能被遮罩压暗）
# 检查该区域亮度直方图
from collections import Counter
c = Counter()
for y in range(1039, 1110):
    for x in range(1200, 1900):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        c[int(l//32)] += 1
print('brightness histogram (32-luma bins):', sorted(c.items()))
# 保存该区域为高对比 PNG 供视觉分析
out = im.crop((1200, 1039, 1900, 1110))
out = out.resize((out.width*2, out.height*2), Image.NEAREST)
out.save('D:/Rust/steel-front/title_zone_right.png')
print('saved title_zone_right.png', out.size)
