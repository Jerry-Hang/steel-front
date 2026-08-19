from PIL import Image
im = Image.open('D:/Rust/steel-front/play2_shot.png').convert('RGB')
w, h = im.size
px = im.load()
# STRICT red: r>170, g<110, b<90 (marker wall red, enemy red, gun muzzle etc.)
# STRICT blue: b>180, r<90, g<130 (npc blue)
reds = []; blues = []
for y in range(0, h, 2):
    for x in range(0, w, 2):
        p = px[x, y]
        if p[0] > 170 and p[1] < 110 and p[2] < 90:
            reds.append((x, y, p))
        elif p[2] > 180 and p[0] < 90 and p[1] < 130:
            blues.append((x, y, p))
print('strict red :', len(reds), reds[:5])
print('strict blue:', len(blues), blues[:5])