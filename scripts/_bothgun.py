from PIL import Image
def gunscan(path, label):
    im = Image.open(path).convert('RGB')
    w, h = im.size
    px = im.load()
    res = {}
    for name, t, tol in [('walnut', (189,156,108), 28), ('steel', (172,179,186), 20)]:
        locs = []
        for y in range(0, h, 2):
            for x in range(0, w, 2):
                p = px[x, y]
                if abs(p[0]-t[0])<tol and abs(p[1]-t[1])<tol and abs(p[2]-t[2])<tol:
                    locs.append((x, y))
        if locs:
            xs = [l[0] for l in locs]; ys = [l[1] for l in locs]
            res[name] = (len(locs), (min(xs), min(ys)), (max(xs), max(ys)))
        else:
            res[name] = (0, None, None)
    print(label, res)
gunscan('D:/Rust/steel-front/game2_shot.png', 'HIP:')
gunscan('D:/Rust/steel-front/ads_shot.png', 'ADS:')