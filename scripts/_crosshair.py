from PIL import Image
im = Image.open(r'D:\Rust\steel-front\game_shot.png').convert('RGB')
w, h = im.size
px = im.load()
cx, cy = w // 2, h // 2
print('center pixel:', px[cx, cy])
# scan a 200x200 region around center for bright pixels
bright = []
for y in range(cy - 100, cy + 100, 2):
    for x in range(cx - 100, cx + 100, 2):
        p = px[x, y]
        if p[0] + p[1] + p[2] > 300:
            bright.append((x, y, p))
print('bright px count:', len(bright))
for b in bright[:12]:
    print(b)
# also check a wider horizontal strip for a line
row_bright = []
for x in range(cx - 300, cx + 300, 2):
    p = px[x, cy]
    if p[0] + p[1] + p[2] > 300:
        row_bright.append((x, p))
print('center row bright:', len(row_bright))
for b in row_bright[:12]:
    print(b)