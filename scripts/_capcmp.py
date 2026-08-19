from PIL import Image
a = Image.open('D:/Rust/steel-front/cap_win.png').convert('RGB')
b = Image.open('D:/Rust/steel-front/cap_screen.png').convert('RGB')
wa, ha = a.size
wb, hb = b.size
print('window:', wa, ha, ' screen:', wb, hb)
pa, pb = a.load(), b.load()
diff = 0; total = 0
for y in range(0, min(ha, hb), 6):
    for x in range(0, min(wa, wb), 6):
        total += 1
        p1, p2 = pa[x, y], pb[x, y]
        if abs(p1[0]-p2[0]) + abs(p1[1]-p2[1]) + abs(p1[2]-p2[2]) > 40: diff += 1
print('diff ratio: %.1f%%' % (diff / total * 100))
def white_region(im, x0, y0, x1, y1):
    p = im.load(); n = 0
    for y in range(y0, y1, 2):
        for x in range(x0, x1, 2):
            q = p[x, y]
            if q[0] > 220 and q[1] > 220 and q[2] > 220: n += 1
    return n
for name, im in [('window', a), ('screen', b)]:
    hh = im.size[1]; ww = im.size[0]
    print(name, 'top-left-white:', white_region(im, 0, 0, 500, 150), ' bot-left-white:', white_region(im, 0, hh-150, 500, hh))