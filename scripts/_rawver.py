
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
px = im.load()
# 原始截图（未翻转）y 130-175 扫描 version 行（灰色 0.6 = 153）
print('=== raw y 130-175 x 900-1900 ===')
for y in range(130, 175, 3):
    line = ''
    for x in range(900, 1900, 4):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = ' '
        if l > 160: ch = 'W'
        elif l > 90: ch = '.'
        elif l > 40: ch = ':'
        else: ch = ' '
        line += ch
    print(line)
