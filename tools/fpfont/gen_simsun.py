# -*- coding: utf-8 -*-
# 一次性提取 SimSun(宋体) 12px 硬边位图 → cjk_glyphs.rs（构建时，运行时纯查表）
import ctypes

gdi32 = ctypes.windll.gdi32
user32 = ctypes.windll.user32

class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [('biSize', ctypes.c_uint32), ('biWidth', ctypes.c_int32),
                ('biHeight', ctypes.c_int32), ('biPlanes', ctypes.c_uint16),
                ('biBitCount', ctypes.c_uint16), ('biCompression', ctypes.c_uint32),
                ('biSizeImage', ctypes.c_uint32), ('biXPelsPerMeter', ctypes.c_int32),
                ('biYPelsPerMeter', ctypes.c_int32), ('biClrUsed', ctypes.c_uint32),
                ('biClrImportant', ctypes.c_uint32)]

class RECT(ctypes.Structure):
    _fields_ = [('left', ctypes.c_long), ('top', ctypes.c_long), ('right', ctypes.c_long), ('bottom', ctypes.c_long)]

def extract_char(hdc, hbmp, bmi, px, ch, size=12):
    brush = gdi32.CreateSolidBrush(0)
    r = RECT(0, 0, size, size)
    user32.FillRect(hdc, ctypes.byref(r), brush)
    gdi32.DeleteObject(brush)
    txt = (ctypes.c_wchar * 2)(ch)
    gdi32.TextOutW(hdc, 0, 0, txt, 1)
    gdi32.GetDIBits(hdc, hbmp, 0, size, px, ctypes.byref(bmi), 0)
    rows = []
    for y in range(size):
        v = 0
        for x in range(size):
            off = (y*size + x)*4
            if px[off] > 128 or px[off+1] > 128 or px[off+2] > 128:
                v |= 1 << (11 - x)
        rows.append(v)
    return rows

def main():
    size = 12
    hdc_s = user32.GetDC(0)
    hdc = gdi32.CreateCompatibleDC(hdc_s)
    user32.ReleaseDC(0, hdc_s)
    hfont = gdi32.CreateFontW(-size, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 3, 0, 'SimSun')
    gdi32.SelectObject(hdc, hfont)
    gdi32.SetBkMode(hdc, 2)
    gdi32.SetBkColor(hdc, 0x000000)
    gdi32.SetTextColor(hdc, 0xFFFFFF)
    hbmp = gdi32.CreateCompatibleBitmap(hdc, size, size)
    gdi32.SelectObject(hdc, hbmp)
    bmi = BITMAPINFOHEADER()
    bmi.biSize = 40; bmi.biWidth = size; bmi.biHeight = -size
    bmi.biPlanes = 1; bmi.biBitCount = 32
    px = (ctypes.c_ubyte * (size*size*4))()
    chars = {}
    # 覆盖：CJK 标点/假名 + 统一表意 + 全角形式
    ranges = [(0x3000, 0x30FF), (0x4E00, 0x9FFF), (0xFF00, 0xFFEF)]
    for (lo, hi) in ranges:
        for cp in range(lo, hi + 1):
            ch = chr(cp)
            try:
                rows = extract_char(hdc, hbmp, bmi, px, ch)
            except Exception:
                continue
            if any(rows):
                # 内容垂直拉伸占满 12 行（对齐渲染基线）
                top = 12; bottom = -1
                for j in range(size):
                    if rows[j] != 0:
                        top = min(top, j); bottom = j
                if top < bottom and (bottom - top + 1) < size:
                    h = bottom - top + 1
                    st = [0]*size
                    for j in range(size):
                        st[j] = rows[top + j*h//size]
                    rows = st
                chars[cp] = rows
    gdi32.DeleteObject(hbmp); gdi32.DeleteObject(hfont); gdi32.DeleteDC(hdc)
    print('extracted:', len(chars))
    items = sorted(chars.items())
    out = []
    out.append('//! 预烘焙 12x12 中文像素点阵（SimSun 宋体 12px 硬边位图）')
    out.append('//! 提取自 Windows 系统字体 SimSun（构建时一次性提取，运行时纯查表）')
    out.append('pub static CJK_GLYPHS: &[(char, [u16; 12])] = &[')
    lines = []
    for cp, rows in items:
        rb = ', '.join('0x%03X' % r for r in rows)
        lines.append(('    (' + chr(39) + chr(92) + 'u{%04X}' + chr(39) + ', [' + '%s' + ']),') % (cp, rb))
    out.append('\n'.join(lines))
    out.append('];')
    open(r'D:\\Rust\\steel-front\\src\\engine\\cjk_glyphs.rs', 'w', encoding='utf-8').write('\n'.join(out))
    print('written ok')

main()