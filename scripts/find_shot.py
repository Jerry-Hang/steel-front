# -*- coding: utf-8 -*-
import glob, os, shutil
folder = r'C:\Users\Jerry-Huang\Desktop\钢铁前线图片文件夹'
files = sorted(glob.glob(os.path.join(folder, '*.png')), key=os.path.getmtime, reverse=True)
print('files:', [os.path.basename(f) for f in files])
if files:
    src = files[0]
    print('using:', os.path.basename(src))
    # 缩图（PIL 可能没有——用简单方式：只复制，read 时超限再处理）
    shutil.copy(src, r'D:\Rust\steel-front\screenshots\launcher_issue.png')
    print('copied, size:', os.path.getsize(src))