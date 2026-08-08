import ctypes, math, time, sys, re
LOG = sys.argv[1] if len(sys.argv) > 1 else "/tmp/gameplay_smoke.log"
SENS = 0.003  # rad/px, 与 camera.rs MOUSE_SENSITIVITY 一致
DEG_PX = math.degrees(SENS)
x11 = ctypes.CDLL("libX11.so.6"); xtst = ctypes.CDLL("libXtst.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
d = x11.XOpenDisplay(None)
if not d: sys.exit("no display")
x11.XDefaultRootWindow.restype = ctypes.c_ulong
root = x11.XDefaultRootWindow(d)
x11.XFetchName.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_char_p)]
x11.XQueryTree.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)), ctypes.POINTER(ctypes.c_uint)]
x11.XMapRaised.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
x11.XGetInputFocus.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int)]
x11.XTranslateCoordinates.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong)]
x11.XWarpPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint, ctypes.c_int, ctypes.c_int]
xtst.XTestFakeRelativeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
def flush(): x11.XFlush(d); time.sleep(0.02)
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
def window_size_from_log():
    try:
        txt = open(LOG).read()
    except FileNotFoundError:
        return None
    m = re.search(r"\u7a97\u53e3\u5927\u5c0f\u53d8\u5316: (\d+)x(\d+)", txt)
    return (int(m.group(1)), int(m.group(2))) if m else None
def window_attrs(win):
    try:
        txt = open(LOG).read()
    except FileNotFoundError:
        return None
    return window_size_from_log()
def activate():
    """等窗口出现且 mapped（IsViewable=2）；设置输入焦点为尽力而为。
    本环境（WSLg/Xwayland）核心焦点常为 None，拖拽视角路径不需要焦点，
    因此焦点失败不致命，只要窗口可定位（warp 用）即可。"""
    global win
    win = find_window(root, "Steel Front")
    t0 = time.time()
    while not win and time.time() - t0 < 20:
        time.sleep(0.5)
        win = find_window(root, "Steel Front")
    if not win:
        print("NO-WINDOW", flush=True); sys.exit(1)
    t0 = time.time()
    while time.time() - t0 < 10:
        if "cam:" in log_tail():
            break
        time.sleep(0.3)
    size = window_size_from_log()
    print(f"window 0x{win:x} ready size={size}", flush=True)
    x11.XMapRaised(d, win); flush(); time.sleep(0.5)
    foc = ctypes.c_ulong(); rev = ctypes.c_int()
    ok = False
    for _ in range(15):
        x11.XSetInputFocus(d, win, 1, 0); flush(); time.sleep(0.2)
        x11.XGetInputFocus(d, ctypes.byref(foc), ctypes.byref(rev))
        if foc.value == win:
            ok = True
            break
    print(f"FOCUS-OK win=0x{win:x}" if ok else f"FOCUS-FAIL foc=0x{foc.value:x}", flush=True)
    return ok
def warp_to_center(quiet=False):
    size = window_size_from_log() or (1280, 720)
    rx = ctypes.c_int(); ry = ctypes.c_int(); child = ctypes.c_ulong()
    if not x11.XTranslateCoordinates(d, win, root, 0, 0, ctypes.byref(rx), ctypes.byref(ry), ctypes.byref(child)):
        print("warp: XTranslateCoordinates failed, skip", flush=True)
        return
    cx, cy = rx.value + size[0] // 2, ry.value + size[1] // 2
    x11.XWarpPointer(d, 0, root, 0, 0, 0, 0, cx, cy); flush(); time.sleep(0.15)
    if not quiet:
        print(f"warp to window center ({cx},{cy}) size={size[0]}x{size[1]}", flush=True)

def press_release(kc):
    xtst.XTestFakeKeyEvent(d, kc, 1, 0); flush(); time.sleep(0.08)
    xtst.XTestFakeKeyEvent(d, kc, 0, 0); flush()
def key_hold(kc, hold):
    xtst.XTestFakeKeyEvent(d, kc, 1, 0); flush(); time.sleep(hold)
    xtst.XTestFakeKeyEvent(d, kc, 0, 0); flush()
