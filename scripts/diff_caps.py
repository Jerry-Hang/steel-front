
from PIL import Image, ImageChops
a = Image.open(r'D:\Rust\steel-front\scripts\qw_before.png').convert('RGB')
b = Image.open(r'D:\Rust\steel-front\scripts\qw_after.png').convert('RGB')
diff = ImageChops.difference(a, b)
# 统计差异像素比例
w, h = diff.size
pa = a.load(); pb = b.load(); pd = diff.load()
changed = 0
total = 0
for y in range(0, h, 6):
    for x in range(0, w, 6):
        r, g, bl = pd[x, y]
        total += 1
        if r + g + bl > 30:
            changed += 1
print(f'changed ratio: {changed/total*100:.1f}%')
# 找出差异最大的区域（按 100px 块）
from collections import Counter
blocks = Counter()
for y in range(0, h, 50):
    for x in range(0, w, 50):
        s = 0
        for dy in range(0, 50, 8):
            for dx in range(0, 50, 8):
                if y+dy < h and x+dx < w:
                    r, g, bl = pd[x+dx, y+dy]
                    s += r+g+bl
        if s > 300:
            blocks[(x//50*50, y//50*50)] = s
for (bx, by), s in blocks.most_common(8):
    print(f'block at ({bx},{by}) diff={s}')
