import ctypes
from ctypes import wintypes
u = ctypes.WinDLL('user32')
u.FindWindowW.restype = wintypes.HWND
h = u.FindWindowW(None, 'Steel Front - Vulkan')
print('FINDWINDOW:', h)
buf = ctypes.create_unicode_buffer(256)
if h:
    u.GetWindowTextW(h, buf, 256)
    print('TITLE:', buf.value)