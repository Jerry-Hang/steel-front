# -*- coding: utf-8 -*-
import glob, os
folder = r'C:\Users\Jerry-Huang\Desktop\钢铁前线图片文件夹'
files = sorted(glob.glob(os.path.join(folder, '*')), key=os.path.getmtime)
for f in files:
    print(os.path.basename(f), os.path.getsize(f))