
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
px = im.load()
n = 0
for y in range(1000, 1180):
    for x in range(700, 1900):
        p = px[x, y]
        if p[0] > 200 and p[1] > 200 and p[2] > 200:
            n += 1
print('white pixels title zone:', n)
