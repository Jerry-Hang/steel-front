
import ctypes, struct, sys, time
from ctypes import wintypes
u = ctypes.WinDLL('user32')
g = ctypes.WinDLL('gdi32')
try: u.SetProcessDPIAware()
except Exception: pass
u.FindWindowW.restype = wintypes.HWND
h = u.FindWindowW(None, 'Steel Front - Vulkan')
if not h: print('NO WINDOW'); sys.exit(1)
u.SetForegroundWindow(h); time.sleep(0.5)
r = wintypes.RECT(); u.GetWindowRect(h, ctypes.byref(r))
w, hh = r.right - r.left, r.bottom - r.top
hdc = u.GetDC(h); memdc = g.CreateCompatibleDC(hdc)
bmp = g.CreateCompatibleBitmap(hdc, w, hh)
g.SelectObject(memdc, bmp)
g.BitBlt(memdc, 0, 0, w, hh, hdc, 0, 0, 0x00CC0020)
class BI(ctypes.Structure):
    _fields_ = [('biSize', wintypes.DWORD), ('biWidth', ctypes.c_long), ('biHeight', ctypes.c_long),
                ('biPlanes', wintypes.WORD), ('biBitCount', wintypes.WORD), ('biCompression', wintypes.DWORD),
                ('biSizeImage', wintypes.DWORD), ('biXPels', ctypes.c_long), ('biYPels', ctypes.c_long),
                ('biClrUsed', wintypes.DWORD), ('biClrImportant', wintypes.DWORD)]
bi = BI(); bi.biSize = ctypes.sizeof(BI); bi.biWidth = w; bi.biHeight = -hh
bi.biPlanes = 1; bi.biBitCount = 32
buf = ctypes.create_string_buffer(w * hh * 4)
g.GetDIBits(memdc, bmp, 0, hh, buf, ctypes.byref(bi), 0)
g.DeleteObject(bmp); g.DeleteDC(memdc); u.ReleaseDC(h, hdc)
with open(r'D:/Rust/steel-front/ui_start.png'.replace('/', '\\') + '.bmp', 'wb') as f:
    f.write(b'BM' + struct.pack('<IHHI', 54 + len(buf.raw), 0, 0, 54))
    f.write(struct.pack('<IiiHHIIiiII', 40, w, hh, 1, 32, 0, len(buf.raw), 0, 0, 0, 0))
    f.write(buf.raw)
from PIL import Image
Image.open(r'D:\Rust\steel-front\ui_start.bmp').save(r'D:\Rust\steel-front\ui_start.png')
print('shot ui_start', w, 'x', hh)
