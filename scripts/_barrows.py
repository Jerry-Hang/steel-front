from PIL import Image
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# HP bar expected at (48,1508)-(768,1552): dark back (0.08,0.08,0.10,0.75) over scene
# sample the exact bar rows vs rows above it
def rowavg(y):
    tot = [0,0,0]; n = 0
    for x in range(48, 768, 4):
        p = px[x, y]
        tot[0]+=p[0]; tot[1]+=p[1]; tot[2]+=p[2]; n+=1
    return [round(t/n) for t in tot]
for y in [1500, 1510, 1520, 1530, 1540, 1550, 1560, 1570]:
    print('y=', y, rowavg(y))