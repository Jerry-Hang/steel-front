
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 直接扫描截图 y 140-175 和 y 290-315 找文字条纹
print('=== y 140-175 ===')
for y in range(140, 175, 3):
    line = ''
    for x in range(0, w, 8):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 160: ch = 'W'
        elif l > 90: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print(line)
print('=== y 290-315 ===')
for y in range(290, 315, 3):
    line = ''
    for x in range(0, w, 8):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 160: ch = 'W'
        elif l > 90: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print(line)
