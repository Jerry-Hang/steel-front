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
cursor grab.  Since 2026-09-22 the player DOES move: distant targets get an
approach walk (W along the sight line), a capped stand line gets up to
`--max-repos` perpendicular A/D strafes before give-up, and bursts are
reload-aware (the engine auto-reloads on an empty mag but loses that shot, so
missing `shot #` ticks mean "wait out the 2.3s window, fire the rest").  Aim
angles are computed player-relative via `target_angles_rel` + the `pos=` field
on the engine's 1s `game:` status line -- the smoke-side origin assumption of
`target_angles` no longer applies here (smoke itself still never moves).

Caveat stated up front: this run uses RV3D_INVINCIBLE=1 so the match cannot end in
`survive: 玩家阵亡于第 N 波 → 失败` before wave 5 -- the defeat path is covered by unit
tests, the point here is the wave/intermission/victory chain.

Usage
-----
    python scripts/survive_pm.py <log path> [--secs 900] [--shot-every 60]
                                   [--max-engage 6] [--max-repos 2]
"""
import argparse
import ctypes
import ctypes.wintypes as wintypes
import math
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


def live_pos(txt):
    """id -> (x, y, z) of the LAST `npcpos: #id x y z State [vis=0|1]` line.

    🔴 2026-09-25 加：`stands()` 给的只是**进入 Attack 那一刻**的快照，移动靶/反复进出
    Attack 的残局目标全程被瞄在旧位置上打空（实测 12 发/杀、残局 8 分钟零命中）。
    `npcpos` 每秒一只一行 ⇒ 这才是"当前在哪"。没开这个开关时返回空表，调用方回退到 stands()。
    """
    out = {}
    for m in re.finditer(
            r"npcpos: #(\d+) ([-\d.]+) ([-\d.]+) ([-\d.]+) ", txt):
        out[int(m.group(1))] = (float(m.group(2)), float(m.group(3)), float(m.group(4)))
    return out


def live_visible(txt):
    """id -> True/False：引擎给的「玩家眼位看不看得见」判据（`npcpos: … vis=0|1`）。

    🔴 2026-09-25 加：`RV3D_PROJ_DIAG` 的两秒行量出 **33% 的子弹打在掩体上**（§21.18）
    —— 盲选目标 = 三分之一的弹药送给墙。`vis=1` 才是"这一枪有射线"。
    没有这个字段（旧引擎/未开 `RV3D_NPC_POS`）时返回空表 ⇒ 调用方按"全都可见"处理。
    """
    out = {}
    for m in re.finditer(r"npcpos: #(\d+) [-\d.]+ [-\d.]+ [-\d.]+ \S+ vis=(\d)", txt):
        out[int(m.group(1))] = (m.group(2) == "1")
    return out


def targets(txt):
    """活靶优先、stand 行兜底：id -> (x, y, z)。"""
    live = live_pos(txt)
    fallback = stands(txt)
    for k, v in fallback.items():
        live.setdefault(k, v)
    return live


def dry_weapons(txt):
    """引擎报「备弹耗尽」的武器名集合（`weapons: <名> 备弹耗尽（弹匣 0 / 备弹 0）`）。

    引擎侧 2026-09-25 新加的一次性告警（commit d1391d1）：弹药打空后 `try_fire` 恒返回
    None 且此前一条日志都不打 ⇒ harness 空点 8 分钟。有了这行，harness 才能知道该换枪。

    🔴 武器名**含空格**（实机那行是 `weapons: AK-12M 风暴 备弹耗尽（…）`）⇒ 这里只能用
    `(.+?)` 惰性吃到标记词。旧版 `(\\S+) 备弹耗尽` 在真机上**恒不匹配**（2026-09-25 实测）。
    """
    return set(m.group(1) for m in
               re.finditer(r"weapons:\s+(.+?)\s+备弹耗尽", txt))


def dry_switch(txt, slot, handled, max_slot=9):
    """→ `(新槽位, 本次新识别的武器名集合)`；不需要换枪时返回 `None`。

    两条判据都是从实机 bug 反推的（2026-09-25 那次 run 的 10:19:44 告警之后 **100 秒 0 发**）：

    1. **换枪与「有没有活靶」无关** —— 弹药是全局的：引擎说这一把打不出来了，再瞄多久
       也是 0 发。旧版把换枪藏在 `if not live:` 分支里 ⇒ **有活靶时永远不换枪**。
    2. **只认没处理过的武器名** —— `log_tail` 是滑动窗口，换枪之后旧的「耗尽」行仍在
       窗口里；若按「窗口里有没有耗尽行」判，harness 会把 9 个槽一路空切到底。
    """
    fresh = dry_weapons(txt) - set(handled)
    if not fresh or slot >= max_slot:
        return None
    return slot + 1, fresh


def self_test():
    """`--self-test`：把「从实机 bug 反推出来的判据」钉死（不进游戏、不注入输入）。

    夹具取自 `logs/survive_pm.log.err` 里那两行**原文**。有真日志时顺手回放一次
    （只打印统计，不设断言 —— 免得以后修好了反而红）。
    """
    dry_line = ("[2026-09-25T10:19:44Z WARN  steel_front::engine::weapons] "
                "weapons: AK-12M 风暴 备弹耗尽（弹匣 0 / 备弹 0）"
                "—— 本局再也打不出一发，只能等波间补给或换武器")
    shot_line = ("[2026-09-25T10:18:49Z INFO  steel_front::engine::game] "
                 "weapons: shot #1 (1 alive) [AK-12M 风暴]")
    fails = []

    def chk(name, cond):
        print("  %-56s %s" % (name, "ok" if cond else "FAIL"))
        if not cond:
            fails.append(name)

    chk("dry line -> full weapon name (name has a space!)",
        dry_weapons(dry_line) == {"AK-12M 风暴"})
    chk("ordinary shot line is not a dry warning", dry_weapons(shot_line) == set())
    chk("empty text yields nothing", dry_weapons("") == set())
    chk("dry -> switch to the next slot",
        dry_switch(dry_line, 0, set()) == (1, {"AK-12M 风暴"}))
    chk("handled weapon does not switch again",
        dry_switch(dry_line, 1, {"AK-12M 风暴"}) is None)
    chk("last slot does not switch", dry_switch(dry_line, 9, set()) is None)
    chk("shot line never switches", dry_switch(shot_line, 0, set()) is None)
    chk("stall watchdog: no enemies -> never fires",
        not stall_due(1000.0, 0.0, 0, 25.0))
    chk("stall watchdog: recent progress -> no stall",
        not stall_due(1000.0, 990.0, 3, 25.0))
    chk("stall watchdog: stale + enemies alive -> stall",
        stall_due(1000.0, 900.0, 3, 25.0))
    # 🔴 2026-09-25：33% 的子弹打在掩体上（`RV3D_PROJ_DIAG` 量出来的，见 PROGRESS §21.18）
    # ⇒ 目标选择必须能用引擎给的遮挡判据。夹具是引擎 `npcpos:` 行的原文格式。
    vis_line = ("[2026-09-25T23:10:00Z INFO  steel_front::engine::game] "
                "npcpos: #7 12.34 0.00 -5.67 Attack vis=1\n"
                "[2026-09-25T23:10:00Z INFO  steel_front::engine::game] "
                "npcpos: #8 -3.00 0.00 9.00 Chase vis=0")
    chk("npcpos vis field: 1 = visible, 0 = blocked",
        live_visible(vis_line) == {7: True, 8: False})
    chk("npcpos without vis (old engine) -> empty map (all visible)",
        live_visible("npcpos: #7 12.34 0.00 -5.67 Attack") == {})
    chk("position parse is unaffected by the trailing vis field",
        live_pos(vis_line)[7] == (12.34, 0.0, -5.67))
    # 选择顺序：可见的排前面（哪怕它更远）
    mk = lambda i, vis_: (i, (100.0 if i == 8 else 5.0, 0.0, 0.0))
    order = sorted([mk(7, True), mk(8, False)],
                   key=lambda kv: (0 if live_visible(vis_line).get(kv[0], True) else 1,
                                   0, 0, kv[1][0] ** 2))
    chk("target order prefers the visible one", [i for i, _ in order] == [7, 8])

    logp = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                        "..", "logs", "survive_pm.log.err")
    if os.path.exists(logp):
        with open(logp, "r", encoding="utf-8", errors="replace") as fh:
            real = fh.read()
        if "备弹耗尽" in real:
            tail = real.split("备弹耗尽", 1)[1]
            print("  real log replay: dry names=%s, `shot #` after the dry line=%d"
                  % (sorted(dry_weapons(real)), tail.count("shot #")))
        else:
            print("  real log replay: no dry warning in %s" % os.path.basename(logp))
    else:
        print("  real log replay: %s not found" % logp)

    print("SELF-TEST: %s (%d checks, %d failed)"
          % ("OK" if not fails else "FAIL", 14 + 1, len(fails)))
    return 0 if not fails else 1


def wave_now(txt):
    m = re.findall(r"game: wave=(\d+) enemies=(\d+)", txt)
    return (int(m[-1][0]), int(m[-1][1])) if m else (0, -1)


def score_now(txt):
    m = re.findall(r"game: wave=\d+ enemies=\d+ .*? score=(\d+)", txt)
    return int(m[-1]) if m else -1


def player_pos(txt):
    """(x, z) from the LAST `game:` status line's pos= field (added 2026-09-22
    together with the repositioning below; the 1s cadence is fine because the
    player only ever moves inside reposition())."""
    m = re.findall(r"game: wave=\d+ .*? pos=\(([-\d.]+),([-\d.]+)\)", txt)
    return (float(m[-1][0]), float(m[-1][1])) if m else None


def shots_count(txt):
    """Cumulative player shots from the engine's own `weapons: shot #N` counter."""
    m = re.findall(r"weapons: shot #(\d+)", txt)
    return int(m[-1]) if m else 0


