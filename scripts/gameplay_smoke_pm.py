# -*- coding: utf-8 -*-
"""gameplay_smoke_pm.py - PostMessage 版的游戏内验证驱动（替代 SendInput 版）

为什么要有这个文件
------------------
`gameplay_smoke_win.py` 走 `SendInput`，而 2026-09-12 实测确认：本机上它一个事件都送不到
游戏里（`SendInput` 喂的是**前台窗口**的输入队列，而前台是浏览器；6/6 事件被系统接受、
游戏零响应）。这就是冒烟闸门 `kills=0`、28 次 `cam: yaw=` 全为 0.0 的原因。
`PostMessage` 直接进目标窗口队列，实测能确定性改变游戏状态。

本文件保留原冒烟的结构与断言口径（VUID / kills / fps / panics），只把注入层换掉，
并按 2026-09-12 标定出的四条配方实现视角注入（见 AGENTS.md 铁律 C）。

用法
----
    python scripts/gameplay_smoke_pm.py <log 路径>

四条配方（缺任何一条都会静默失效）
----------------------------------
1. 一律 PostMessage，不用 SendInput。
2. 每一步前重新按下左键（WM_LBUTTONDOWN），与移动背靠背投递——post 的第一次移动会让
   winit 判定指针"进入窗口"并 TrackMouseEvent，真实光标不在窗口上导致立刻 WM_MOUSELEAVE
   -> CursorLeft -> 游戏把 dragging 清掉，后续移动全部到达但视角被跳过。
3. 每次投的坐标必须是"窗口中心 + 增量"，不能累加：拖拽路径每个事件后把 last_cursor
   重设回中心，增量恒为 posted - centre；累加会超过 MAX_LOOK_DELTA_PX=512 被当传送丢弃。
   相邻两次坐标要差 1px，否则被 winit 的位置去重丢掉。
4. 步间隔 > 150ms（用 300ms），否则落在 recenter_pending_until 的 150ms 窗口里被吞掉。
"""
import ctypes
import math
import os
import re
import sys
import time
from ctypes import wintypes

user32 = ctypes.WinDLL("user32", use_last_error=True)
# MUST be called before any size query. Without it Windows virtualises GetClientRect
# for this DPI-unaware process and reports 1706x1066 instead of the real 2560x1600,
# so the "centre + delta" coordinates drift by the 1.5x scale factor and every look
# goes to the wrong place. Measured 2026-09-12.
user32.SetProcessDPIAware()

# ---------- PostMessage 结构 ----------
WM_MOUSEMOVE = 0x0200
WM_LBUTTONDOWN = 0x0201
WM_LBUTTONUP = 0x0202
WM_KEYDOWN = 0x0100
WM_KEYUP = 0x0101
MK_LBUTTON = 0x0001

# Set 1 扫描码
SC = {"w": 0x11, "a": 0x1E, "s": 0x1F, "d": 0x20, "r": 0x13, "space": 0x39,
      "2": 0x03, "3": 0x04, "1": 0x02}
VK = {"w": 0x57, "a": 0x41, "s": 0x53, "d": 0x44, "r": 0x52, "space": 0x20,
      "2": 0x32, "3": 0x33, "1": 0x31}

user32.FindWindowW.restype = wintypes.HWND
user32.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, ctypes.c_size_t, ctypes.c_ssize_t]
user32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
user32.ClientToScreen.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.POINT)]


def find_window(title="Steel Front - Vulkan", tries=60, interval=0.5):
    for _ in range(tries):
        h = user32.FindWindowW(None, title)
        if h:
            return h
        time.sleep(interval)
    return None


def client_size(hwnd):
    r = wintypes.RECT()
    user32.GetClientRect(hwnd, ctypes.byref(r))
    return r.right - r.left, r.bottom - r.top


def pack_xy(x, y):
    return (int(y) & 0xFFFF) << 16 | (int(x) & 0xFFFF)


def post_move(hwnd, x, y):
    user32.PostMessageW(hwnd, WM_MOUSEMOVE, MK_LBUTTON, pack_xy(x, y))


def post_lbutton(hwnd, down, x, y):
    if down:
        user32.PostMessageW(hwnd, WM_LBUTTONDOWN, MK_LBUTTON, pack_xy(x, y))
    else:
        user32.PostMessageW(hwnd, WM_LBUTTONUP, 0, pack_xy(x, y))


def post_key(hwnd, name, down):
    scan = SC[name]
    lp = 1 | (scan << 16)
    if not down:
        lp |= (1 << 30) | (1 << 31)
    user32.PostMessageW(hwnd, WM_KEYDOWN if down else WM_KEYUP, VK[name], lp)


def tap_key(hwnd, name, hold=0.08):
    post_key(hwnd, name, True)
    time.sleep(hold)
    post_key(hwnd, name, False)


