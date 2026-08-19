from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
walnut = 0; steel = 0; wal = []; stl = []
for y in range(900, 1600, 1):
    for x in range(1000, 2500, 1):
        p = px[x, y]
        if abs(p[0]-189)<28 and abs(p[1]-156)<28 and abs(p[2]-108)<28:
            walnut += 1
            if len(wal) < 1: wal.append((x, y))
        if abs(p[0]-172)<20 and abs(p[1]-179)<20 and abs(p[2]-186)<20:
            steel += 1
            if len(stl) < 1: stl.append((x, y))
print('walnut bottom:', walnut, wal)
print('steel bottom:', steel, stl)
def rowavg(y):
    tot = [0,0,0]; n = 0
    for x in range(48, 768, 4):
        p = px[x, y]
        tot[0]+=p[0]; tot[1]+=p[1]; tot[2]+=p[2]; n+=1
    return [round(t/n) for t in tot]
print('rows 1500-1560:', [rowavg(y) for y in range(1500, 1561, 10)])
n = 0
for y in range(h//2-60, h//2+60):
    for x in range(w//2-60, w//2+60):
        p = px[x, y]
        if p[0]+p[1]+p[2] > 450: n += 1
print('center bright:', n)