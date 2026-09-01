import ctypes
import sys

KEYUP = 2
vk = int(sys.argv[1], 0)
u = ctypes.windll.user32
u.keybd_event(vk, 0, 0, 0)
u.keybd_event(vk, 0, KEYUP, 0)
print("sent vk=%s" % hex(vk))
