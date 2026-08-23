# -*- coding: utf-8 -*-
import os, shutil
folder = r'C:/Users/Jerry-Huang/Desktop/钢铁前线图片文件夹'
src = os.path.join(folder, '2-1.png')
if os.path.exists(src):
    shutil.copy(src, 'D:/Rust/steel-front/screenshots/2-1.png')
    print('copied', os.path.getsize(src))
else:
    print('MISSING')