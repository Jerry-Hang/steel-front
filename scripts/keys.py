# -*- coding: utf-8 -*-
import ctypes, time, sys
from ctypes import wintypes
user32 = ctypes.windll.user32
INPUT_MOUSE = 0
INPUT_KEYBOARD = 1
KEYEVENTF_KEYUP = 0x0002
MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_RIGHTDOWN = 0x0008
MOUSEEVENTF_RIGHTUP = 0x0010

class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", ctypes.c_long), ("dy", ctypes.c_long), ("mouseData", ctypes.c_ulong),
                ("dwFlags", ctypes.c_ulong), ("time", ctypes.c_ulong), ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong))]
class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", ctypes.c_ushort), ("wScan", ctypes.c_ushort), ("dwFlags", ctypes.c_ulong),
                ("time", ctypes.c_ulong), ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong))]
class _U(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT)]
class INPUT(ctypes.Structure):
    _fields_ = [("type", ctypes.c_ulong), ("u", _U)]

def key(vk, up=False):
    inp = INPUT()
    inp.type = INPUT_KEYBOARD
    inp.u.ki = KEYBDINPUT(vk, 0, KEYEVENTF_KEYUP if up else 0, 0, None)
    user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))

def mousemove(dx, dy, flags, mdata=0):
    inp = INPUT()
    inp.type = INPUT_MOUSE
    inp.u.mi = MOUSEINPUT(dx, dy, mdata, flags, 0, None)
    user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))

def tap(vk, delay=0.08):
    key(vk); time.sleep(delay); key(vk, True); time.sleep(0.05)

def hold(vk, secs):
    key(vk); time.sleep(secs); key(vk, True); time.sleep(0.05)

act = sys.argv[1]
if act == 'tab':
    tap(0x09)
elif act.startswith('w'):
    hold(0x57, float(act[1:]))
elif act.startswith('e'):
    hold(0x45, float(act[1:]))
elif act.startswith('q'):
    hold(0x51, float(act[1:]))
elif act.startswith('space'):
    hold(0x20, float(act[5:]))
elif act.startswith('enter'):
    tap(0x0D)
elif act.startswith('r'):
    tap(0x52)
elif act.startswith('rdrag'):
    parts = act[5:].split(',')
    dx, dy = int(parts[0]), int(parts[1])
    user32.SetCursorPos(1280, 800)
    time.sleep(0.15)
    mousemove(0, 0, MOUSEEVENTF_RIGHTDOWN)
    time.sleep(0.1)
    n = max(1, abs(dx) // 40)
    sx = 40 if dx > 0 else -40
    sy = 40 if dy > 0 else -40
    for i in range(n):
        mousemove(sx, 0, MOUSEEVENTF_MOVE)
        time.sleep(0.03)
    for i in range(max(1, abs(dy) // 40)):
        mousemove(0, sy, MOUSEEVENTF_MOVE)
        time.sleep(0.03)
    mousemove(0, 0, MOUSEEVENTF_RIGHTUP)
elif act == 'click':
    user32.SetCursorPos(1280, 800)
    time.sleep(0.1)
    mousemove(0, 0, 0x0002)
    time.sleep(0.05)
    mousemove(0, 0, 0x0004)
print('done', act)