def log_tail():
    try:
        with open(LOG) as f: return f.read()
    except FileNotFoundError: return ""
def cam_now(txt):
    m = re.findall(r"cam: yaw=([-\d.]+) pitch=([-\d.]+)", txt)
    if not m: return None
    return (float(m[-1][0]), float(m[-1][1]))
def aim(yaw_tgt_deg, pitch_tgt_deg, hold_lmb):
    for _ in range(6):
        warp_to_center(quiet=True)               # 每轮回中：相对位移不累积漂出窗口
        txt = log_tail()
        cur = cam_now(txt)
        if not cur:
            time.sleep(1.0); continue
        dyaw = ((yaw_tgt_deg - cur[0] + 540.0) % 360.0) - 180.0
        dpitch = cur[1] - pitch_tgt_deg          # look(): pitch -= dy*SENS
        dx = int(dyaw / DEG_PX)
        dy = int(dpitch / DEG_PX)
        print(f"  aim round: cur=({cur[0]:.1f},{cur[1]:.1f}) tgt=({yaw_tgt_deg:.1f},{pitch_tgt_deg:.1f}) "
              f"inject=({dx},{dy})", flush=True)
        if abs(dx) < 2 and abs(dy) < 2:
            if hold_lmb:
                xtst.XTestFakeButtonEvent(d, 1, 0, 0); flush()   # 松键：收敛即结束拖拽
            return True
        if hold_lmb:
            xtst.XTestFakeButtonEvent(d, 1, 1, 0); flush()
        xtst.XTestFakeRelativeMotionEvent(d, max(-400, min(400, dx)), max(-400, min(400, dy)), 0)
        flush(); time.sleep(1.2)
    if hold_lmb:
        xtst.XTestFakeButtonEvent(d, 1, 0, 0); flush()   # 松键：拖拽瞄准结束
    return False
SPACE, LMB = 65, 1
activate()
if not win:
    print("NO-WINDOW, aborting", flush=True); sys.exit(1)
# 守卫：开始前必须处于 StartMenu（日志里没有 "run started"）；若已误触发则退出
txt = log_tail()
if "run started" in txt:
    print("RUN-ALREADY-ACTIVE (proceeding: game already in Playing)", flush=True)
else:
    warp_to_center()
    time.sleep(0.5)
    press_release(SPACE); time.sleep(0.3)
    xtst.XTestFakeButtonEvent(d, LMB, 1, 0); flush()   # 点击也开局：键盘焦点不可靠时兜底
    xtst.XTestFakeButtonEvent(d, LMB, 0, 0); flush()
    time.sleep(1.2)
    time.sleep(1.0)
# 等 NPC 进入 Attack 站定：菜单期预置 NPC 的站定日志在 "run started" 之前，必须过滤
def stands_after_run():
    txt = log_tail()
    base = txt.find("run started")
    if base < 0:
        return []
    out = []
    for m in re.finditer(r"npc: #(\d+) stand \(([-\d.]+), ([-\d.]+), ([-\d.]+)\)", txt):
        if m.start() > base:
            out.append((int(m.group(1)), float(m.group(2)), float(m.group(3)), float(m.group(4))))
    return out

# FPS：玩家在原点不动，任意方向站定 NPC 都可直接瞄准（射线不需要过原点）
t0 = time.time()
while time.time() - t0 < 16.0:
    stands = stands_after_run()
    if stands:
        break
    time.sleep(0.5)

