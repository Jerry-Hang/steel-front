from PIL import Image
im = Image.open('D:/Rust/Rust_Vulkan_3D/.dsh-vision-toolkit/tmp/pasted-images/1a140e6de555011627c5/b53f7939-8525-402c-aa58-3b4e07562278-image.png').convert('RGB')
w, h = im.size
px = im.load()
def white(x0, y0, x1, y1, label):
    n = 0
    for y in range(y0, y1, 2):
        for x in range(x0, x1, 2):
            p = px[x, y]
            if p[0] > 220 and p[1] > 220 and p[2] > 220: n += 1
    print(label, ':', n)
white(0, 0, 500, 200, 'top-left-white')
white(0, h-200, 500, h, 'bot-left-white')
white(w-600, 0, w, 200, 'top-right-white')
white(w-600, h-200, w, h, 'bot-right-white')
# 绿色横条（HP fill）位置
def green(x0, y0, x1, y1, label):
    n = 0
    for y in range(y0, y1, 2):
        for x in range(x0, x1, 2):
            p = px[x, y]
            if p[1] > 150 and p[1] > p[0] + 40 and p[1] > p[2] + 40: n += 1
    print(label, ':', n)
green(0, 0, 900, 200, 'top-left-green')
green(0, h-200, 900, h, 'bot-left-green')