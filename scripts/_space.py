import ctypes, time, sys
from ctypes import wintypes
u = ctypes.WinDLL('user32')
u.SetProcessDPIAware()
u.FindWindowW.restype = wintypes.HWND
TITLE = 'Steel Front - Vulkan'
h = None
for i in range(40):
    h = u.FindWindowW(None, TITLE)
    if h: break
    time.sleep(0.5)
if not h: print('NO WINDOW'); sys.exit(1)
time.sleep(2)
u.SetForegroundWindow(h)
time.sleep(0.3)
u.keybd_event(0x20, 0, 0, 0)
time.sleep(0.08)
u.keybd_event(0x20, 0, 2, 0)
print('space sent')
time.sleep(6)