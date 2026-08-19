from PIL import Image
from collections import Counter
# 对比: 用户截图 vs 我的 game2_shot —— 绿色(树/植被)与纯色(天空?)的分布
def analyze(path, label):
    im = Image.open(path).convert('RGB')
    w, h = im.size
    px = im.load()
    # 绿色植被分布（按水平条带）
    bands = []
    for k in range(8):
        y0, y1 = int(h*k/8), int(h*(k+1)/8)
        green = 0; uniform = 0; detail = 0
        for y in range(y0, y1, 4):
            for x in range(0, w, 8):
                p = px[x, y]
                if p[1] > 100 and p[1] > p[0] + 25 and p[1] > p[2] + 25: green += 1
        bands.append(green)
    print(label, '绿色条带分布(上->下, 每带8分之1):', bands)
    # 全图颜色分布
    c = Counter()
    for y in range(0, h, 6):
        for x in range(0, w, 6):
            p = px[x, y]
            c[(p[0]//32, p[1]//32, p[2]//32)] += 1
    print(label, '全图top5:', c.most_common(5))
analyze('D:/Rust/Rust_Vulkan_3D/.dsh-vision-toolkit/tmp/pasted-images/1a140e6de555011627c5/b53f7939-8525-402c-aa58-3b4e07562278-image.png', '用户截图:')
analyze('D:/Rust/steel-front/game2_shot.png', '我的截图:')