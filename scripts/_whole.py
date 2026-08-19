from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/steel-front/game2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
c = Counter()
for y in range(0, h, 6):
    for x in range(0, w, 6):
        p = px[x, y]
        c[(p[0]//32, p[1]//32, p[2]//32)] += 1
print('whole image:', c.most_common(6))
print('center px:', px[w//2, h//2])