from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
def match(p, t, tol): return all(abs(p[i]-t[i]) <= tol for i in range(3))
targets = [('marker_red', (217, 64, 38), 70), ('npc_blue', (20, 89, 250), 70), ('walnut', (133, 87, 38), 45)]
for label, t, tol in targets:
    n = 0; locs = []
    for y in range(0, h, 3):
        for x in range(0, w, 3):
            p = px[x, y]
            if match(p, t, tol):
                n += 1
                if len(locs) < 3: locs.append((x, y, p))
    print(label, ':', n, locs)