
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
px = im.load()
# title zone 右侧 (x 1400-1900, y 1039-1110) 里有没有白色文字像素？
n_white = 0
for y in range(1039, 1110):
    for x in range(1200, 1900):
        p = px[x, y]
        if p[0] > 200 and p[1] > 200 and p[2] > 200:
            n_white += 1
print('white text pixels in title zone right:', n_white)
# 中部 (y 1000-1160, x 760-1800) 有没有面板深色 (0.05,0.06,0.10) 叠加遮罩后 ≈ (13,15,26)*0.72 + 世界*0.28*0.28
# 遮罩 0.72 + 面板 0.72: 面板色 ≈ 0.72*(13,15,26) + 0.28*遮罩结果
dark = 0
for y in range(840, 1180):
    for x in range(760, 1800):
        p = px[x, y]
        if p[0] < 60 and p[1] < 70 and p[2] < 90:
            dark += 1
print('dark panel pixels (y840-1180 x760-1800):', dark)
