#!/usr/bin/env python3
"""look_bot.py — 固定视角驱动：yaw=0/pitch=-10°，不射击不移动，死亡按 R 重开。
用于分辨率对照基准（避免后坐力把视角压到 -89° 触发低头剔除 bug 污染数据）。"""
import ctypes, time, sys, re
LOG = sys.argv[1] if len(sys.argv) > 1 else "/tmp/perf.log"
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
xtst.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeRelativeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
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
def cam_now():
    try:
        txt = open(LOG).read()
    except FileNotFoundError:
        return None
    m = re.search(r"cam: yaw=([-\d.]+) pitch=([-\d.]+)", txt)
    return (float(m.group(1)), float(m.group(2))) if m else None
def game_over():
    try:
        return re.search(r"hp=0/", open(LOG).read()) is not None
    except FileNotFoundError:
        return False
def look(yaw_target=-0.0, pitch_target=-10.0):
    """拖拽回正到目标视角（LMB 按住 + 相对移动，复用冒烟机制）"""
    c = cam_now()
    if not c: return
    deg_px = math.degrees(0.0015)
    dyaw = ((yaw_target - c[0] + 540.0) % 360.0) - 180.0
    dpitch = c[1] - pitch_target
    dx = int(dyaw / deg_px); dy = int(dpitch / deg_px)
    steps = 5
    dx = max(-400, min(400, dx)) // max(1, steps)
    dy = max(-400, min(400, dy)) // max(1, steps)
    xtst.XTestFakeButtonEvent(d, 1, 1, 0); flush()
    for _ in range(steps):
        xtst.XTestFakeRelativeMotionEvent(d, dx, dy, 0); flush(); time.sleep(0.06)
    xtst.XTestFakeButtonEvent(d, 1, 0, 0); flush()
    time.sleep(0.3)
import math
win = None; t0 = time.time()
while not win and time.time() - t0 < 20:
    time.sleep(0.5); win = find_window(root, "Steel Front")
if not win: print("NO-WINDOW"); sys.exit(1)
k_space = x11.XKeysymToKeycode(d, x11.XStringToKeysym(b"space"))
k_r = x11.XKeysymToKeycode(d, x11.XStringToKeysym(b"r"))
key(k_space, True); key(k_space, False); time.sleep(0.5)
look()
end = time.time() + float(sys.argv[2]) if len(sys.argv) > 2 else time.time() + 45
last_look = 0.0
while time.time() < end:
    if game_over():
        key(k_r, True); key(k_r, False); time.sleep(1.0); look(); continue
    if time.time() - last_look > 5.0:
        look(); last_look = time.time()
    time.sleep(0.5)
print("LOOK-BOT-DONE")