def move_hold(hwnd, logpath, key, hold):
    """Hold a movement key for `hold` seconds and report the ACTUAL displacement
    measured from the engine's pos= field (blocked movement shows as ~0m, which
    is exactly what the caller needs to know). Waits for a status line that
    postdates the key release (the line is 1s-cadenced, so poll up to ~2.4s)."""
    p0 = player_pos(S.log_tail(logpath))
    S.post_key(hwnd, key, True)
    time.sleep(hold)
    S.post_key(hwnd, key, False)
    p1 = p0
    for _ in range(6):
        time.sleep(0.4)
        cand = player_pos(S.log_tail(logpath))
        if cand is not None:
            p1 = cand
            if p0 and (abs(cand[0] - p0[0]) > 0.2 or abs(cand[1] - p0[1]) > 0.2):
                break
    if p0 and p1:
        return math.hypot(p1[0] - p0[0], p1[1] - p0[1])
    return -1.0


def target_angles_rel(npc, ppos):
    """S.target_angles minus the player's actual position. The smoke version
    hardcodes the origin because smoke never moves; survive strafes now, so
    the subtraction is mandatory -- yaw is atan2 of the PLAYER->NPC vector.

    🔴 2026-09-25 瞄点由 +0.8m 抬到 +1.25m（**胸腔**）。引擎的部位倍率
    （`Game::part_multiplier`：头 1.5 / 胸 1.0 / 臂 0.8 / 腿 0.6）按**离地高度**分区，
    0.8m 落在腿/臂区 ⇒ 每发只有 0.6–0.8 倍伤害。真机实测（600s，wave 2 剩 3 只）：
    瞄点收敛到 err≈0.0 却打不动，`kills/shots 11/150`、最后 250 秒 100 发只杀 1 只；
    按 120HP / 0.6 倍算正好每次要 ~11 发。抬到 1.25m = 胸区 1.0 倍，同样的命中率下
    击杀时间缩短约 1.6 倍（且胸腔比头大得多，不追求爆头）。
    """
    _, nx, ny, nz = npc
    EYE = 1.6
    rx, rz = nx - ppos[0], nz - ppos[1]
    ry = ny + 1.25 - EYE
    return (math.degrees(math.atan2(-rx, -rz)),
            math.degrees(math.atan2(-ry, math.hypot(rx, rz))))


