from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def dom(y0, y1):
    c = Counter()
    for y in range(y0, y1, 4):
        for x in range(0, w, 8):
            p = px[x, y]
            c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    return c.most_common(2)
print('top    :', dom(100, 500))
print('bottom :', dom(1200, 1560))
wal = 0
for y in range(900, 1600, 2):
    for x in range(1000, 2500, 2):
        p = px[x, y]
        if abs(p[0]-189)<28 and abs(p[1]-156)<28 and abs(p[2]-108)<28: wal += 1
print('walnut bottom-right:', wal)