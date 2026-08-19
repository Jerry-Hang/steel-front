from PIL import Image
im = Image.open('D:/Rust/steel-front/show_me_gun_v4.png').convert('RGB')
w, h = im.size
px = im.load()
def cnt(x0, y0, x1, y1, label, pred):
    n = 0
    for y in range(y0, y1, 2):
        for x in range(x0, x1, 2):
            p = px[x, y]
            if pred(p): n += 1
    print(label, ':', n)
# 全屏扫描 walnut 与 steel
cnt(0, 0, 2560, 1600, 'walnut anywhere', lambda p: p[0] > 110 and 70 < p[1] < 200 and p[2] < 150 and p[0] > p[2] + 35)
cnt(0, 0, 2560, 1600, 'steel anywhere', lambda p: abs(p[0]-p[1]) < 30 and abs(p[1]-p[2]) < 30 and 90 < p[0] < 200)