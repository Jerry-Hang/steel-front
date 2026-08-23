# -*- coding: utf-8 -*-
import os, shutil
folder = r'C:/Users/Jerry-Huang/Desktop/钢铁前线图片文件夹'
targets = ['1-1.png', '1-2.png', '1-3.png', 'AK-12M.png']
for t in targets:
    src = os.path.join(folder, t)
    if os.path.exists(src):
        shutil.copy(src, 'D:/Rust/steel-front/screenshots/' + t)
        print('copied', t)
    else:
        print('MISSING', t)