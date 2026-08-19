
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
px = im.load()
# 关键：确定截图方向！开始菜单标题 STEEL FRONT 应该在哪个方向？
# 方法：找白色文字的位置与形状。若标题在截图底部 = framebuffer 底部 = 用户看到顶部？
# 直接看截图底部 y 1540-1600 的黄色文字（PRESS ANY KEY 应该是黄色）
def ascii_region(x0, y0, x1, y1, step=3):
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
print('=== bottom yellow text y 1540-1600 x 0-900 ===')
ascii_region(0, 1540, 900, 1600, 3)
