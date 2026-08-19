from PIL import Image
im = Image.open('D:/Rust/steel-front/show_me_gun_v2.png').convert('RGB')
w, h = im.size
px = im.load()
def cnt(x0, y0, x1, y1, label, pred):
    n = 0
    for y in range(y0, y1, 2):
        for x in range(x0, x1, 2):
            p = px[x, y]
            if pred(p): n += 1
    print(label, ':', n)
cnt(1000, 900, 2560, 1600, 'walnut-ish BR', lambda p: p[0] > 120 and 80 < p[1] < 190 and p[2] < 140 and p[0] > p[2] + 40)
cnt(1000, 900, 2560, 1600, 'steel-ish BR', lambda p: abs(p[0]-p[1]) < 25 and abs(p[1]-p[2]) < 25 and 90 < p[0] < 190)
cnt(0, 0, 500, 150, 'top-left white', lambda p: p[0] > 220 and p[1] > 220 and p[2] > 220)