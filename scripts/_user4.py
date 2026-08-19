
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 用户视角（翻转）ASCII
im2 = im.transpose(Image.FLIP_TOP_BOTTOM)
px2 = im2.load()
cols, rows = 80, 50
chars = ' .:-=+*#%@'
for r in range(rows):
    line = ''
    for c in range(cols):
        x = int((c + 0.5) * w / cols)
        y = int((r + 0.5) * h / rows)
        p = px2[x, y]
        l = (p[0] + p[1] + p[2]) / 3
        ch = chars[min(9, int(l / 26))]
        if p[2] > p[0] + 30 and p[0] < 120: ch = 'B'
        elif p[1] > p[0] + 30 and p[1] > p[2] + 30: ch = 'G'
        elif p[0] > p[1] + 40 and p[0] > p[2] + 40: ch = 'R'
        elif p[0] > 140 and p[1] > 120 and p[2] < 110: ch = 'Y'
        elif l > 220: ch = 'W'
        line += ch
    print(line)
