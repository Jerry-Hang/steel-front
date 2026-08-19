
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 检查 y 146-166 和 y 288-312 的白色条纹形状
for label, y0, y1 in [('stripA', 146, 168), ('stripB', 288, 312)]:
    print('===', label, '===')
    for y in range(y0, y1, 2):
        line = ''
        for x in range(0, w, 6):
            p = px[x, y]
            l = (p[0]+p[1]+p[2])/3
            if l > 180: line += 'W'
            elif l > 100: line += '.'
            elif l > 40: line += ':'
            else: line += ' '
        print(line)
