from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# sample a horizontal strip at y=300 (top area) and y=1300 (bottom area)
def strip(y, label):
    from collections import Counter
    c = Counter()
    for x in range(0, w, 8):
        p = px[x, y]
        c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    print(label, c.most_common(3))
strip(300, 'y=300 :')
strip(800, 'y=800 :')
strip(1300, 'y=1300:')
strip(1550, 'y=1550:')