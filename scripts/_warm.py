from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# count saturated warm colors (r > g > b, r-b > 30) in bottom-center area
warm = 0; warm_locs = []
for y in range(h-550, h-60, 2):
    for x in range(w//2-450, w//2+900, 2):
        p = px[x, y]
        if p[0] > p[1] > p[2] and p[0] - p[2] > 30 and p[0] > 60:
            warm += 1
            if len(warm_locs) < 8: warm_locs.append((x, y, p))
print('warm px:', warm, warm_locs[:6])