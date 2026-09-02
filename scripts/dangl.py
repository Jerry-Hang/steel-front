# -*- coding: utf-8 -*-
import io
p = 'src/engine/assets.rs'
s = io.open(p, encoding='utf-8').read()
old = """// ---------------------------------------------------------------------------
// PNG/JPEG 解码：Windows GDI+（系统组件，零外部库）→ RGBA8
// 复用 launcher wallpaper.rs 的 GdiplusStartup 模式；LockBits 取 32bpp ARGB
// ---------------------------------------------------------------------------
#[cfg(windows)]
"""
new = ""
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
    print('dangling removed')
else:
    print('miss')
