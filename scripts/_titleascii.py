
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
im = im.transpose(Image.FLIP_TOP_BOTTOM)  # 用户视角
px = im.load()
# 开始菜单标题 STEEL FRONT 用户 y≈480-536 (scale4 x2 = 56px 高)
# 裁剪标题区域精细 ASCII (每字符 4px)
def ascii_region(x0, y0, x1, y1, step=4):
    for y in range(y0, y1, step):
        line = ''
        for x in range(x0, x1, step):
            p = px[x, y]
            l = (p[0]+p[1]+p[2])/3
            ch = ' '
            if l > 200: ch = 'W'
            elif p[0] > 150 and p[1] < 100: ch = 'R'
            elif p[0] > 150 and p[1] > 120 and p[2] < 110: ch = 'Y'
            elif p[2] > p[0]+20 and p[0] < 130: ch = 'B'
            elif l > 100: ch = '.'
            elif l > 40: ch = ':'
            else: ch = ' '
            line += ch
        print(line)
print('--- title area (user view) x 700-1900, y 440-600 ---')
ascii_region(700, 440, 1900, 600)