# ---------- 日志 ----------
def log_tail(path):
    out = ""
    for p in (path, path + ".err"):
        try:
            with open(p, encoding="utf-8", errors="replace") as f:
                out += f.read()
        except FileNotFoundError:
            pass
    return out


def cam_now(txt):
    m = re.findall(r"cam: yaw=([-\d.]+) pitch=([-\d.]+)", txt)
    return (float(m[-1][0]), float(m[-1][1])) if m else None


def stands_after_run(txt):
    base = txt.find("run started")
    if base < 0:
        return []
    out = []
    for m in re.finditer(r"npc: #(\d+) stand \(([-\d.]+), ([-\d.]+), ([-\d.]+)\)", txt):
        if m.start() > base:
            out.append((int(m.group(1)), float(m.group(2)), float(m.group(3)), float(m.group(4))))
    return out


def game_state(txt):
    """返回 (enemies, score, hp)；取最后一次 game: 行。"""
    m = re.findall(r"game: wave=\d+ enemies=(\d+) .*? hp=(\d+)/\d+ score=(\d+)", txt)
    if not m:
        return None
    e, hp, sc = m[-1]
    return int(e), int(sc), int(hp)


def last_fps(txt):
    m = re.findall(r"fps=([\d.]+)", txt)
    return float(m[-1]) if m else 0.0


def load_sens():
    try:
        with open(os.path.expanduser("~/.steel_front.cfg")) as f:
            for line in f:
                if line.startswith("sensitivity="):
                    return 0.0005 + float(line.strip().split("=", 1)[1]) * 0.002
    except Exception:
        pass
    return 0.0015


SENS = load_sens()
DEG_PX = math.degrees(SENS)   # 每像素多少度


# ---------- 视角注入（四条配方） ----------
def look(hwnd, cx, cy, dpx_x, dpx_y):
    """按配方注入 (dpx_x, dpx_y) 像素的视角位移。返回实际投递的步数。"""
    steps = 0
    rx, ry = int(dpx_x), int(dpx_y)
    while rx != 0 or ry != 0:
        sx = max(-400, min(400, rx))
        sy = max(-400, min(400, ry))
        # 配方 2：每次移动前重新按下左键（CursorLeft 会清掉 dragging）
        post_lbutton(hwnd, True, cx, cy)
        # 配方 3：坐标 = 中心 + 增量；配方 3b：±1px 抖动绕开 winit 位置去重
        j = steps % 2
        post_move(hwnd, cx + sx + j, cy + sy + j)
        # 配方 4：> 150ms 的 recenter 窗口
        time.sleep(0.30)
        rx -= sx
        ry -= sy
        steps += 1
        if steps > 40:
            break
    post_lbutton(hwnd, False, cx, cy)
    time.sleep(0.35)
    return steps


def aim(hwnd, cx, cy, logpath, tgt_yaw, tgt_pitch, rounds=6):
    """闭环瞄准：读日志里的当前 cam yaw/pitch，按标定换算成像素注入，直到误差收敛。"""
    for _ in range(rounds):
        cur = cam_now(log_tail(logpath))
        if not cur:
            time.sleep(0.6)
            continue
        dyaw = ((tgt_yaw - cur[0] + 540.0) % 360.0) - 180.0
        dpitch = tgt_pitch - cur[1]
        # yaw -= dx*sens ⇒ dx = -dyaw/sens;cam 日志是度 ⇒ 先转度到像素
        dpx_x = -dyaw / DEG_PX
        dpx_y = dpitch / DEG_PX
        print("  aim: cur=(%.1f,%.1f) tgt=(%.1f,%.1f) err=(%.1f,%.1f) -> inject %.0f,%.0f px"
              % (cur[0], cur[1], tgt_yaw, tgt_pitch, dyaw, dpitch, dpx_x, dpx_y), flush=True)
        if abs(dyaw) <= 1.5 and abs(dpitch) <= 1.5:
            return True
        look(hwnd, cx, cy, dpx_x, dpx_y)
        time.sleep(0.5)
    return False


def target_angles(npc):
    """NPC world position -> (yaw, pitch) in degrees. The player spawns at the origin,
    so the NPC's world coordinates double as player-relative ones."""
    _, nx, ny, nz = npc
    EYE = 1.6
    rx, ry, rz = nx, ny + 0.8 - EYE, nz
    tgt_yaw = math.degrees(math.atan2(-rx, -rz))
    tgt_pitch = math.degrees(math.atan2(-ry, math.hypot(rx, rz)))
    return tgt_yaw, tgt_pitch


