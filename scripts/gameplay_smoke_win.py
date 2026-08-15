#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Steel Front Windows 原生冒烟测试（X11 版 gameplay_smoke.py 的 Win32 移植）
- 窗口查找/激活：FindWindowW / SetForegroundWindow（替代 XQueryTree/XSetInputFocus）
- 输入注入：SendInput（替代 XTestFakeKeyEvent / XTestFakeRelativeMotionEvent / XTestFakeButtonEvent）
- 日志断言与原版一致：VUID=0、kills>=1、fps>=120、yaw/pitch 变化、hp、wave、无 panic
用法: python scripts/gameplay_smoke_win.py <log_path>
"""
import ctypes
import math
import os
import re
import sys
import time
from ctypes import wintypes

LOG = sys.argv[1] if len(sys.argv) > 1 else "smoke.log"

user32 = ctypes.WinDLL("user32", use_last_error=True)

# ---------- SendInput 结构 ----------
INPUT_MOUSE = 0
INPUT_KEYBOARD = 1
MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_SCANCODE = 0x0008


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", wintypes.LONG), ("dy", wintypes.LONG), ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(wintypes.ULONG)),
    ]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wintypes.WORD), ("wScan", wintypes.WORD), ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.POINTER(wintypes.ULONG)),
    ]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [("uMsg", wintypes.DWORD), ("wParamL", wintypes.WORD), ("wParamH", wintypes.WORD)]


class INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", wintypes.DWORD), ("union", INPUTUNION)]


def send_mouse(dx, dy, flags):
    inp = INPUT()
    inp.type = INPUT_MOUSE
    inp.union.mi.dx = int(dx)
    inp.union.mi.dy = int(dy)
    inp.union.mi.mouseData = 0
    inp.union.mi.dwFlags = flags
    inp.union.mi.time = 0
    user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))
    time.sleep(0.02)


def send_key(scancode, down):
    inp = INPUT()
    inp.type = INPUT_KEYBOARD
    inp.union.ki.wVk = 0
    inp.union.ki.wScan = scancode
    inp.union.ki.dwFlags = KEYEVENTF_SCANCODE | (KEYEVENTF_KEYUP if not down else 0)
    inp.union.ki.time = 0
    user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))
    time.sleep(0.02)


# 扫描码（Set 1）：Space=0x39, W=0x11, A=0x1E, S=0x1F, D=0x20, R=0x13
SC_SPACE, SC_W, SC_A, SC_S, SC_D, SC_R = 0x39, 0x11, 0x1E, 0x1F, 0x20, 0x13

# ---------- 窗口 ----------
user32.FindWindowW.restype = wintypes.HWND
user32.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
user32.SetForegroundWindow.argtypes = [wintypes.HWND]
user32.ShowWindow.argtypes = [wintypes.HWND, ctypes.c_int]
user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]


def find_window(name, tries=60, interval=0.5):
    for _ in range(tries):
        hwnd = user32.FindWindowW(None, name)
        if hwnd:
            return hwnd
        time.sleep(interval)
    return None


def activate(hwnd):
    user32.ShowWindow(hwnd, 9)  # SW_RESTORE
    user32.SetForegroundWindow(hwnd)
    time.sleep(0.3)


def window_center(hwnd):
    r = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    return ((r.left + r.right) // 2, (r.top + r.bottom) // 2)


def warp_to_center(hwnd):
    cx, cy = window_center(hwnd)
    user32.SetCursorPos(cx, cy)
    time.sleep(0.15)


# ---------- 灵敏度（与 X11 版一致：~/.steel_front.cfg） ----------
def load_sens():
    try:
        with open(os.path.expanduser("~/.steel_front.cfg")) as f:
            for line in f:
                if line.startswith("sensitivity="):
                    s = float(line.strip().split("=", 1)[1])
                    return 0.0005 + s * 0.002
    except Exception:
        pass
    return 0.0015  # 默认 0.5 档


SENS = load_sens()
DEG_PX = math.degrees(SENS)


# ---------- 日志 ----------
def log_tail():
    out = ""
    for p in (LOG, LOG + ".err"):
        try:
            with open(p, encoding="utf-8", errors="replace") as f:
                out += f.read()
        except FileNotFoundError:
            pass
    return out


def cam_now(txt):
    m = re.findall(r"cam: yaw=([-\d.]+) pitch=([-\d.]+)", txt)
    if not m:
        return None
    return (float(m[-1][0]), float(m[-1][1]))


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


def press_release(sc):
    send_key(sc, True)
    time.sleep(0.08)
    send_key(sc, False)


def lmb_click():
    send_mouse(0, 0, MOUSEEVENTF_LEFTDOWN)
    time.sleep(0.05)
    send_mouse(0, 0, MOUSEEVENTF_LEFTUP)


# ---------- 瞄准注入（分块 <=400px，与 X11 版同策略） ----------
def aim(hwnd, yaw_tgt_deg, pitch_tgt_deg, hold_lmb):
    for _ in range(5):
        txt = log_tail()
        cur = cam_now(txt)
        if not cur:
            time.sleep(1.0)
            continue
        dyaw = ((yaw_tgt_deg - cur[0] + 540.0) % 360.0) - 180.0
        dpitch = pitch_tgt_deg - cur[1]
        # 2026-08-15 游戏鼠标 X 方向修正（右移=右转，yaw -= dx*sens）：
        # 目标角度差 dyaw 需取反注入（旧方向 dx = dyaw/DEG_PX 会振荡不收敛）
        dx = -int(dyaw / DEG_PX)
        dy = int(dpitch / DEG_PX)
        print(f"  aim round: cur=({cur[0]:.1f},{cur[1]:.1f}) tgt=({yaw_tgt_deg:.1f},{pitch_tgt_deg:.1f}) inject=({dx},{dy})", flush=True)
        if abs(dx) <= 8 and abs(dy) <= 8:
            return True
        if hold_lmb:
            send_mouse(0, 0, MOUSEEVENTF_LEFTDOWN)
        rx, ry = dx, dy
        while rx != 0 or ry != 0:
            cx = max(-400, min(400, rx))
            cy = max(-400, min(400, ry))
            send_mouse(cx, cy, MOUSEEVENTF_MOVE)
            time.sleep(0.03)
            rx -= cx
            ry -= cy
        time.sleep(1.0)
    if hold_lmb:
        send_mouse(0, 0, MOUSEEVENTF_LEFTUP)
    return False


def fire_at(hwnd, npc):
    _, nx, ny, nz = npc
    EYE = 1.6
    cx, cy, cz = nx, ny + 0.8 - EYE, nz
    yaw_tgt = math.degrees(math.atan2(-cx, -cz))
    pitch_tgt = math.degrees(math.atan2(-cy, math.hypot(cx, cz)))
    converged = False
    for sub in range(3):
        if aim(hwnd, yaw_tgt, pitch_tgt, hold_lmb=True):
            converged = True
            break
        print(f"  aim sub-attempt {sub + 1}/3 NOT converged, retrying", flush=True)
    if not converged:
        print("  aim FAILED to converge, skipping fire", flush=True)
        return False
    time.sleep(0.6)
    cur = cam_now(log_tail())
    if cur:
        dyaw = ((yaw_tgt - cur[0] + 540.0) % 360.0) - 180.0
        dpitch = pitch_tgt - cur[1]
        if abs(dyaw) > 3.0 or abs(dpitch) > 3.0:
            print(f"  aim drift before fire: dyaw={dyaw:.1f} dpitch={dpitch:.1f}, skip", flush=True)
            return False
    for _ in range(6):
        lmb_click()
        time.sleep(0.35)
    time.sleep(0.5)
    return "kill:" in log_tail()


# ---------- 主流程 ----------
def main():
    hwnd = find_window("Steel Front - Vulkan")
    if not hwnd:
        print("NO-WINDOW, aborting", flush=True)
        sys.exit(1)
    activate(hwnd)
    txt = log_tail()
    if "run started" in txt:
        print("RUN-ALREADY-ACTIVE (proceeding: game already in Playing)", flush=True)
    else:
        warp_to_center(hwnd)
        time.sleep(0.5)
        press_release(SC_SPACE)
        time.sleep(0.3)
        lmb_click()
        time.sleep(2.2)

    t0 = time.time()
    while time.time() - t0 < 16.0:
        if stands_after_run():
            break
        time.sleep(0.5)

    killed = False
    tried = set()
    for attempt in range(2):
        latest = {}
        for s in stands_after_run():
            latest[s[0]] = s
        cands = [s for s in latest.values() if s[0] not in tried]
        cands.sort(key=lambda t: math.hypot(t[1], t[3]))
        if not cands or time.time() - t0 > 19.0:
            break
        npc = cands[0]
        tried.add(npc[0])
        print(f"attempt {attempt + 1}/2: target npc #{npc[0]} C=({npc[1]:.1f},{npc[2]:.1f},{npc[3]:.1f})", flush=True)
        if fire_at(hwnd, npc):
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
    ok = (vuid == 0 and fps and min(fps) >= 120.0 and len(yaws) >= 2 and len(pitches) >= 2
          and kills >= 1 and len(hp_vals) >= 2 and len(set(waves)) >= 1 and panics == 0)
    print("ALL-OK" if ok else "FAIL", flush=True)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
