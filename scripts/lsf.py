# -*- coding: utf-8 -*-
import os, glob
folder = r'C:/Users/Jerry-Huang/Desktop/钢铁前线图片文件夹'
for f in sorted(glob.glob(os.path.join(folder, '*')), key=os.path.getmtime):
    print(os.path.basename(f), os.path.getsize(f))