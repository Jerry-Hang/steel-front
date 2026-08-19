import ctypes, time
from ctypes import wintypes
u = ctypes.WinDLL('user32')
u.FindWindowW.restype = wintypes.HWND
h = u.FindWindowW(None, 'Steel Front - Vulkan')
print('hwnd', h)
if not h: raise SystemExit
# focus window
u.ShowWindow(h, 9)
u.SetForegroundWindow(h)
time.sleep(0.5)
# ESC open menu
u.keybd_event(0x1B, 0x01, 0, 0); time.sleep(0.08); u.keybd_event(0x1B, 0x01, 2, 0)
time.sleep(0.8)
# window rect
r = wintypes.RECT()
u.GetWindowRect(h, ctypes.byref(r))
print('rect', r.left, r.top, r.right, r.bottom)
w = r.right - r.left; hh = r.bottom - r.top
# SETTINGS option: panel py = (hh-240)/2, option1 y = py+146; physical window coords
py_panel = (hh - 240) / 2
oy = py_panel + 146.0 + 10.0
cx = r.left + w / 2
cy = r.top + oy
print('click at', cx, cy)
u.SetCursorPos(int(cx), int(cy))
time.sleep(0.3)
# click left
u.mouse_event(2, 0, 0, 0, 0); time.sleep(0.05); u.mouse_event(4, 0, 0, 0, 0)
time.sleep(1.5)
print('done')