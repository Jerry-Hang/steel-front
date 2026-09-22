# -*- coding: utf-8 -*-
"""survive_pm.py - drive a full `survive` match (5 waves) with PostMessage injection.

Why this file exists
--------------------
Unresolved item #17: "`survive` 完整 5 波真机未验".  The rule itself is unit-tested
(`survive_rule_advances_waves_and_wins_at_last`), but nobody had ever driven the real
map (`assets/maps/defense_line.toml`, `[rule] kind = "survive" waves = 5`) from wave 1
to the victory state on the actual GPU build.  This driver does exactly that: it plays
the match with the same injection stack as the smoke test and reports the wave
progression the *engine* logged, not what a screenshot looked like.

Injection layer
---------------
Reuses `gameplay_smoke_pm.py` verbatim (PostMessage only, the four calibrated recipes,
the closed-loop aim against `npc: #id stand`).  No SendInput, no foreground change, no
cursor grab.  Player never moves (no W/A/S/D), which is what keeps the "npc world
position == player-relative position" assumption in `target_angles` true.

Caveat stated up front: this run uses RV3D_INVINCIBLE=1 so the match cannot end in
`survive: 玩家阵亡于第 N 波 → 失败` before wave 5 -- the defeat path is covered by unit
tests, the point here is the wave/intermission/victory chain.

Usage
-----
    python scripts/survive_pm.py <log path> [--secs 900] [--shot-every 60]
"""
import argparse
import ctypes
import ctypes.wintypes as wintypes
import os
import re
import sys
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gameplay_smoke_pm as S  # noqa: E402  (the calibrated injection stack)

from PIL import Image  # noqa: E402  (dev-script only; the Rust crate has no new deps)

user32 = S.user32
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
user32.GetWindowDC.restype = ctypes.c_void_p
user32.GetWindowDC.argtypes = [ctypes.c_void_p]
user32.GetWindowRect.argtypes = [ctypes.c_void_p, ctypes.POINTER(wintypes.RECT)]
user32.PrintWindow.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint]
gdi32.CreateCompatibleDC.restype = ctypes.c_void_p
gdi32.CreateCompatibleDC.argtypes = [ctypes.c_void_p]
gdi32.CreateCompatibleBitmap.restype = ctypes.c_void_p
gdi32.CreateCompatibleBitmap.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
gdi32.SelectObject.restype = ctypes.c_void_p
gdi32.SelectObject.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
gdi32.DeleteObject.argtypes = [ctypes.c_void_p]
gdi32.DeleteDC.argtypes = [ctypes.c_void_p]
user32.ReleaseDC.argtypes = [ctypes.c_void_p, ctypes.c_void_p]


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [("biSize", ctypes.c_uint32), ("biWidth", ctypes.c_int32),
                ("biHeight", ctypes.c_int32), ("biPlanes", ctypes.c_uint16),
                ("biBitCount", ctypes.c_uint16), ("biCompression", ctypes.c_uint32),
                ("biSizeImage", ctypes.c_uint32), ("biXPelsPerMeter", ctypes.c_int32),
                ("biYPelsPerMeter", ctypes.c_int32), ("biClrUsed", ctypes.c_uint32),
                ("biClrImportant", ctypes.c_uint32)]


gdi32.GetDIBits.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint,
                            ctypes.c_void_p, ctypes.POINTER(BITMAPINFOHEADER), ctypes.c_uint]


def screenshot(hwnd, path):
    """PrintWindow(hwnd, dc, 2) -- PW_RENDERFULLCONTENT, the only flag that gets live
    Vulkan content out of an unfocused window (see AGENTS.md #1)."""
    rect = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    w, h = rect.right - rect.left, rect.bottom - rect.top
    if w < 16 or h < 16:
        return False
    hdc = user32.GetWindowDC(hwnd)
    mem = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, w, h)
    old = gdi32.SelectObject(mem, bmp)
    user32.PrintWindow(hwnd, mem, 2)
    bmi = BITMAPINFOHEADER()
    bmi.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    bmi.biWidth = w
    bmi.biHeight = -h          # top-down
    bmi.biPlanes = 1
    bmi.biBitCount = 32
    buf = ctypes.create_string_buffer(w * h * 4)
    gdi32.GetDIBits(mem, bmp, 0, h, ctypes.cast(buf, ctypes.c_void_p),
                    ctypes.byref(bmi), 0)
    img = Image.frombuffer("RGBA", (w, h), buf, "raw", "BGRA", 0, 0).convert("RGB")
    img.save(path)
    gdi32.SelectObject(mem, old)
    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(hwnd, hdc)
    return True


