
from PIL import Image, ImageEnhance, ImageOps
img = Image.open(r'D:\Rust\steel-front\scripts\qianwen_cap.png').convert('L')
# 自动对比度 + 放大 2 倍
img = ImageOps.autocontrast(img, cutoff=1)
img = img.resize((img.width * 2, img.height * 2), Image.LANCZOS)
img.save(r'D:\Rust\steel-front\scripts\qianwen_enh.png')
print('enhanced saved', img.size)