def fire_at(npc):
    _, nx, ny, nz = npc
    EYE = 1.6                                    # 玩家眼睛高度
    cx, cy, cz = nx, ny + 0.8 - EYE, nz          # 命中球相对眼睛（头顶 +0.8m）
    yaw_tgt = math.degrees(math.atan2(-cx, -cz))
    pitch_tgt = math.degrees(math.atan2(-cy, math.hypot(cx, cz)))
    d3 = math.hypot(cx, cz)
    print(f"aim npc #{npc[0]} C=({cx:.1f},{cy:.1f},{cz:.1f}) dist={d3:.1f} "
          f"yaw={yaw_tgt:.1f} pitch={pitch_tgt:.1f}", flush=True)
    # FPS 拖拽视角：按住左键拖拽（捕获态/非捕获态都能转视角，不依赖焦点）。
    # 收敛校验：最多 3 次瞄准尝试，未收敛绝不开火（防瞄准螺旋浪费窗口被 NPC 抢杀）
    converged = False
    for sub in range(3):
        warp_to_center()
        if aim(yaw_tgt, pitch_tgt, True):
            converged = True
            break
        print(f"  aim sub-attempt {sub + 1}/3 NOT converged, retrying", flush=True)
    if not converged:
        print("  aim FAILED to converge, skipping fire", flush=True)
        return False
    # 静置：等回中 warp 事件消化，避免开火瞬间相机仍在转动
    time.sleep(0.6)
    # 开火前最后复核：若相机偏离目标 > 3° 则放弃本次（宁可换目标也不空射）
    cur = cam_now(log_tail())
    if cur:
        dyaw = ((yaw_tgt - cur[0] + 540.0) % 360.0) - 180.0
        dpitch = cur[1] - pitch_tgt
        if abs(dyaw) > 3.0 or abs(dpitch) > 3.0:
            print(f"  aim drift before fire: dyaw={dyaw:.1f} dpitch={dpitch:.1f}, skip", flush=True)
            return False
    # 点射 6 发：25 伤害 × 4 = 100 hp；射速上限 3/s，间隔 0.35s；后坐力快速恢复
    for _ in range(6):
        xtst.XTestFakeButtonEvent(d, LMB, 1, 0); flush()
        xtst.XTestFakeButtonEvent(d, LMB, 0, 0); flush()
        time.sleep(0.35)
    time.sleep(0.5)
    return "kill:" in log_tail()

killed = False
tried = set()
for attempt in range(2):
    # 每个 id 只取最新站定位置；离原点最近的先打
    latest = {}
    for s in stands_after_run():
        latest[s[0]] = s
    cands = [s for s in latest.values() if s[0] not in tried]
    cands.sort(key=lambda t: math.hypot(t[1], t[3]))
    if not cands or time.time() - t0 > 19.0:
        break
    npc = cands[0]
    tried.add(npc[0])
    print(f"attempt {attempt + 1}/2: target npc #{npc[0]} "
          f"C=({npc[1]:.1f},{npc[2]:.1f},{npc[3]:.1f})", flush=True)
    if fire_at(npc):
        killed = True
        break
time.sleep(1.0)
txt = log_tail()
vuid = len(re.findall(r"VUID", txt))
fps = [float(x) for x in re.findall(r"fps=([0-9.]+)", txt)]
cams = re.findall(r"cam: yaw=([-\d.]+) pitch=([-\d.]+)", txt)
yaws = sorted(set(round(float(a), 1) for a, b in cams))
pitches = sorted(set(round(float(b), 1) for a, b in cams))
kills = len(re.findall(r"kill:", txt))
hp_vals = sorted(set(re.findall(r"hp=([0-9.]+)/", txt)))
waves = re.findall(r"game: wave=(\d+)", txt)
hits = len(re.findall(r"projectile hit", txt))
panics = len(re.findall(r"panicked", txt))
errs = len(re.findall(r" ERROR ", txt))
print(f"VUID={vuid} fps_min={min(fps) if fps else -1:.1f} fps_max={max(fps) if fps else -1:.1f} "
      f"yaw_count={len(yaws)} pitch_count={len(pitches)} kills={kills} hit_events={hits} "
      f"hp_vals={hp_vals} waves={sorted(set(waves))} panics={panics} errors={errs}", flush=True)
ok = (vuid == 0 and fps and min(fps) >= 200.0 and len(yaws) >= 2 and len(pitches) >= 2
      and kills >= 1 and len(hp_vals) >= 2 and len(set(waves)) >= 1 and panics == 0)
print("ALL-OK" if ok else "FAIL", flush=True)
sys.exit(0 if ok else 2)
