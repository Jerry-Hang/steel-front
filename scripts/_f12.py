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
u.keybd_event(0x7C, 0, 0, 0)  # VK_F12
time.sleep(0.06)
u.keybd_event(0x7C, 0, 2, 0)
print('F12 sent')
time.sleep(2)