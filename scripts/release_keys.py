import ctypes, time
x11 = ctypes.CDLL("libX11.so.6"); xtst = ctypes.CDLL("libXtst.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
d = x11.XOpenDisplay(None)
if not d: raise SystemExit("no display")
# 释放可能卡住的键/按钮（XTEST 状态是 server 级的）
for kc in (65, 37, 38, 39, 40, 113, 114, 9, 66, 25):  # Space A S D W Q E Tab Shift B
    xtst.XTestFakeKeyEvent(d, kc, 0, 0); x11.XFlush(d)
for btn in (1, 2, 3, 4, 5):
    xtst.XTestFakeButtonEvent(d, btn, 0, 0); x11.XFlush(d)
time.sleep(0.1)
print("keys/buttons released", flush=True)
