
from PIL import Image
img = Image.open(r'D:\Rust\steel-front\scripts\dsh_cap.png').convert('RGB')
w, h = img.size
px = img.load()
print(f'size: {w}x{h}')
# 底部 300px 行亮度
for y in range(h - 300, h, 20):
    vals = []
    for x in range(0, w, 20):
        r, g, b = px[x, y]
        vals.append((r+g+b)/3)
    avg = sum(vals)/len(vals)
    bright = sum(1 for v in vals if v > 100)
    print(f'y={y} avg={avg:5.1f} bright={bright}/{len(vals)}')
# 整个窗口的行概览（前 20 行采样）
print('--- overview ---')
for y in range(0, h, 60):
    vals = []
    for x in range(0, w, 30):
        r, g, b = px[x, y]
        vals.append((r+g+b)/3)
    avg = sum(vals)/len(vals)
    print(f'y={y:4d} avg={avg:5.1f}')
