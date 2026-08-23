# -*- coding: utf-8 -*-
import os, shutil
folder = r'C:/Users/Jerry-Huang/Desktop/钢铁前线图片文件夹'
for t in ['1-4.png', '1-5.png']:
    src = os.path.join(folder, t)
    if os.path.exists(src):
        shutil.copy(src, 'D:/Rust/steel-front/screenshots/' + t)
        print('copied', t, os.path.getsize(src))
    else:
        print('MISSING', t)