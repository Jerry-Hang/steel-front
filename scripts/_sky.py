from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# sky = clear color (0.1,0.1,0.15)*255 = (25,25,38) ± 8
top = 0; bottom = 0
top_locs = []; bottom_locs = []
for y in range(0, h//2, 4):
    for x in range(0, w, 8):
        p = px[x, y]
        if abs(p[0]-25) < 10 and abs(p[1]-25) < 10 and abs(p[2]-38) < 12:
            top += 1
            if len(top_locs) < 3: top_locs.append((x, y, p))
for y in range(h//2, h, 4):
    for x in range(0, w, 8):
        p = px[x, y]
        if abs(p[0]-25) < 10 and abs(p[1]-25) < 10 and abs(p[2]-38) < 12:
            bottom += 1
            if len(bottom_locs) < 3: bottom_locs.append((x, y, p))
print('sky px top   :', top, top_locs[:2])
print('sky px bottom:', bottom, bottom_locs[:2])
# also: darkest blue anywhere
dark = []
for y in range(0, h, 8):
    for x in range(0, w, 8):
        p = px[x, y]
        if p[2] > p[0] + 5 and p[0] < 60:
            dark.append((x, y, p))
if dark:
    ys = [d[1] for d in dark]
    print('bluish px:', len(dark), 'y-range:', min(ys), '-', max(ys))