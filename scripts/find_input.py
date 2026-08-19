
from PIL import Image
img = Image.open(r'D:\Rust\steel-front\scripts\qianwen_cap.png').convert('RGB')
w, h = img.size
px = img.load()
print('--- bottom region row brightness ---')
for y in range(h - 250, h, 15):
    row = []
    for x in range(0, w, 20):
        r, g, b = px[x, y]
        row.append(int((r+g+b)/3))
    bright = sum(1 for v in row if v > 80)
    avg = sum(row)/len(row)
    print(f'y={y} avg={avg:5.1f} bright_cells={bright}/{len(row)}')
best_y = None; best_avg = 0
for y in range(h - 250, h, 5):
    vals = []
    for x in range(0, w, 10):
        r, g, b = px[x, y]
        vals.append((r+g+b)/3)
    avg = sum(vals)/len(vals)
    if avg > best_avg: best_avg = avg; best_y = y
print(f'--- brightest row: y={best_y} avg={best_avg:.1f} ---')
if best_y is not None:
    xs = []
    for x in range(0, w, 10):
        r, g, b = px[x, best_y]
        if (r+g+b)/3 > 100: xs.append(x)
    if xs:
        print(f'bright x range: {min(xs)}..{max(xs)}')
