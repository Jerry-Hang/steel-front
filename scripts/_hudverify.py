from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def rowavg(y):
    tot = [0,0,0]; n = 0
    for x in range(48, 768, 4):
        p = px[x, y]
        tot[0]+=p[0]; tot[1]+=p[1]; tot[2]+=p[2]; n+=1
    return [round(t/n) for t in tot]
print('HP bar rows (bottom-left):')
for y in [1500, 1510, 1520, 1530, 1540, 1550, 1560, 1570]:
    print(' y=', y, rowavg(y))
n = 0
for y in range(h//2-60, h//2+60):
    for x in range(w//2-60, w//2+60):
        p = px[x, y]
        if p[0]+p[1]+p[2] > 450: n += 1
print('center bright:', n)