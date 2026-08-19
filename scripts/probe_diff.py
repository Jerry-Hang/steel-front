
from PIL import Image, ImageChops
base = Image.open(r'D:\Rust\steel-front\scripts\qw_base.png').convert('RGB')
for i in range(1, 9):
    try:
        p = Image.open(f'D:\\Rust\\steel-front\\scripts\\qw_probe_{i}.png').convert('RGB')
    except Exception:
        continue
    diff = ImageChops.difference(base, p)
    w, h = diff.size
    pd = diff.load()
    changed = 0; total = 0
    for y in range(0, h, 6):
        for x in range(0, w, 6):
            r, g, b = pd[x, y]
            total += 1
            if r + g + b > 30: changed += 1
    print(f'probe {i}: changed {changed/total*100:.2f}%')
