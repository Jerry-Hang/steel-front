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
strip(150, 'y=150 :')
strip(400, 'y=400 :')
strip(700, 'y=700 :')
strip(900, 'y=900 :')
strip(1200, 'y=1200:')
strip(1500, 'y=1500:')
# sky clear color
sky = 0
for y in range(0, h, 4):
    for x in range(0, w, 8):
        p = px[x, y]
        if abs(p[0]-25)<10 and abs(p[1]-25)<10 and abs(p[2]-38)<12: sky += 1
print('sky px:', sky)
# gun green cubes (TEMP 10 parts) - where?
greens = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[1] > 200 and p[0] < 80 and p[2] < 80:
            greens.append((x, y))
if greens:
    xs = [g[0] for g in greens]; ys = [g[1] for g in greens]
    print('green cubes:', len(greens), 'bbox:', (min(xs), min(ys)), '-', (max(xs), max(ys)))
else:
    print('green cubes: none')