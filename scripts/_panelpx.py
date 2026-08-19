
from PIL import Image
im = Image.open('D:/Rust/steel-front/ui_start.png').convert('RGB')
w, h = im.size
im = im.transpose(Image.FLIP_TOP_BOTTOM)
px = im.load()
# 面板中心（用户视角）: 面板 (380,210,520,170) 设计 -> 像素 (760,420,1040,340)
# 中心 (1280,590)
for label, x, y in [('panel center',1280,590),('panel top-left inner',800,460),('panel bottom-right inner',1760,720),
                    ('subtitle zone',1600,570),('ops zone',1600,615),('title zone',1200,500)]:
    p = px[x,y]
    print('%-24s (%4d,%4d) = (%3d,%3d,%3d)' % (label,x,y,p[0],p[1],p[2]))
