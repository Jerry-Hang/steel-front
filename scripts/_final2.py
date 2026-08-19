from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
wal = 0; walpos = []
for y in range(900, 1600, 2):
    for x in range(1000, 2500, 2):
        p = px[x, y]
        if abs(p[0]-189)<28 and abs(p[1]-156)<28 and abs(p[2]-108)<28:
            wal += 1
            if len(walpos) < 2: walpos.append((x, y))
print('walnut bottom:', wal, walpos)
# green trees at bottom (ground) vs top
def green(y0, y1):
    n = 0
    for y in range(y0, y1, 3):
        for x in range(0, w, 6):
            p = px[x, y]
            if p[1] > 120 and p[1] > p[0] + 30 and p[1] > p[2] + 30: n += 1
    return n
print('green top(100-500):', green(100, 500), ' green bottom(1100-1500):', green(1100, 1500))