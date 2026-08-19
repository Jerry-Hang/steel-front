
from PIL import Image, ImageChops
base = Image.open(r'D:\Rust\steel-front\scripts\qw_before2.png').convert('RGB')
aft = Image.open(r'D:\Rust\steel-front\scripts\qw_after2.png').convert('RGB')
diff = ImageChops.difference(base, aft)
w, h = diff.size
pd = diff.load()
changed = 0; total = 0
blocks = {}
for y in range(0, h, 10):
    for x in range(0, w, 10):
        r, g, b = pd[x, y]
        total += 1
        if r + g + b > 30:
            changed += 1
            bx, by = x // 100 * 100, y // 100 * 100
            blocks[(bx, by)] = blocks.get((bx, by), 0) + 1
print(f'changed: {changed/total*100:.2f}%')
for (bx, by), c in sorted(blocks.items(), key=lambda kv: -kv[1])[:10]:
    print(f'  block ({bx},{by}): {c}px')
