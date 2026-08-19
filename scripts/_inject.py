import ctypes, time
from ctypes import wintypes
u = ctypes.WinDLL('user32')
u.FindWindowW.restype = wintypes.HWND
h = u.FindWindowW(None, 'Steel Front - Vulkan')
print('hwnd', h)
if not h: raise SystemExit
u.ShowWindow(h, 9)
r1 = u.SetForegroundWindow(h)
print('SetForegroundWindow', r1)
time.sleep(0.5)
u.keybd_event(0x20, 0x39, 0, 0)
time.sleep(0.08)
u.keybd_event(0x20, 0x39, 2, 0)
print('space sent')
time.sleep(0.5)
u.mouse_event(2, 0, 0, 0, 0)
time.sleep(0.05)
u.mouse_event(4, 0, 0, 0, 0)
print('lmb sent')
time.sleep(2)