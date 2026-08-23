# -*- coding: utf-8 -*-
import ctypes, time, sys
user32 = ctypes.windll.user32
INPUT_MOUSE = 0
MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
MOUSEEVENTF_WHEEL = 0x0800
class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", ctypes.c_long), ("dy", ctypes.c_long), ("mouseData", ctypes.c_ulong),
                ("dwFlags", ctypes.c_ulong), ("time", ctypes.c_ulong), ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong))]
class INPUT(ctypes.Structure):
    _fields_ = [("type", ctypes.c_ulong), ("mi", MOUSEINPUT)]
def send(dx, dy, flags, mdata=0):
    inp = INPUT()
    inp.type = INPUT_MOUSE
    inp.mi = MOUSEINPUT(dx, dy, mdata, flags, 0, None)
    user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))
n = int(sys.argv[1]) if len(sys.argv) > 1 else 0
w = int(sys.argv[2]) if len(sys.argv) > 2 else 0
user32.SetCursorPos(1280, 800)
time.sleep(0.2)
if n > 0:
    send(0, 0, MOUSEEVENTF_LEFTDOWN)
    time.sleep(0.1)
    for i in range(n):
        send(40, 0, MOUSEEVENTF_MOVE)
        time.sleep(0.03)
    send(0, 0, MOUSEEVENTF_LEFTUP)
for i in range(w):
    send(0, 0, MOUSEEVENTF_WHEEL, 120)
    time.sleep(0.05)
print("dragged", n, "wheel", w)
