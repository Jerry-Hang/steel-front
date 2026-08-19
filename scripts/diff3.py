
from PIL import Image, ImageChops
base = Image.open(r'D:\Rust\steel-front\scripts\qw_b3.png').convert('RGB')
aft = Image.open(r'D:\Rust\steel-front\scripts\qw_a3.png').convert('RGB')
diff = ImageChops.difference(base, aft)
w, h = diff.size
pd = diff.load()
changed = 0; total = 0
for y in range(0, h, 8):
    for x in range(0, w, 8):
        r, g, b = pd[x, y]
        total += 1
        if r + g + b > 30: changed += 1
print(f'changed: {changed/total*100:.2f}%')
