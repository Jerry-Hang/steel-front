#!/usr/bin/env python3
"""key_bot.py — 极简驱动：Space 开局，死亡按 R 重开；不碰鼠标（相机由 RV3D_BENCH_* 固定）。"""
import ctypes, time, sys, re
LOG = sys.argv[1] if len(sys.argv) > 1 else "/tmp/perf.log"
SECS = float(sys.argv[2]) if len(sys.argv) > 2 else 45.0
x11 = ctypes.CDLL("libX11.so.6"); xtst = ctypes.CDLL("libXtst.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
d = x11.XOpenDisplay(None)
if not d: sys.exit("no display")
x11.XDefaultRootWindow.restype = ctypes.c_ulong
root = x11.XDefaultRootWindow(d)
x11.XFetchName.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_char_p)]
x11.XQueryTree.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)), ctypes.POINTER(ctypes.c_uint)]
x11.XStringToKeysym.argtypes = [ctypes.c_char_p]; x11.XStringToKeysym.restype = ctypes.c_ulong
x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]; x11.XKeysymToKeycode.restype = ctypes.c_uint
xtst.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
def flush(): x11.XFlush(d); time.sleep(0.02)
def key(kc, down): xtst.XTestFakeKeyEvent(d, kc, 1 if down else 0, 0); flush()
def find_window(win, name):
    nm = ctypes.c_char_p()
    if x11.XFetchName(d, win, ctypes.byref(nm)) and nm.value:
        if name in nm.value.decode(errors="ignore"): return win
    rr=ctypes.c_ulong(); pp=ctypes.c_ulong(); ch=ctypes.POINTER(ctypes.c_ulong)(); n=ctypes.c_uint()
    if x11.XQueryTree(d, win, ctypes.byref(rr), ctypes.byref(pp), ctypes.byref(ch), ctypes.byref(n)):
        for i in range(n.value):
            r = find_window(ch[i], name)
            if r: return r
    return None
def game_over():
    try:
        return re.search(r"hp=0/", open(LOG).read()) is not None
    except FileNotFoundError:
        return False
win = None; t0 = time.time()
while not win and time.time() - t0 < 20:
    time.sleep(0.5); win = find_window(root, "Steel Front")
if not win: print("NO-WINDOW"); sys.exit(1)
k_space = x11.XKeysymToKeycode(d, x11.XStringToKeysym(b"space"))
k_r = x11.XKeysymToKeycode(d, x11.XStringToKeysym(b"r"))
key(k_space, True); key(k_space, False)
end = time.time() + SECS
while time.time() < end:
    if game_over():
        key(k_r, True); key(k_r, False); time.sleep(1.2)
    time.sleep(0.5)
print("KEY-BOT-DONE")
