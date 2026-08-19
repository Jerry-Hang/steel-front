import ctypes, struct, sys, time, subprocess
from ctypes import wintypes
u = ctypes.WinDLL('user32')
g = ctypes.WinDLL('gdi32')
u.SetProcessDPIAware()
u.FindWindowW.restype = wintypes.HWND
TITLE = 'Steel Front - Vulkan'
OUT = 'D:/Rust/steel-front'
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
    open(path + '.bmp', 'wb').write(b'BM' + struct.pack('<IHHI', 54 + len(buf.raw), 0, 0, 54)
        + struct.pack('<IiiHHIIiiII', 40, w, hh, 1, 32, 0, len(buf.raw), 0, 0, 0, 0) + buf.raw)
    from PIL import Image
    Image.open(path + '.bmp').save(path + '.png')
h = None
for i in range(40):
    h = u.FindWindowW(None, TITLE)
    if h: break
    time.sleep(0.5)
if not h: print('NO WINDOW'); sys.exit(1)
time.sleep(4)
u.SetForegroundWindow(h)
time.sleep(0.3)
u.keybd_event(0x20, 0, 0, 0)
time.sleep(0.08)
u.keybd_event(0x20, 0, 2, 0)
time.sleep(6)
# hold right mouse = ADS
u.mouse_event(0x0008, 0, 0, 0, 0)  # RIGHTDOWN
time.sleep(1.2)
print('ads shot:', shot(OUT + '/ads_shot', h))
time.sleep(1.0)
u.mouse_event(0x0010, 0, 0, 0, 0)  # RIGHTUP
print('ads released')