# ---------- 日志解析 ----------
def dead_ids(txt):
    return set(int(m.group(1)) for m in re.finditer(r"kill: npc #(\d+) eliminated", txt))


def stands(txt):
    """id -> (x, y, z) of the LAST `npc: #id stand` line.

    NPCs stand still in the Attack state (game.rs: 攻击态原地站定), which is what makes
    a stand line usable as an aim point later on. A later line for the same id means it
    left and re-entered Attack, so the last one is the freshest.
    """
    out = {}
    for m in re.finditer(r"npc: #(\d+) stand \(([-\d.]+), ([-\d.]+), ([-\d.]+)\)", txt):
        out[int(m.group(1))] = (float(m.group(2)), float(m.group(3)), float(m.group(4)))
    return out


def wave_now(txt):
    m = re.findall(r"game: wave=(\d+) enemies=(\d+)", txt)
    return (int(m[-1][0]), int(m[-1][1])) if m else (0, -1)


def score_now(txt):
    m = re.findall(r"game: wave=\d+ enemies=\d+ .*? score=(\d+)", txt)
    return int(m[-1]) if m else -1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("logpath")
    ap.add_argument("--secs", type=float, default=900.0, help="driving budget")
    ap.add_argument("--shot-every", type=float, default=60.0)
    ap.add_argument("--max-engage", type=int, default=6,
                    help="give up on one stand line after this many aim engagements "
                         "(smoke's 'stopping this target' equivalent; the 2026-09-22 "
                         "run dead-looped to try=85 without it)")
    ap.add_argument("--shotdir", default=None)
    args = ap.parse_args()

    logpath = args.logpath
    shotdir = args.shotdir or os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                           "..", "screenshots")
    os.makedirs(shotdir, exist_ok=True)
    tag = os.path.splitext(os.path.basename(logpath))[0]

    hwnd = S.find_window()
    if not hwnd:
        print("NO-WINDOW, aborting", flush=True)
        return 2
    w, h = S.client_size(hwnd)
    cx, cy = w // 2, h // 2
    print("hwnd=%#x client=%dx%d centre=(%d,%d) sens=%.6f rad/px"
          % (hwnd, w, h, cx, cy, S.SENS), flush=True)

    # Enter Playing. `on_any_key()` is what leaves the menu; RV3D_AUTOSTART does NOT
    # skip the menu (AGENTS.md 铁律 C). The smoke test's Digit2 probe does this too.
    txt = S.log_tail(logpath)
    if "run started" not in txt:
        print("menu: tapping Digit2 to leave the menu (on_any_key)", flush=True)
        S.tap_key(hwnd, "2")
        for _ in range(30):
            time.sleep(1.0)
            if "run started" in S.log_tail(logpath):
                break
    txt = S.log_tail(logpath)
    print("state: run started = %s, wave/enemies = %s"
          % ("run started" in txt, wave_now(txt)), flush=True)

    # The first tap only opens the menu path; make sure the weapon is a rifle.
    time.sleep(1.0)

    t0 = time.time()
    deadline = t0 + args.secs
    last_shot_at = t0
    attempts = {}
    stand_pos = {}          # npc_id -> the stand line the attempts are counted against
    waves_seen = []
    last_wave = -1
    engaged = 0
    result = "TIMEOUT"

    while time.time() < deadline:
        txt = S.log_tail(logpath)

        # --- end conditions, straight out of the engine's own log -------------
        if re.search(r"survive: 全部 \d+ 波守住", txt):
            result = "VICTORY"
            break
        if "survive: 玩家阵亡" in txt:
            result = "DEFEAT"
            break
        if re.search(r"panicked at", txt):
            result = "PANIC"
            break

        wave, enemies = wave_now(txt)
        if wave != last_wave:
            last_wave = wave
            waves_seen.append(wave)
            taken = os.path.join(shotdir, "%s_wave%d.png" % (tag, wave))
            screenshot(hwnd, taken)
            print("[%6.0fs] WAVE %d  enemies=%d  -> %s"
                  % (time.time() - t0, wave, enemies, os.path.basename(taken)), flush=True)

        dead = dead_ids(txt)
        live = {i: p for i, p in stands(txt).items() if i not in dead}
        if not live:
            time.sleep(2.0)
            continue

        # Nearest first, then least-attempted: an NPC already shot at twice without
        # dying is usually one whose stand line is stale or which is behind cover.
        order = sorted(live.items(), key=lambda kv: (attempts.get(kv[0], 0),
                                                     kv[1][0] ** 2 + kv[1][2] ** 2))
        npc_id, pos = order[0]
        # The cap is per stand LINE, not per id: a fresh line (the NPC left and
        # re-entered Attack at a different spot) is a new target worth a full
        # budget again. The 2026-09-22 run dead-looped on one frozen line to
        # try=85 because nothing stopped engaging it (smoke has "stopping this
        # target"; survive's outer loop re-picks every iteration).
        if stand_pos.get(npc_id) != pos:
            stand_pos[npc_id] = pos
            attempts[npc_id] = 0
        attempts[npc_id] = attempts.get(npc_id, 0) + 1
        if attempts[npc_id] > args.max_engage:
            # == max+1 prints exactly once per line; the corpse sentinel (99)
            # skips the message and just never re-engages.
            if attempts[npc_id] == args.max_engage + 1:
                print("    giving up on npc#%d after %d tries (stale stand line or "
                      "behind cover), stopping this target"
                      % (npc_id, args.max_engage), flush=True)
            time.sleep(2.0)
            continue
        engaged += 1
        sc0 = score_now(txt)
        print("[%6.0fs] wave %d/%d enemies=%d  aim npc#%d @(%.1f,%.1f,%.1f) try=%d"
              % (time.time() - t0, wave, 5, enemies, npc_id, pos[0], pos[1], pos[2],
                 attempts[npc_id]), flush=True)
        ty, tp = S.target_angles((npc_id, pos[0], pos[1], pos[2]))
        if not S.aim(hwnd, cx, cy, logpath, ty, tp, rounds=4):
            print("    aim did not converge", flush=True)
            continue
        for _ in range(4):
            S.post_lbutton(hwnd, True, cx, cy)
            time.sleep(0.08)
            S.post_lbutton(hwnd, False, cx, cy)
            time.sleep(0.16)
        time.sleep(0.4)
        sc1 = score_now(S.log_tail(logpath))
        if sc1 > sc0 >= 0:
            print("    KILL (score %d -> %d), enemies=%d"
                  % (sc0, sc1, wave_now(S.log_tail(logpath))[1]), flush=True)
            attempts[npc_id] = 99  # never re-engage a corpse

        if time.time() - last_shot_at >= args.shot_every:
            last_shot_at = time.time()
            shot = os.path.join(shotdir, "%s_t%.0f.png" % (tag, time.time() - t0))
            screenshot(hwnd, shot)
            print("    shot -> %s" % os.path.basename(shot), flush=True)

    txt = S.log_tail(logpath)
    vuid = len(re.findall(r"VUID", txt))
    panics = len(re.findall(r"panic", txt, re.I))
    lost = len(re.findall(r"has been lost", txt))
    kills = len(re.findall(r"kill: npc #\d+ eliminated", txt))
    shots = len(re.findall(r"shot #", txt))
    cleared = re.findall(r"wave: wave (\d+) cleared", txt)
    supply = re.findall(r"survive: 波间补给（血量 ([\d.]+)%", txt)
    spawned = re.findall(r"wave: wave (\d+) spawned (\d+) enemies", txt)
    victory = re.findall(r"survive: 全部 (\d+) 波守住", txt)

    print("", flush=True)
    print("=== survive run summary ===", flush=True)
    print("  result        : %s (%.0fs of a %.0fs budget)" % (result, time.time() - t0, args.secs))
    print("  waves logged  : %s" % (waves_seen,))
    print("  spawns        : %s" % (spawned,))
    print("  waves cleared : %s" % (cleared,))
    print("  supply windows: %s (hp%%)" % (supply,))
    print("  victory line  : %s" % (victory,))
    print("  kills/shots   : %d / %d   engagements=%d" % (kills, shots, engaged))
    print("  VUID=%d panics=%d device_lost=%d fps=%.1f"
          % (vuid, panics, lost, S.last_fps(txt)), flush=True)
    ok = (result == "VICTORY" and vuid == 0 and panics == 0 and lost == 0
          and len(cleared) == len(spawned) and len(supply) == len(cleared) - 1)
    print("RESULT: %s" % ("ALL-OK" if ok else "CHECK"), flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
