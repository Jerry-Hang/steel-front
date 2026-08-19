
from PIL import Image
img = Image.open(r'D:\Rust\steel-front\scripts\dsh_cap.png').convert('RGB')
w, h = img.size
px = img.load()
# y=820-900 亮列范围
for y in range(820, 905, 10):
    xs = []
    for x in range(0, w, 5):
        r, g, b = px[x, y]
        if (r+g+b)/3 > 60:
            xs.append(x)
    if xs:
        print(f'y={y}: bright x {min(xs)}..{max(xs)} count={len(xs)}')
    else:
        print(f'y={y}: none')
# y=975-1000
print('--- bottom ---')
for y in range(975, 1005, 8):
    xs = []
    for x in range(0, w, 5):
        r, g, b = px[x, y]
        if (r+g+b)/3 > 60:
            xs.append(x)
    if xs:
        print(f'y={y}: bright x {min(xs)}..{max(xs)} count={len(xs)}')
    else:
        print(f'y={y}: none')
