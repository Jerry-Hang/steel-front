import ctypes, struct, sys, time
from ctypes import wintypes
u = ctypes.WinDLL('user32')
g = ctypes.WinDLL('gdi32')
u.SetProcessDPIAware()
u.FindWindowW.restype = wintypes.HWND
TITLE = 'Steel Front - Vulkan'
# find window with retry
h = None
for i in range(30):
    h = u.FindWindowW(None, TITLE)
    if h: break
    time.sleep(0.5)
if not h:
    print('NO WINDOW'); sys.exit(1)
print('window found:', h)
# bring to foreground + press SPACE to start game
u.SetForegroundWindow(h)
time.sleep(0.4)
u.keybd_event(0x20, 0, 0, 0)   # VK_SPACE down
time.sleep(0.06)
u.keybd_event(0x20, 0, 2, 0)   # up
print('space sent; waiting for map load...')
time.sleep(7)
# screenshot
r = wintypes.RECT()
u.GetWindowRect(h, ctypes.byref(r))
w, hh = r.right - r.left, r.bottom - r.top
print('window:', w, 'x', hh)
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
with open(r'D:\Rust\steel-front\game_shot.bmp', 'wb') as f:
    f.write(bmp_header); f.write(dib); f.write(buf.raw)
from PIL import Image
Image.open(r'D:\Rust\steel-front\game_shot.bmp').save(r'D:\Rust\steel-front\game_shot.png')
print('saved', w, 'x', hh)