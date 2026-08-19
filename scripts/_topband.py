
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 截图顶部 y 130-175 全宽扫描找文字（version 行灰色 153 或 ctrl 行）
print('=== top band y 130-175, sampling every 4px ===')
for y in range(130, 175, 4):
    line = ''
    for x in range(0, w, 8):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 150: ch = 'W'
        elif l > 90: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print('%3d %s' % (y, line))
