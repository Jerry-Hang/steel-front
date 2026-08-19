
from PIL import Image
img = Image.open(r'D:\Rust\steel-front\scripts\qianwen_cap.png').convert('RGB')
w, h = img.size
px = img.load()
# 统计亮度分布
import statistics
samples = []
bright = 0
dark = 0
for y in range(0, h, 10):
    for x in range(0, w, 10):
        r, g, b = px[x, y]
        lum = (r + g + b) / 3
        samples.append(lum)
        if lum > 150: bright += 1
        elif lum < 60: dark += 1
total = len(samples)
print(f'size: {w}x{h}')
print(f'avg lum: {sum(samples)/len(samples):.1f}')
print(f'bright%: {bright/total*100:.1f}  dark%: {dark/total*100:.1f}')
# 检查是否有明显 UI 结构（行间亮度变化）
row_avg = []
for y in range(0, h, 25):
    s = 0; n = 0
    for x in range(0, w, 8):
        r, g, b = px[x, y]
        s += (r+g+b)/3; n += 1
    row_avg.append(s/n)
# 打印行亮度概览
for i, v in enumerate(row_avg):
    bar = '#' * int(v / 10)
    print(f'y={i*25:4d} lum={v:5.1f} {bar}')
