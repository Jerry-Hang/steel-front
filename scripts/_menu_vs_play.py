import ctypes, struct, sys, time, subprocess, os
from ctypes import wintypes
u = ctypes.WinDLL('user32')
g = ctypes.WinDLL('gdi32')
k = ctypes.WinDLL('kernel32')
u.SetProcessDPIAware()
u.FindWindowW.restype = wintypes.HWND
TITLE = 'Steel Front - Vulkan'
OUT = r'D:\Rust\steel-front'
def shot(path, h):
    r = wintypes.RECT()
    u.GetWindowRect(h, ctypes.byref(r))
    w, hh = r.right - r.left, r.bottom - r.top
    hdc = u.GetDC(h)
    memdc = g.CreateCompatibleDC(hdc)
    bmp = g.CreateCompatibleBitmap(hdc, w, hh)
    g.SelectObject(memdc, bmp)
    g.BitBlt(memdc, 0, 0, w, hh, hdc, 0, 0, 0x00CC0020)
    class BI(ctypes.Structure):
        _fields_ = [('biSize', wintypes.DWORD), ('biWidth', ctypes.c_long), ('biHeight', ctypes.c_long),
                    ('biPlanes', wintypes.WORD), ('biBitCount', wintypes.WORD), ('biCompression', wintypes.DWORD),
                    ('biSizeImage', wintypes.DWORD), ('biXPels', ctypes.c_long), ('biYPels', ctypes.c_long),
                    ('biClrUsed', wintypes.DWORD), ('biClrImportant', wintypes.DWORD)]
    bi = BI()
    bi.biSize = ctypes.sizeof(BI)
    bi.biWidth = w
    bi.biHeight = -hh
    bi.biPlanes = 1
    bi.biBitCount = 32
    buf = ctypes.create_string_buffer(w * hh * 4)
    g.GetDIBits(memdc, bmp, 0, hh, buf, ctypes.byref(bi), 0)
    g.DeleteObject(bmp)
    g.DeleteDC(memdc)
    u.ReleaseDC(h, hdc)
    bmp_header = b'BM' + struct.pack('<IHHI', 54 + len(buf.raw), 0, 0, 54)
    dib = struct.pack('<IiiHHIIiiII', 40, w, hh, 1, 32, 0, len(buf.raw), 0, 0, 0, 0)
    open(path + '.bmp', 'wb').write(bmp_header + dib + buf.raw)
    from PIL import Image
    Image.open(path + '.bmp').save(path + '.png')
    return w, hh
# kill old
subprocess.run(['taskkill', '/F', '/IM', 'steel-front.exe'], capture_output=True)
time.sleep(1)
p = subprocess.Popen([OUT + r'\target\release\steel-front.exe'], cwd=OUT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
# wait for window
h = None
for i in range(40):
    h = u.FindWindowW(None, TITLE)
    if h: break
    time.sleep(0.5)
if not h: print('NO WINDOW'); p.kill(); sys.exit(1)
print('window found after wait')
time.sleep(5)
print('menu shot:', shot(OUT + r'\menu_shot', h))
# focus + start
fg = u.GetForegroundWindow()
print('foreground before:', hex(fg), 'target:', hex(h))
u.SetForegroundWindow(h)
time.sleep(0.5)
fg = u.GetForegroundWindow()
print('foreground after :', hex(fg))
u.keybd_event(0x20, 0, 0, 0)
time.sleep(0.08)
u.keybd_event(0x20, 0, 2, 0)
print('space sent')
time.sleep(9)
print('play shot:', shot(OUT + r'\play_shot', h))
# compare
from PIL import Image
a = Image.open(OUT + r'\menu_shot.png').convert('RGB')
b = Image.open(OUT + r'\play_shot.png').convert('RGB')
wa, ha = a.size
pa, pb = a.load(), b.load()
changed = 0
total = 0
for y in range(0, ha, 8):
    for x in range(0, wa, 8):
        total += 1
        pa_ = pa[x, y]; pb_ = pb[x, y]
        if abs(pa_[0]-pb_[0]) + abs(pa_[1]-pb_[1]) + abs(pa_[2]-pb_[2]) > 30:
            changed += 1
print(f'changed px ratio: {changed}/{total} = {changed/total*100:.1f}%')
# check playing HUD
def bright_count(im, cx, cy, rng, thr=450):
    p = im.load(); n = 0
    for y in range(cy-rng, cy+rng):
        for x in range(cx-rng, cx+rng):
            p_ = p[x, y]
            if p_[0]+p_[1]+p_[2] > thr: n += 1
    return n
print('play center bright:', bright_count(b, wa//2, ha//2, 60))
p.kill()
print('done')