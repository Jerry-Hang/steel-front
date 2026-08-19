from PIL import Image
for name in ['menu_shot', 'play_shot']:
    im = Image.open('D:/Rust/steel-front/' + name + '.png').convert('RGB')
    w, h = im.size
    px = im.load()
    rows = {}
    for y in range(0, h, 8):
        n = 0
        for x in range(0, w, 8):
            p = px[x, y]
            if p[0]+p[1]+p[2] > 450: n += 1
        if n > 0: rows[y] = n
    if rows:
        ys = sorted(rows)
        bands = []
        cur = [ys[0], ys[0]]
        for y in ys[1:]:
            if y - cur[1] > 40:
                bands.append(tuple(cur)); cur = [y, y]
            else: cur[1] = y
        bands.append(tuple(cur))
        print(name, 'bright rows:', len(rows), 'y-range:', ys[0], '-', ys[-1])
        print(name, 'bands:', bands)
        for b in bands[:8]:
            xs = [x for y in range(b[0], b[1]+1, 8) for x in range(0, w, 8) if sum(px[x, y]) > 450]
            if xs: print('   band', b, 'x-range:', min(xs), '-', max(xs))
    else:
        print(name, 'NO bright rows')