
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
px = im.load()
# 截图底部 y 1498-1560 x 0-700 白色大字 —— 识别是什么文字
# 高精度：每像素
for y in range(1498, 1560):
    line = ''
    for x in range(0, 700):
        p = px[x, y]
        l = (p[0]+p[1]+p[2])/3
        ch = 'W' if l > 160 else ('.' if l > 60 else ' ')
        line += ch
    if line.strip():
        print(line.rstrip())
