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
            if len(walpos) < 1: walpos.append((x, y))
print('walnut bottom:', wal, walpos)
top_blue = 0
for y in range(100, 400, 4):
    for x in range(0, w, 8):
        p = px[x, y]
        if p[2] > p[0] + 5 and p[0] < 70: top_blue += 1
print('bluish top:', top_blue)