def reposition(hwnd, logpath, side):
    """One ~3m perpendicular strafe (PLAYER_SPEED=6 m/s x 0.5s) via move_hold."""
    return move_hold(hwnd, logpath, side, 0.5)


def hits_now(txt):
    """引擎状态行的累计命中数（打墙/打友军不计）。没有该字段时返回 -1。"""
    m = re.findall(r"hits=(\d+)", txt)
    return int(m[-1]) if m else -1


def stall_due(now, last_progress_at, enemies, stall_secs):
    """卡死看门狗判据（纯函数，`--self-test` 钉住它）。

    🔴 2026-09-25 加：600s 那次 run 里「预算打满就 `sleep 2.0` 无限循环」让 harness 在
    **只剩一只躲在掩体后的 NPC** 时 8 分钟一发未发（整场只 202 发），而引擎那边一切正常。
    判据 = 还有敌人活着、但「上一次有进展」（开火/走位/换位）已经过去 `stall_secs` 秒。
    """
    return enemies > 0 and (now - last_progress_at) > stall_secs


def main():
    if "--self-test" in sys.argv:
        return self_test()
    ap = argparse.ArgumentParser()
    ap.add_argument("logpath")
    ap.add_argument("--no-shot", action="store_true",
                    help="skip every PrintWindow capture. On the discrete GPU the "
                         "combination of Playing + capture reproducibly loses the Vulkan "
                         "device (2026-09-25 matrix), while the same scene without capture "
                         "runs fine at ~100fps; the integrated GPU is unaffected.")
    ap.add_argument("--secs", type=float, default=900.0, help="driving budget")
    ap.add_argument("--shot-every", type=float, default=60.0)
    ap.add_argument("--max-engage", type=int, default=6,
                    help="give up on one stand line after this many aim engagements "
                         "(smoke's 'stopping this target' equivalent; the 2026-09-22 "
                         "run dead-looped to try=85 without it)")
    ap.add_argument("--max-repos", type=int, default=2,
                    help="strafe repositions to try on a capped stand line before "
                         "giving up (alternating d/a; a converged-aim-no-kill means "
                         "the NPC is behind cover, and a perpendicular 3m strafe is "
                         "the cheapest way to break that alignment)")
    ap.add_argument("--stall-secs", type=float, default=25.0,
                    help="stall watchdog: enemies alive but nothing fired/moved for "
                         "this long -> clear every target budget and reposition "
                         "(the 2026-09-25 600s run dead-looped 8 minutes this way)")
    ap.add_argument("--approach-gt", type=float, default=35.0,
                    help="walk toward a target farther than this (m): the 1.5deg "
                         "aim tolerance is ~1.6m of drift at 60m, wider than an "
                         "NPC hitbox -- distant targets are unkillable, period")
    ap.add_argument("--approach-stop", type=float, default=20.0,
                    help="close to roughly this range before firing")
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
    repos_left = {}         # npc_id -> repositioning budget left for the CURRENT line
    slot = [0]              # 当前武器槽（0 起）；备弹耗尽时递增换枪（见主循环）
    dry_handled = set()     # 已经因「备弹耗尽」换掉过的武器名（滑动窗口会一直看得见旧行）
    last_progress_at = t0   # 上一次"有进展"（开火 / 走位 / 换位）的时间；卡死看门狗用它
    stalls = 0              # 看门狗触发次数（打印用，也让左右侧移交替）
    blocked = {}            # npc_id -> "打出去但 hits 不动"的连续次数（换位判据）
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

        # 换枪优先于一切交战决策：弹药是全局的（判据见 dry_switch 的注释）。
        sw = dry_switch(txt, slot[0], dry_handled)
        if sw:
            slot[0], fresh = sw
            dry_handled |= fresh
            S.tap_key(hwnd, str(slot[0] + 1), 0.12)
            print("    dry (%s) -> switch to slot %d"
                  % (",".join(sorted(fresh)), slot[0] + 1), flush=True)
            time.sleep(1.2)
            continue

        wave, enemies = wave_now(txt)
        if wave != last_wave:
            last_wave = wave
            waves_seen.append(wave)
            taken = os.path.join(shotdir, "%s_wave%d.png" % (tag, wave))
            if not args.no_shot:
                screenshot(hwnd, taken)
            print("[%6.0fs] WAVE %d  enemies=%d  -> %s"
                  % (time.time() - t0, wave, enemies, os.path.basename(taken)), flush=True)

        # 卡死看门狗：有敌人活着但「上一次有进展」已经过了 --stall-secs 秒 ⇒
        # 清零全部目标的尝试预算 + 换个射击位置重来（旧版在这里永久放弃 ⇒ 8 分钟一发未发）。
        if stall_due(time.time(), last_progress_at, enemies, args.stall_secs):
            stalls += 1
            attempts.clear()
            stand_pos.clear()
            repos_left.clear()
            side = "d" if stalls % 2 else "a"
            moved = move_hold(hwnd, logpath, side, 1.0)
            print("    STALL %.0fs with %d enemies alive, nothing fired -> budgets cleared, "
                  "reposition %s %.1fm"
                  % (time.time() - last_progress_at, enemies, side, moved), flush=True)
            last_progress_at = time.time()
            continue

        dead = dead_ids(txt)
        # 瞄点用**活靶位置**（`npcpos:`，引擎 `RV3D_NPC_POS=1` 每秒一只一行），stand 行兜底；
        # 但「尝试预算」的键仍用 **stand 行**：活靶位置每秒都在变，拿它当键会把预算无限重置
        # （`max_engage` 形同虚设 —— 那正是 2026-09-22 try=85 死循环的成因）。
        lines = {i: p for i, p in stands(txt).items() if i not in dead}
        live = {i: p for i, p in targets(txt).items() if i not in dead}
        # 遮挡判据（引擎侧 `npc_occluded`）：33% 的子弹原本送给掩体（§21.18）。
        # 表为空（旧引擎 / 没开 RV3D_NPC_POS）时按"全都可见"处理，行为与旧版一致。
        vis = {i: v for i, v in live_visible(txt).items() if i not in dead}
        if not live:
            time.sleep(2.0)
            continue

        # Nearest first, then least-attempted: an NPC already shot at twice without
        # dying is usually one whose stand line is stale or which is behind cover.
        # ⚠️ 弹药是稀缺资源（满弹 120 发 / 一局）⇒ **交火中（有 stand 行 = 有视线）的目标优先**，
        # 只在"没人交火"时才退而求其次打"只有活靶位置"的那批。
        # 🔴 2026-09-25：可见性排在最前 —— 打看不见的目标 = 打墙（实测 33% 的子弹）。
        order = sorted(live.items(), key=lambda kv: (0 if vis.get(kv[0], True) else 1,
                                                     0 if kv[0] in lines else 1,
                                                     attempts.get(kv[0], 0),
                                                     kv[1][0] ** 2 + kv[1][2] ** 2))
        npc_id, pos = order[0]
        budget_key = lines.get(npc_id, pos)
        # The cap is per stand LINE, not per id: a fresh line (the NPC left and
        # re-entered Attack at a different spot) is a new target worth a full
        # budget again. The 2026-09-22 run dead-looped on one frozen line to
        # try=85 because nothing stopped engaging it (smoke has "stopping this
        # target"; survive's outer loop re-picks every iteration).
        if stand_pos.get(npc_id) != budget_key:
            stand_pos[npc_id] = budget_key
            attempts[npc_id] = 0
            repos_left.pop(npc_id, None)   # fresh line = fresh repositioning budget too
        attempts[npc_id] = attempts.get(npc_id, 0) + 1
        if attempts[npc_id] > args.max_engage:
            # Reposition BEFORE surrendering: both observed failure shapes break
            # under a perpendicular strafe -- a converged-aim-no-kill means the
            # firing line is occluded and 3m sideways walks it off the cover; a
            # frozen stale line gets a fresh geometry to fail against. The corpse
            # sentinel (>=90) must not burn repositions, hence the <90 guard.
            left = repos_left.get(npc_id, args.max_repos)
            if left > 0 and attempts[npc_id] < 90:
                repos_left[npc_id] = left - 1
                side = "d" if (args.max_repos - left) % 2 == 0 else "a"
                moved = reposition(hwnd, logpath, side)
                print("    reposition %s: moved %.1fm, re-arming npc#%d (%d left)"
                      % (side, moved, npc_id, left - 1), flush=True)
                if moved > 0.5:
                    last_progress_at = time.time()
                attempts[npc_id] = 0
                continue
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
        ppos = player_pos(txt) or (0.0, 0.0)
        npc = (npc_id, pos[0], pos[1], pos[2])
        ty, tp = target_angles_rel(npc, ppos)
        # Approach: W walks along the camera forward, so aim first, close the
        # gap on the line of sight, then re-aim from the new position. The
        # wave-3 cluster sat 50-75m out and ate 150 engagements with zero
        # kills -- not cover, just geometry: 1.5deg tolerance > NPC hitbox.
        dist = math.hypot(pos[0] - ppos[0], pos[2] - ppos[1])
        if dist > args.approach_gt:
            if S.aim(hwnd, cx, cy, logpath, ty, tp, rounds=4):
                moved = move_hold(hwnd, logpath, "w",
                                  max(min((dist - args.approach_stop) / 6.0, 5.0), 0.5))
                print("    approach w: moved %.1fm toward npc#%d (was %.0fm)"
                      % (moved, npc_id, dist), flush=True)
                if moved > 0.5:
                    last_progress_at = time.time()
                txt = S.log_tail(logpath)
                ppos = player_pos(txt) or ppos
                ty, tp = target_angles_rel(npc, ppos)
        # 🔴 2026-09-25：看不见就别开枪 —— 埋点量出 **33% 的子弹打在掩体上**（§21.18）。
        # 对着被挡住的目标扣扳机是纯浪费（弹药一局就那么多）；改成"转向它 + 走过去拿视线"，
        # 把子弹留给打得着的目标。visibility 表为空（旧引擎）时按可见处理 = 旧行为。
        if not vis.get(npc_id, True):
            if S.aim(hwnd, cx, cy, logpath, ty, tp, rounds=4):
                moved = move_hold(hwnd, logpath, "w", 1.0)
                print("    no line of sight to npc#%d -> walked %.1fm to gain it"
                      % (npc_id, moved), flush=True)
                if moved > 0.5:
                    last_progress_at = time.time()
            else:
                print("    no line of sight and aim did not converge", flush=True)
            continue
        if not S.aim(hwnd, cx, cy, logpath, ty, tp, rounds=4):
            print("    aim did not converge", flush=True)
            continue
        # 🔴 2026-09-25：到此为止瞄的是**上一次读日志时**的位置。样本过期 + NPC 走动
        # （4–5 m/s）= 提前量错误，这正是 hits 只有 20–33%、12–15 发/杀的来源。
        # 现在两件事一起做：① 引擎侧 `RV3D_NPC_POS_HZ=10`（样本年龄 ≤100ms）；
        # ② 扣扳机前**再读一次**尾日志，目标动过 ≥0.5m 就重算角度再收敛一次。
        txt = S.log_tail(logpath)
        fresh = targets(txt).get(npc_id)
        if fresh:
            moved_m = math.hypot(fresh[0] - pos[0], fresh[2] - pos[2])
            if moved_m > 0.5:
                print("    re-aim: npc#%d moved %.1fm since the sample" % (npc_id, moved_m),
                      flush=True)
                pos = fresh
                npc = (npc_id, pos[0], pos[1], pos[2])
                ty, tp = target_angles_rel(npc, player_pos(txt) or ppos)
                if not S.aim(hwnd, cx, cy, logpath, ty, tp, rounds=4):
                    print("    re-aim did not converge", flush=True)
                    continue
        # Reload-aware burst: try_fire on an EMPTY magazine auto-arms the
        # reload but loses that shot (weapons.rs try_fire + the
        # firearm_empty_magazine_auto_reloads_and_cannot_fire test), and every
        # click inside the 2.3s window is silent -- the 2026-09-22 run wasted
        # ~60% of its trigger pulls that way (282 shots from 700 clicks).
        # Count what the engine actually fired and make up the rest after the
        # window instead of eating the dry clicks.
        rounds = 4
        s0 = shots_count(txt)
        h0 = hits_now(txt)
        for k in range(rounds):
            # 🔴 2026-09-25：连发本身要花 ~1 秒，而目标以 4–5 m/s 走 —— 70m 外 1 秒就是 3.5°，
            # 足够让后面几发整发打空（`RV3D_PROJ_DIAG` 里 41–50% 的子弹是"飞到寿命尽头"）。
            # 所以**每发之前**都拿最新样本重瞄一次（`S.aim` 自己会收敛，不必重算四次）。
            if k:
                t2 = S.log_tail(logpath)
                p2 = targets(t2).get(npc_id)
                if p2:
                    ppos2 = player_pos(t2) or ppos
                    ty, tp = target_angles_rel((npc_id, p2[0], p2[1], p2[2]), ppos2)
                    S.aim(hwnd, cx, cy, logpath, ty, tp, rounds=2)
            S.post_lbutton(hwnd, True, cx, cy)
            time.sleep(0.08)
            S.post_lbutton(hwnd, False, cx, cy)
            time.sleep(0.16)
        time.sleep(0.4)
        fired = shots_count(S.log_tail(logpath)) - s0
        if fired > 0:
            last_progress_at = time.time()
        if 0 <= fired < rounds:
            time.sleep(2.6)
            for _ in range(rounds - fired):
                S.post_lbutton(hwnd, True, cx, cy)
                time.sleep(0.08)
                S.post_lbutton(hwnd, False, cx, cy)
                time.sleep(0.16)
            time.sleep(0.4)
        # 打空枪的判据：**打出去了，但累计命中数没动** ⇒ 这一条射击线被掩体挡住
        # （2026-09-25 实测：环形工事外的 npc#34 卡在 (-13.1,-14.3)，harness 隔着墙连打
        # 4 把枪把它打「干」，`hits` 全程不动）。射不动就**换位置**，别继续喂子弹：
        # 先朝目标走 1.2s，走不动就侧移 1.2s（相当于贴墙找门），再来一轮。
        h1 = hits_now(S.log_tail(logpath))
        if fired >= 3 and h1 >= 0 and h1 == h0:
            blocked[npc_id] = blocked.get(npc_id, 0) + 1
            if blocked[npc_id] >= 2:
                moved = move_hold(hwnd, logpath, "w", 1.2)
                if moved < 0.5:
                    side = "d" if blocked[npc_id] % 2 == 0 else "a"
                    moved = move_hold(hwnd, logpath, side, 1.2)
                    where = side
                else:
                    where = "w"
                print("    no hit after %d rounds x%d -> move %s %.1fm (firing line blocked)"
                      % (fired, blocked[npc_id], where, moved), flush=True)
                blocked[npc_id] = 0
                attempts[npc_id] = 0
                if moved > 0.5:
                    last_progress_at = time.time()
                continue
        elif h1 > h0:
            blocked[npc_id] = 0
        sc1 = score_now(S.log_tail(logpath))
        if sc1 > sc0 >= 0:
            print("    KILL (score %d -> %d), enemies=%d"
                  % (sc0, sc1, wave_now(S.log_tail(logpath))[1]), flush=True)
            attempts[npc_id] = 99  # never re-engage a corpse

        if time.time() - last_shot_at >= args.shot_every:
            last_shot_at = time.time()
            shot = os.path.join(shotdir, "%s_t%.0f.png" % (tag, time.time() - t0))
            if not args.no_shot:
                screenshot(hwnd, shot)
            print("    shot -> %s" % os.path.basename(shot), flush=True)

    txt = S.log_tail(logpath)
    vuid = len(re.findall(r"VUID", txt))
    panics = len(re.findall(r"panic", txt, re.I))
    lost = len(re.findall(r"has been lost", txt))
    kills = len(re.findall(r"kill: npc #\d+ eliminated", txt))
    shots = len(re.findall(r"shot #", txt))
    # `hits=` = 玩家弹丸命中 NPC 的**累计**次数（引擎 1 Hz 状态行，2026-09-25 加）。
    # 这是唯一能判「改瞄法有没有用」的指标：`kills/shots` 里混着"打掩体"和"残局空点"。
    hitseries = re.findall(r"hits=(\d+)", txt)
    hits = int(hitseries[-1]) if hitseries else -1
    cleared = re.findall(r"wave: wave (\d+) cleared", txt)
    supply = re.findall(r"survive: 波间补给（血量 ([\d.]+)%", txt)
    # 每个波次只算一次：`spawn_wave` 可能对同一波打两条 spawned 行
    # （2026-09-25 通关那次 wave 5 出现两次 —— 胜利那一拍又刷了一波，引擎侧已修）。
    spawned = sorted(set(re.findall(r"wave: wave (\d+) spawned (\d+) enemies", txt)))
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
    print("  hits          : %d   (命中率 %.1f%%，理想 ≈%.1f 发/杀)"
          % (hits, (100.0 * hits / shots) if shots else 0.0,
             (hits / float(kills)) if kills else 0.0))
    print("  VUID=%d panics=%d device_lost=%d fps=%.1f"
          % (vuid, panics, lost, S.last_fps(txt)), flush=True)
    # 判据（2026-09-25 通关后收紧）：
    #  · VICTORY = 引擎自己打了 `survive: 全部 N 波守住` ⇒ 通关是**充分条件**，
    #    只要没有 VUID / panic / 丢设备就算 ALL-OK；
    #  · 没通关时，要求「每波都有 spawned、都 cleared、补给窗口数 = 波数-1」——
    #    之前的写法用 `len(cleared) == len(spawned)`，同一波两条 spawned 行就会假红。
    clean = vuid == 0 and panics == 0 and lost == 0
    if result == "VICTORY":
        ok = clean and bool(victory)
    else:
        ok = (clean and len(cleared) == len(spawned)
              and len(supply) == max(len(cleared) - 1, 0))
    print("RESULT: %s" % ("ALL-OK" if ok else "CHECK"), flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
