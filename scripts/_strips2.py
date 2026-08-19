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
    print(label, c.most_common(4))
strip(100, 'y=100 :')
strip(300, 'y=300 :')
strip(500, 'y=500 :')
strip(700, 'y=700 :')
strip(900, 'y=900 :')
strip(1100, 'y=1100:')
strip(1300, 'y=1300:')
strip(1500, 'y=1500:')
# sky clear color (25,25,38) anywhere?
sky = 0
for y in range(0, h, 4):
    for x in range(0, w, 8):
        p = px[x, y]
        if abs(p[0]-25)<10 and abs(p[1]-25)<10 and abs(p[2]-38)<12: sky += 1
print('sky px:', sky)