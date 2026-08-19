
from PIL import Image, ImageChops
a = Image.open(r'D:\Rust\steel-front\scripts\dsh_before.png').convert('RGB')
b = Image.open(r'D:\Rust\steel-front\scripts\dsh_after.png').convert('RGB')
diff = ImageChops.difference(a, b)
w, h = diff.size
pd = diff.load()
changed = 0; total = 0
blocks = {}
for y in range(0, h, 8):
    for x in range(0, w, 8):
        r, g, bl = pd[x, y]
        total += 1
        if r + g + bl > 30:
            changed += 1
            bx, by = x // 100 * 100, y // 100 * 100
            blocks[(bx, by)] = blocks.get((bx, by), 0) + 1
print(f'changed: {changed/total*100:.2f}%')
for (bx, by), c in sorted(blocks.items(), key=lambda kv: -kv[1])[:6]:
    print(f'  block ({bx},{by}): {c}px')
