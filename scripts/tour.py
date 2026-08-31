# 极简键鼠注入器（巡检用）：python scripts/tour.py tab "key:0x51:5" "mm:600:0" "sleep:1" "r"
import sys, time, ctypes
from ctypes import wintypes

user32 = ctypes.windll.user32
INPUT_MOUSE, INPUT_KEYBOARD = 0, 1
KEYEVENTF_KEYUP = 0x0002
MOUSEEVENTF_MOVE = 0x0001

class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", wintypes.LONG), ("dy", wintypes.LONG), ("mouseData", wintypes.DWORD),
                ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.POINTER(wintypes.ULONG))]
class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wintypes.WORD), ("wScan", wintypes.WORD), ("dwFlags", wintypes.DWORD),
                ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.POINTER(wintypes.ULONG))]
class INPUT_UNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT)]
class INPUT(ctypes.Structure):
    _fields_ = [("type", wintypes.DWORD), ("u", INPUT_UNION)]

def send(inp):
    user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(inp))

def key(vk, times=1):
    for _ in range(times):
        i = INPUT(type=INPUT_KEYBOARD, u=INPUT_UNION(ki=KEYBDINPUT(wVk=vk, wScan=0, dwFlags=0, time=0, dwExtraInfo=None)))
        send(i); time.sleep(0.06)
        i.u.ki.dwFlags = KEYEVENTF_KEYUP
        send(i); time.sleep(0.06)

def mm(dx, dy, chunk=300):
    x, y = dx, dy
    while x or y:
        cx = max(-chunk, min(chunk, x)); cy = max(-chunk, min(chunk, y))
        i = INPUT(type=INPUT_MOUSE, u=INPUT_UNION(mi=MOUSEINPUT(dx=cx, dy=cy, mouseData=0, dwFlags=MOUSEEVENTF_MOVE, time=0, dwExtraInfo=None)))
        send(i); x -= cx; y -= cy; time.sleep(0.03)

VK = {"tab": 0x09, "r": 0x52, "enter": 0x0D, "q": 0x51, "e": 0x45, "w": 0x57, "a": 0x41, "s": 0x53, "d": 0x44, "space": 0x20}

for arg in sys.argv[1:]:
    p = arg.split(":")
    cmd = p[0].lower()
    if cmd == "sleep":
        time.sleep(float(p[1]))
    elif cmd == "mm":
        mm(int(p[1]), int(p[2]))
    elif cmd == "key":
        key(int(p[1], 0), int(p[2]) if len(p) > 2 else 1)
    elif cmd in VK:
        key(VK[cmd], int(p[1]) if len(p) > 1 else 1)
    else:
        print("unknown:", arg)
