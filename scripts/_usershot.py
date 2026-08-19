from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/Rust_Vulkan_3D/.dsh-vision-toolkit/tmp/pasted-images/1a140e6de555011627c5/b53f7939-8525-402c-aa58-3b4e07562278-image.png').convert('RGB')
w, h = im.size
px = im.load()
print('size:', w, 'x', h)
def strip(y, label):
    c = Counter()
    for x in range(0, w, 8):
        p = px[x, y]
        c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    print(label, c.most_common(3))
for y in [int(h*0.05), int(h*0.15), int(h*0.3), int(h*0.5), int(h*0.65), int(h*0.85), int(h*0.95)]:
    strip(y, 'y=%.2f:' % (y/h))
def skyish(y0, y1):
    n = 0
    for y in range(y0, y1, 3):
        for x in range(0, w, 6):
            p = px[x, y]
            if p[2] > p[0] + 5 and p[0] < 120 and p[2] < 200: n += 1
    return n
print('skyish top    :', skyish(int(h*0.05), int(h*0.35)))
print('skyish bottom :', skyish(int(h*0.65), int(h*0.95)))
def greenish(y0, y1):
    n = 0
    for y in range(y0, y1, 3):
        for x in range(0, w, 6):
            p = px[x, y]
            if p[1] > 100 and p[1] > p[0] + 20 and p[1] > p[2] + 20: n += 1
    return n
print('green top    :', greenish(int(h*0.05), int(h*0.35)))
print('green bottom :', greenish(int(h*0.65), int(h*0.95)))
cx, cy = w//2, h//2
bright = 0
for y in range(cy-60, cy+60):
    for x in range(cx-60, cx+60):
        p = px[x, y]
        if p[0]+p[1]+p[2] > 450: bright += 1
print('center bright (crosshair?):', bright)