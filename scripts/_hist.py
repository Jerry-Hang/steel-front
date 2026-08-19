from PIL import Image
from collections import Counter
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# gun expected: bottom-center-right. Bucket 32 and count distinct colors
c = Counter()
for y in range(900, 1560, 3):
    for x in range(950, 2400, 3):
        p = px[x, y]
        c[(p[0]//32, p[1]//32, p[2]//32)] += 1
print('top 20 color buckets in gun region:')
for k, v in c.most_common(20):
    print(' ', k, '->', v, '≈ rgb', (k[0]*32+16, k[1]*32+16, k[2]*32+16))