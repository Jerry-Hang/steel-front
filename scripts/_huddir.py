from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def white(x0, y0, x1, y1, label):
    n = 0
    for y in range(y0, y1, 2):
        for x in range(x0, x1, 2):
            p = px[x, y]
            if p[0] > 220 and p[1] > 220 and p[2] > 220: n += 1
    print(label, ':', n)
white(0, 0, 500, 150, 'top-left-white')
white(0, h-150, 500, h, 'bot-left-white')
white(w-500, 0, w, 150, 'top-right-white')
white(w-500, h-150, w, h, 'bot-right-white')
def yc(x0, y0, x1, y1, label):
    n = 0
    for y in range(y0, y1, 2):
        for x in range(x0, x1, 2):
            p = px[x, y]
            if (p[0] > 200 and p[1] > 200 and p[2] < 120) or (p[2] > 200 and p[0] < 120): n += 1
    print(label, ':', n)
yc(0, 0, 800, 200, 'top-left-yc')
yc(0, h-200, 800, h, 'bot-left-yc')
n = 0
for y in range(40, 140):
    for x in range(700, 900):
        p = px[x, y]
        if p[0] > 220 and p[1] > 220 and p[2] > 220: n += 1
print('near-greenbar-white:', n)