def fire_at(hwnd, cx, cy, logpath, npc, cycles=3, burst=4):
    """Aim, fire a burst, re-aim, repeat.
    The NPC keeps walking between the moment the aim converges and the moment the
    bullets leave, so one fixed burst at a target ~20m out mostly misses even when
    the aim was dead on."""
    for c in range(cycles):
        ty, tp = target_angles(npc)
        if not aim(hwnd, cx, cy, logpath, ty, tp, rounds=5):
            print("  aim did not converge, stopping this target", flush=True)
            return False
        time.sleep(0.15)
        for _ in range(burst):
            post_lbutton(hwnd, True, cx, cy)
            time.sleep(0.08)
            post_lbutton(hwnd, False, cx, cy)
            time.sleep(0.16)
        print("  burst %d/%d done (%d shots)" % (c + 1, cycles, burst), flush=True)
        time.sleep(0.3)
    return True


def wait_for_targets(logpath, timeout=45):
    """NPC positions only get logged at the moment an NPC ENTERS Attack state
    (game.rs:4189 `if state == NpcState::Attack && prev != NpcState::Attack`).
    In wave mode the enemies walk to the player, so this just has to wait."""
    t0 = time.time()
    while time.time() - t0 < timeout:
        v = stands_after_run(log_tail(logpath))
        if v:
            return v
        time.sleep(2.0)
    return []


def main():
    logpath = sys.argv[1] if len(sys.argv) > 1 else "smoke_pm.log"
    hwnd = find_window()
    if not hwnd:
        print("NO-WINDOW, aborting", flush=True)
        return 2
    w, h = client_size(hwnd)
    cx, cy = w // 2, h // 2
    print("hwnd=%#x client=%dx%d centre=(%d,%d) sens=%.6f rad/px (%.4f deg/px)"
          % (hwnd, w, h, cx, cy, SENS, DEG_PX), flush=True)
    print("inject: PostMessage only (no foreground, no cursor grab, no pointer lock)", flush=True)

    txt = log_tail(logpath)
    st = game_state(txt)
    print("initial enemies/score/hp = %s" % (st,), flush=True)

    # Keyboard probe: switching weapons is logged by the game, so it is a
    # machine-readable witness that the keyboard channel works.
    before = len(re.findall(r"weapons: ", txt))
    tap_key(hwnd, "2")
    time.sleep(0.8)
    after = len(re.findall(r"weapons: ", log_tail(logpath)))
    print("keyboard probe: weapons log %d -> %d  %s"
          % (before, after, "OK" if after > before else "FAIL"), flush=True)

    # Deliberately NO movement here. target_angles() treats the npc's WORLD position as
    # player-relative, which is only true while the player stands at the spawn point.
    # Walking first (an earlier revision did) silently offsets every aim by however far
    # the player moved -- 38 shots fired, aim reported converged, zero hits.
    time.sleep(0.3)

    print("waiting up to 60s for npcs to enter Attack state ...", flush=True)
    victims = wait_for_targets(logpath, timeout=60)
    kills_before = game_state(log_tail(logpath))
    print("targets (%d): %s" % (len(victims), victims[:4]), flush=True)

    shots_before = len(re.findall(r"shot #", log_tail(logpath)))
    tried = set()
    t_end = time.time() + 150
    while time.time() < t_end:
        cur = game_state(log_tail(logpath))
        if cur and kills_before and cur[1] > kills_before[1]:
            print("    KILL REGISTERED (score %d -> %d)" % (kills_before[1], cur[1]), flush=True)
            break
        # Re-read each round: more npcs enter Attack state as the wave closes in.
        live = [v for v in stands_after_run(log_tail(logpath)) if v[0] not in tried]
        live.sort(key=lambda t: t[1] ** 2 + t[3] ** 2)
        if not live:
            time.sleep(2.0)
            continue
        npc = live[0]
        tried.add(npc[0])
        print("--- aim npc#%d @ (%.1f,%.1f,%.1f)  [hp=%s]" % (npc[0], npc[1], npc[2], npc[3], cur[2] if cur else "?"), flush=True)
        fire_at(hwnd, cx, cy, logpath, npc, cycles=3, burst=4)
        print("    -> %s" % (game_state(log_tail(logpath)),), flush=True)

    txt = log_tail(logpath)
    final = game_state(txt)
    vuid = len(re.findall(r"VUID", txt))
    panics = len(re.findall(r"panic", txt, re.I))
    fps = last_fps(txt)
    killed = 0 if not (final and kills_before) else final[1] - kills_before[1]
    shots = len(re.findall(r"shot #", txt)) - shots_before
    print("", flush=True)
    print("VUID=%d panics=%d fps=%.1f shots_fired=%d score %s -> %s (score delta %d; 10 pts per kill)"
          % (vuid, panics, fps, shots, kills_before[1] if kills_before else "?", final[1] if final else "?", killed), flush=True)
    ok = vuid == 0 and panics == 0 and killed >= 1
    print("RESULT: %s" % ("ALL-OK" if ok else "FAIL"), flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
