
import ctypes, struct, sys, time
from ctypes import wintypes
u = ctypes.WinDLL('user32')
g = ctypes.WinDLL('gdi32')
try: u.SetProcessDPIAware()
except Exception: pass

INPUT_MOUSE = 0; INPUT_KEYBOARD = 1
KEYEVENTF_KEYUP = 0x0002; KEYEVENTF_SCANCODE = 0x0008
class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", wintypes.LONG), ("dy", wintypes.LONG), ("mouseData", wintypes.DWORD),
                ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.POINTER(wintypes.ULONG))]
class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wintypes.WORD), ("wScan", wintypes.WORD), ("dwFlags", wintypes.DWORD),
                ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.POINTER(wintypes.ULONG))]
class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [("uMsg", wintypes.DWORD), ("wParamL", wintypes.WORD), ("wParamH", wintypes.WORD)]
class INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]
class INPUT(ctypes.Structure):
    _fields_ = [("type", wintypes.DWORD), ("union", INPUTUNION)]
def send_key(scancode, down):
    inp = INPUT(); inp.type = INPUT_KEYBOARD
    inp.union.ki.wVk = 0; inp.union.ki.wScan = scancode
    inp.union.ki.dwFlags = KEYEVENTF_SCANCODE | (KEYEVENTF_KEYUP if not down else 0)
    u.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT)); time.sleep(0.03)
def press(sc):
    send_key(sc, True); time.sleep(0.08); send_key(sc, False)
SC_SPACE = 0x39; SC_TAB = 0x0F; SC_ESC = 0x01

u.FindWindowW.restype = wintypes.HWND
h = u.FindWindowW(None, 'Steel Front - Vulkan')
if not h: print('NO WINDOW'); sys.exit(1)
u.SetForegroundWindow(h); time.sleep(0.4)

def shot(name):
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
    path = 'D:/Rust/steel-front/' + name
    with open(path + '.bmp', 'wb') as f:
        f.write(b'BM' + struct.pack('<IHHI', 54 + len(buf.raw), 0, 0, 54))
        f.write(struct.pack('<IiiHHIIiiII', 40, w, hh, 1, 32, 0, len(buf.raw), 0, 0, 0, 0))
        f.write(buf.raw)
    try:
        from PIL import Image
        Image.open(path + '.bmp').save(path + '.png')
    except Exception:
        pass
    print('shot', name, w, 'x', hh, flush=True)

shot('ui_start')
press(SC_SPACE); time.sleep(1.5)
press(SC_ESC); time.sleep(0.5)
shot('ui_esc')
press(SC_ESC); time.sleep(0.3)
press(SC_TAB); time.sleep(0.5)
shot('ui_settings')
print('DONE')
