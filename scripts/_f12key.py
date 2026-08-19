
import ctypes, time, sys
from ctypes import wintypes
u = ctypes.WinDLL('user32')
INPUT_KEYBOARD = 1
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_SCANCODE = 0x0008
class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wintypes.WORD), ("wScan", wintypes.WORD), ("dwFlags", wintypes.DWORD),
                ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.POINTER(wintypes.ULONG))]
class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", wintypes.LONG), ("dy", wintypes.LONG), ("mouseData", wintypes.DWORD),
                ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.POINTER(wintypes.ULONG))]
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
    u.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT)); time.sleep(0.05)
def press(sc):
    send_key(sc, True); time.sleep(0.1); send_key(sc, False)
u.FindWindowW.restype = wintypes.HWND
h = u.FindWindowW(None, 'Steel Front - Vulkan')
if not h: print('NO WINDOW'); sys.exit(1)
u.SetForegroundWindow(h); time.sleep(0.5)
SC_F12 = 0x58
press(SC_F12)
print('F12 pressed')
