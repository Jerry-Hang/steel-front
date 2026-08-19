from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def strip(y, label):
    c = Counter()
    for x in range(0, w, 8):
        p = px[x, y]
        c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    print(label, c.most_common(3))
for y in [100, 300, 500, 700, 900, 1100, 1300, 1500]:
    strip(y, 'y=%d:' % y)
wal = 0
for y in range(900, 1600, 2):
    for x in range(1000, 2500, 2):
        p = px[x, y]
        if abs(p[0]-189)<28 and abs(p[1]-156)<28 and abs(p[2]-108)<28: wal += 1
print('walnut bottom-right:', wal)
green_low = 0; green_high = 0
for y in range(800, 1600, 3):
    for x in range(0, w, 6):
        p = px[x, y]
        if p[1] > 100 and p[1] > p[0] + 25 and p[1] > p[2] + 25: green_low += 1
for y in range(0, 800, 3):
    for x in range(0, w, 6):
        p = px[x, y]
        if p[1] > 100 and p[1] > p[0] + 25 and p[1] > p[2] + 25: green_high += 1
print('green upper half:', green_high, ' green lower half:', green_low)