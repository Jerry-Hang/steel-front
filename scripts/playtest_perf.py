#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""playtest_perf.py —— 压力模式压力与 AI 智能实测（时长制）

启动（或附加）RV3D_STRESS_AI=64 压力模式游戏：红蓝各 64 NPC 互射 + 一队
团灭全量补员（无限波次）。脚本跑满 PT_SECS 秒（默认 600s），期间持续：
① 采样帧数/CPU/GPU/硬件占用与 AI 行为（状态分布、8 种战术分布、互射阵亡、
波次、爆炸）；② 周期截图供检查 AI 战场推进/贴图/剔除/阴影；③ 玩家自动
瞄准点射（击杀为附带指标，不阻塞完成；M1 150 发耗尽后转旁观 AI 互射）。
最后输出实测报告。核心目的 = 两波 AI 互打对电脑的压力与 AI 智能表现。

用法：
    python3 scripts/playtest_perf.py            # 新启动压力模式游戏
    python3 scripts/playtest_perf.py --attach   # 附加已运行的游戏（手动启动的）
    python3 scripts/playtest_perf.py --no-shadow  # 传 RV3D_NO_SHADOW=1 给游戏（阴影 A/B）

环境变量：
    PT_SECS    测试总时长秒数（默认 600）
    PT_SHOT_EVERY 周期截图间隔秒数（默认 120）
    PT_MAX_DIST 只瞄该距离（米）内的站定目标（默认 70；更远命中率骤降）
    PT_FRESH   站定日志新鲜度窗口秒数（默认 20；防幽灵目标，见下）
    PT_REFOCUS 周期夺焦间隔秒数（默认 45；WSLg 焦点会漂移）
    PT_FPS_MIN / PT_FPS_P50 帧数下限（默认 60 / 120；仅报告参考不判 FAIL）
    PT_WIN_COUNTER 1=额外采样 Windows GPU Engine 计数器（默认关：powershell.exe
      每秒从 WSL 拉起 Windows 进程会抢桌面焦点/拖慢采样，且该值与 nvidia-smi 冗余）

说明：
- 目标筛选 = 站定日志新鲜度窗口 + 距离上限：压力模式战场互射（DPS 结算）
  会静默移除 NPC——只记 `battle: 阵亡 N` 总数、没有按 ID 的死亡日志，历史
  "站定"条目会变成幽灵目标（玩家射击全空）。NPC 每次重新进入攻击态都会
  重打站定日志（实测间隔 1-10s），因此只取最近 PT_FRESH 秒内、且在当前
  波次（最近一次"全量补员开新轮"之后）的条目 = 仍活着且仍在交火的目标；
  死亡 NPC 的旧条目自然过期。实测修复前 303 发仅 30 命中（9.9%，多数打空
  在幽灵上）、每轮 0-2 杀；修复后只打新鲜活目标，命中率显著提升。
- 瞄准 = 开环注入 + 粗校验：读 cam 一次 → 按差值分块注入（≤400px/事件，
  按住左键拖拽，捕获态/非捕获态都生效）→ 等一帧 1Hz cam 日志校验，脱靶
  >12° 补正（≤2 轮），仍 >30° 放弃换目标。不做细粒度闭环收敛——1Hz 日志
  下逐像素收敛太慢；实测 Xwayland 注入偶发 ~20× 放大，粗校验专治此病，
  其余 1:1 精确（probe 验证）。首轮点射有命中时第二轮直接补射不重瞄
  （省 ~3s/目标，命中即收手）。
- 弹药管理：M1 共 150 发（30 弹匣 + 120 备弹），耗尽后只能空枪；脚本每轮
  重启游戏获得全新 150 发，中轮空枪检测（脚本点击数 - 游戏确认射击 >20）
  触发中轮重启，绕开 WSLg 下不可靠的设置面板补给（ContextMenu 键投递实测
  时灵时不灵）。
- 瞄准注入按住左键拖拽（冒烟脚本机制）：WSLg/Xwayland 下窗口焦点/捕获会
  漂移（实测开局约 60s 后 `input: cursor released` → 捕获态绝对路径失效，
  视角冻结），拖拽路径在捕获态/非捕获态都走 CursorMoved 转视角，不依赖焦点。
- 压枪补偿：每发后坐力 kick_pitch=0.014rad（≈0.8°），点射后注入反向
  鼠标位移拉回准星，保证 4 发全中（M1 25 伤害 × 4 = 100hp）。
- 需 X11/Vulkan 环境（WSLg 或 Xvfb）；游戏窗口会显示在桌面，脚本会置顶。
"""

import ctypes
import datetime
import glob
import math
import os
import re
import struct
import subprocess
import sys
import threading
import time
import zlib

# ---------------------------------------------------------------- 配置
TOTAL_SECS = int(os.environ.get("PT_SECS", "600"))          # 压力测试总时长（秒）
SCREENSHOT_EVERY = int(os.environ.get("PT_SHOT_EVERY", "120"))  # 周期截图间隔（秒）
MAX_AIM_DIST = float(os.environ.get("PT_MAX_DIST", "70"))
STAND_FRESH_S = float(os.environ.get("PT_FRESH", "20"))
REFOCUS_EVERY = float(os.environ.get("PT_REFOCUS", "45"))
FPS_MIN = float(os.environ.get("PT_FPS_MIN", "60"))     # 帧数硬下限（压力模式混战瞬时帧率会跌）
FPS_P50 = float(os.environ.get("PT_FPS_P50", "120"))    # 帧数中位数下限（默认冒烟同款 120）
TACTIC_NAMES = ["突进", "包抄", "偷袭", "压制", "掩体跃进", "撤退", "站定", "掩体利用"]
NO_SHADOW = "--no-shadow" in sys.argv
ATTACH = "--attach" in sys.argv
WIN_COUNTER = os.environ.get("PT_WIN_COUNTER", "0") == "1"
LOG = "/tmp/playtest.log"
STRESS_SIDES = os.environ.get("PT_SIDES", "64")
EYE = 1.6  # 玩家眼睛高度
KICK_PITCH_RAD = 0.014  # 每发后坐力（弧度），见 weapons.rs Firearm::kick_pitch


def load_sens():
    """从 ~/.steel_front.cfg 读真实灵敏度（与 main.rs 公式一致）。"""
    try:
        with open(os.path.expanduser("~/.steel_front.cfg")) as f:
            for line in f:
                if line.startswith("sensitivity="):
                    s = float(line.split("=", 1)[1])
                    return 0.0005 + s * 0.002
    except (OSError, ValueError):
        pass
    return 0.0015


SENS = load_sens()  # rad/px
DEG_PX = math.degrees(SENS)  # 度/像素
COMP_PX = max(1, round(KICK_PITCH_RAD / SENS))  # 每发压枪像素（0.8°/0.086° ≈ 9px）

# ---------------------------------------------------------------- X11 封装
x11 = ctypes.CDLL("libX11.so.6")
xtst = ctypes.CDLL("libXtst.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XDefaultRootWindow.restype = ctypes.c_ulong
x11.XFetchName.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_char_p)]
x11.XQueryTree.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong),
                           ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)),
                           ctypes.POINTER(ctypes.c_uint)]
x11.XMapRaised.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
x11.XGetInputFocus.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int)]
x11.XTranslateCoordinates.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int,
                                      ctypes.c_int, ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int),
                                      ctypes.POINTER(ctypes.c_ulong)]
x11.XWarpPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
                             ctypes.c_uint, ctypes.c_uint, ctypes.c_int, ctypes.c_int]
x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XKeysymToKeycode.restype = ctypes.c_ubyte
x11.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
                          ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int]
x11.XGetImage.restype = ctypes.c_void_p
x11.XDestroyImage.argtypes = [ctypes.c_void_p]
xtst.XTestFakeRelativeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]

_d = x11.XOpenDisplay(None)
if not _d:
    sys.exit("no display (需要 X11 环境，如 WSLg/Xvfb；沙箱内请提权运行)")
d = _d
root = x11.XDefaultRootWindow(d)


def flush():
    x11.XFlush(d)
    time.sleep(0.02)


def find_window(win, name):
    nm = ctypes.c_char_p()
    if x11.XFetchName(d, win, ctypes.byref(nm)) and nm.value:
        if name in nm.value.decode(errors="ignore"):
            return win
    rr = ctypes.c_ulong()
    pp = ctypes.c_ulong()
    ch = ctypes.POINTER(ctypes.c_ulong)()
    n = ctypes.c_uint()
    if x11.XQueryTree(d, win, ctypes.byref(rr), ctypes.byref(pp), ctypes.byref(ch), ctypes.byref(n)):
        for i in range(n.value):
            r = find_window(ch[i], name)
            if r:
                return r
    return None


def log_tail():
    try:
        with open(LOG) as f:
            return f.read()
    except FileNotFoundError:
        return ""


def window_size_from_log():
    m = re.search(r"窗口大小变化: (\d+)x(\d+)", log_tail())
    return (int(m.group(1)), int(m.group(2))) if m else (1280, 720)


def activate():
    global win
    win = find_window(root, "Steel Front")
    t0 = time.time()
    while not win and time.time() - t0 < 30:
        time.sleep(0.5)
        win = find_window(root, "Steel Front")
    if not win:
        print("NO-WINDOW", flush=True)
        return False
    t0 = time.time()
    while time.time() - t0 < 15:
        if "cam:" in log_tail():
            break
        time.sleep(0.3)
    print(f"window 0x{win:x} ready size={window_size_from_log()}", flush=True)
    x11.XMapRaised(d, win)
    flush()
    time.sleep(0.5)
    return True


def warp_to_center(quiet=False):
    size = window_size_from_log()
    rx = ctypes.c_int()
    ry = ctypes.c_int()
    child = ctypes.c_ulong()
    if not x11.XTranslateCoordinates(d, win, root, 0, 0, ctypes.byref(rx), ctypes.byref(ry), ctypes.byref(child)):
        return
    x11.XWarpPointer(d, 0, root, 0, 0, 0, 0, rx.value + size[0] // 2, ry.value + size[1] // 2)
    flush()
    time.sleep(0.12)


def refocus_window():
    """点击窗口中心夺焦 + XSetInputFocus 兜底；返回焦点是否落在游戏窗口。

    WSLg/Xwayland 下窗口焦点会漂移（实测开局约 60s 后 capture 被释放，
    捕获态绝对路径失效、F12 收不到键盘事件）。真实点击是唯一可靠夺焦方式
    （shot_fixed.py 实证）；点击会顺带开一枪，无妨。
    """
    size = window_size_from_log()
    rx = ctypes.c_int()
    ry = ctypes.c_int()
    child = ctypes.c_ulong()
    if not x11.XTranslateCoordinates(d, win, root, 0, 0, ctypes.byref(rx), ctypes.byref(ry), ctypes.byref(child)):
        return False
    cx0, cy0 = rx.value + size[0] // 2, ry.value + size[1] // 2
    xtst.XTestFakeMotionEvent(d, -1, cx0, cy0, 0)
    flush()
    time.sleep(0.15)
    xtst.XTestFakeButtonEvent(d, 1, 1, 0)
    xtst.XTestFakeButtonEvent(d, 1, 0, 0)
    flush()
    time.sleep(0.25)
    foc = ctypes.c_ulong()
    rev = ctypes.c_int()
    x11.XGetInputFocus(d, ctypes.byref(foc), ctypes.byref(rev))
    if foc.value != win:
        x11.XSetInputFocus(d, win, 1, 0)
        flush()
        time.sleep(0.15)
        x11.XGetInputFocus(d, ctypes.byref(foc), ctypes.byref(rev))
    print(f"  focus: {'OK' if foc.value == win else 'FAIL'} (0x{foc.value:x})", flush=True)
    return foc.value == win


def cam_now(txt):
    m = re.findall(r"cam: yaw=([-\d.]+) pitch=([-\d.]+)", txt)
    if not m:
        return None
    return (float(m[-1][0]), float(m[-1][1]))


def keycode(keysym):
    return x11.XKeysymToKeycode(d, keysym)


def press_release(keysym):
    kc = keycode(keysym)
    xtst.XTestFakeKeyEvent(d, kc, 1, 0)
    flush()
    time.sleep(0.08)
    xtst.XTestFakeKeyEvent(d, kc, 0, 0)
    flush()


def aim_at(yaw_tgt_deg, pitch_tgt_deg):
    """开环注入 + 粗校验补正（最多 2 轮）；返回最终偏差 (dyaw, dpitch) 或 None。

    实测 WSLg/Xwayland 下 XTest 注入**偶发 ~20× 放大**（约 1/60 次瞄准单次
    yaw 过转 500°+、pitch 打到 ±75°，其余 1:1 精确）。纯开环一次性注入会在
    放大事件时打空整轮点射；闭环收敛又受 1Hz cam 日志拖累。折中：注入后等
    一帧新 cam 日志（≤1.2s），偏差 >12° 补正一次，仍不收敛则交给调用方决定。
    按住左键拖拽注入：捕获态/非捕获态都生效（不依赖窗口焦点/捕获状态）。
    """
    for _ in range(2):
        warp_to_center()
        cur = cam_now(log_tail())
        if not cur:
            time.sleep(1.0)
            continue
        dyaw = ((yaw_tgt_deg - cur[0] + 540.0) % 360.0) - 180.0
        dpitch = pitch_tgt_deg - cur[1]
        dx = int(dyaw / DEG_PX)
        dy = int(dpitch / DEG_PX)
        if abs(dx) <= 2 and abs(dy) <= 2:
            return (dyaw, dpitch)  # 已在准星附近（≈0.17°）
        # 拖拽注入：按下左键（会顺带开一枪，无妨）→ 分块移动 → 松键
        xtst.XTestFakeButtonEvent(d, 1, 1, 0)
        flush()
        time.sleep(0.05)
        rx, ry = dx, dy
        while rx != 0 or ry != 0:
            cx = max(-400, min(400, rx))
            cy = max(-400, min(400, ry))
            xtst.XTestFakeRelativeMotionEvent(d, cx, cy, 0)
            flush()
            time.sleep(0.04)
            rx -= cx
            ry -= cy
        xtst.XTestFakeButtonEvent(d, 1, 0, 0)
        flush()
        # 粗校验：等一帧注入后的新 cam 日志（1Hz，最多等 1.2s）
        base_len = len(log_tail())
        t1 = time.time()
        while time.time() - t1 < 1.2:
            txt = log_tail()
            if len(txt) > base_len:
                m = re.findall(r"cam: yaw=([-\d.]+) pitch=([-\d.]+)", txt[base_len:])
                if m:
                    cur = (float(m[-1][0]), float(m[-1][1]))
                    dyaw = ((yaw_tgt_deg - cur[0] + 540.0) % 360.0) - 180.0
                    dpitch = pitch_tgt_deg - cur[1]
                    if abs(dyaw) <= 12.0 and abs(dpitch) <= 12.0:
                        return (dyaw, dpitch)  # 够准，直接开火
                    print(f"  aim 脱靶补正: err=({dyaw:.0f},{dpitch:.0f})°", flush=True)
                    break  # 进入下一轮补正
            time.sleep(0.1)
        else:
            return None  # 1.2s 内无新 cam 行：放弃校验，开火兜底
    return None  # 两轮后仍未收敛


def aim_point(npc):
    """计算朝向 NPC 命中球（头顶 +0.8m，半径 0.8m）的 yaw/pitch（度）。"""
    _, nx, ny, nz = npc
    cx, cy, cz = nx, ny + 0.8 - EYE, nz
    yaw = math.degrees(math.atan2(-cx, -cz))
    pitch = math.degrees(math.atan2(-cy, math.hypot(cx, cz)))
    return yaw, pitch, math.hypot(cx, cz)


def fire_with_compensation(n):
    """点射 n 发，每发后注入压枪补偿（+COMP_PX 向下拉回后坐力上跳）。"""
    for _ in range(n):
        xtst.XTestFakeButtonEvent(d, 1, 1, 0)
        flush()
        xtst.XTestFakeButtonEvent(d, 1, 0, 0)
        flush()
        time.sleep(0.12)
        xtst.XTestFakeRelativeMotionEvent(d, 0, COMP_PX, 0)
        flush()
        time.sleep(0.23)  # 射速 3/s → 0.35s 间隔，补偿 0.12s 内生效
    time.sleep(0.5)


# ---------------------------------------------------------------- 游戏启动
def ensure_game():
    if ATTACH:
        if subprocess.run(["pgrep", "-f", "target/release/steel-front"],
                          capture_output=True).returncode == 0:
            print("attach: 附加已运行的游戏", flush=True)
            return
        print("attach: 未找到运行中的游戏，改为新启动", flush=True)
    else:
        # 先清理旧实例：多实例并存会争抢 GPU/显存（fps 暴跌）且 XTest 注入
        # 会落到错误窗口（aim 全部失败）。pkill -x 只匹配进程名，不会误杀自身。
        subprocess.run(["pkill", "-x", "steel-front"], capture_output=True)
        time.sleep(1.5)
    env = dict(os.environ)
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("XDG_RUNTIME_DIR", None)
    env["DISPLAY"] = ":0"
    env["RV3D_STRESS_AI"] = STRESS_SIDES
    if NO_SHADOW:
        env["RV3D_NO_SHADOW"] = "1"
    logf = open(LOG, "w")
    subprocess.Popen(["./target/release/steel-front"], stdout=logf, stderr=subprocess.STDOUT,
                     stdin=subprocess.DEVNULL, env=env, start_new_session=True)
    print(f"spawn: 启动压力模式 RV3D_STRESS_AI={STRESS_SIDES}"
          + ("（RV3D_NO_SHADOW=1 阴影关闭）" if NO_SHADOW else ""), flush=True)


# ---------------------------------------------------------------- 性能采样
class PerfSampler(threading.Thread):
    """后台采样：游戏日志字段 + nvidia-smi + Windows GPU counter + 进程内存。"""

    def __init__(self, pid):
        super().__init__(daemon=True)
        self.pid = pid
        self.lock = threading.Lock()
        self.fps = []
        self.cull_us = []
        self.ai_us = []
        self.phys_us = []
        self.frame_us = []
        self.present_us = []
        self.wait_fence_us = []
        self.visible = []
        self.near = []
        self.far = []
        self.marker = []
        self.npc = []
        self.cycle_us = []
        self.update_us = []
        self.render_us = []
        self.ai_npcs = []
        self.ai_chase = []
        self.ai_attack = []
        self.tactics = [0.0] * 8
        self.deaths = 0
        self.waves = 0
        self.explosions = 0
        self.gpu_util = []
        self.vram_mb = []
        self.win_gpu_util = []
        self.rss_mb = []
        self.running = True

    def run(self):
        last = ""
        while self.running:
            try:
                txt = log_tail()
                if txt != last:
                    last = txt
                    self._sample_log(txt)
                self._sample_hw()
            except Exception:
                pass
            time.sleep(1.0)

    def _sample_log(self, txt):
        def last_of(pat):
            m = re.findall(pat, txt)
            return float(m[-1]) if m else None

        fps = last_of(r"fps=([0-9.]+)")
        if fps is not None:
            self.fps.append(fps)
            for key, pat in (("cull_us", r"cull_us=([0-9.]+)"),
                             ("frame_us", r"frame_us=([0-9.]+)"),
                             ("present_us", r"present_us=([0-9.]+)"),
                             ("wait_fence_us", r"wait_fence_us=([0-9.]+)")):
                v = last_of(pat)
                if v is not None:
                    getattr(self, key).append(v)
        ai = last_of(r"ai_us=([0-9.]+)")
        if ai is not None:
            self.ai_us.append(ai)
        ph = last_of(r"phys_us=([0-9.]+)")
        if ph is not None:
            self.phys_us.append(ph)
        for key, pat in (("visible", r"visible=([0-9]+)/"),
                         ("near", r" near=([0-9]+) "),
                         ("far", r" far=([0-9]+) "),
                         ("marker", r" marker=([0-9]+) "),
                         ("npc", r" npc=([0-9]+)")):
            m = re.findall(pat, txt)
            if m:
                getattr(self, key).append(float(m[-1]))
        cyc = last_of(r"cycle_us=([0-9.]+)")
        if cyc is not None:
            self.cycle_us.append(cyc)
            upd = last_of(r" update_us=([0-9.]+)")
            rnd = last_of(r" render_us=([0-9.]+)")
            if upd is not None:
                self.update_us.append(upd)
            if rnd is not None:
                self.render_us.append(rnd)
        # AI 行为采样：状态分布 + 战术分布（AI 互射/包抄/偷袭等）。
        # 注意取**最后一条** ai: 行（re.search 会取到开局 wave=1 的 8 NPC）。
        ms = re.findall(r"ai: npcs=(\d+) near=(\d+) far=(\d+) idle=(\d+) patrol=(\d+) "
                        r"chase=(\d+) attack=(\d+) tactics=\[([^\]]+)\]", txt)
        if ms:
            m = ms[-1]
            self.ai_npcs.append(float(m[0]))
            self.ai_chase.append(float(m[5]))
            self.ai_attack.append(float(m[6]))
            try:
                ts = [float(x) for x in m[7].split(",")]
                for i, v in enumerate(ts[:8]):
                    self.tactics[i] += v
            except ValueError:
                pass
        self.deaths = len(re.findall(r"battle: 阵亡", txt))
        self.waves = len(re.findall(r"压力模式第 \d+ 轮开战", txt))
        self.explosions = len(re.findall(r"explosion: ", txt))

    def _sample_hw(self):
        try:
            out = subprocess.run(["/usr/lib/wsl/lib/nvidia-smi",
                                  "--query-gpu=utilization.gpu,memory.used",
                                  "--format=csv,noheader,nounits"],
                                 capture_output=True, text=True, timeout=4).stdout.strip()
            m = re.match(r"([\d.]+),\s*([\d.]+)", out)
            if m:
                self.gpu_util.append(float(m.group(1)))
                self.vram_mb.append(float(m.group(2)))
        except Exception:
            pass
        if not WIN_COUNTER:
            return
        try:
            out = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command",
                 "(Get-Counter '\\GPU Engine(*)\\Utilization Percentage' -MaxSamples 1)"
                 ".CounterSamples | Measure-Object -Property CookedValue -Sum | Select-Object -ExpandProperty Sum"],
                capture_output=True, text=True, timeout=8).stdout.strip()
            if out and re.match(r"[\d.]+", out):
                self.win_gpu_util.append(float(out.splitlines()[-1].strip()))
        except Exception:
            pass
        try:
            with open(f"/proc/{self.pid}/status") as f:
                m = re.search(r"VmRSS:\s+(\d+)", f.read())
                if m:
                    self.rss_mb.append(int(m.group(1)) / 1024.0)
        except Exception:
            pass

    def summary(self):
        def stat(arr, fn):
            return fn(sorted(arr)) if arr else None

        return {
            "fps": (stat(self.fps, lambda a: a[len(a) // 2]), stat(self.fps, min), stat(self.fps, max)),
            "cull_us": stat(self.cull_us, lambda a: a[len(a) // 2]),
            "ai_us": stat(self.ai_us, lambda a: a[len(a) // 2]),
            "phys_us": stat(self.phys_us, lambda a: a[len(a) // 2]),
            "frame_us": stat(self.frame_us, lambda a: a[len(a) // 2]),
            "present_us": stat(self.present_us, lambda a: a[len(a) // 2]),
            "wait_fence_us": stat(self.wait_fence_us, lambda a: a[len(a) // 2]),
            "visible": stat(self.visible, lambda a: a[len(a) // 2]),
            "near": stat(self.near, lambda a: a[len(a) // 2]),
            "far": stat(self.far, lambda a: a[len(a) // 2]),
            "marker": stat(self.marker, lambda a: a[len(a) // 2]),
            "npc": stat(self.npc, lambda a: a[len(a) // 2]),
            "cycle_us": stat(self.cycle_us, lambda a: a[len(a) // 2]),
            "update_us": stat(self.update_us, lambda a: a[len(a) // 2]),
            "render_us": stat(self.render_us, lambda a: a[len(a) // 2]),
            "ai_npcs": stat(self.ai_npcs, lambda a: a[len(a) // 2]),
            "ai_chase": stat(self.ai_chase, lambda a: a[len(a) // 2]),
            "ai_attack": stat(self.ai_attack, lambda a: a[len(a) // 2]),
            "tactics": list(self.tactics),
            "deaths": self.deaths,
            "waves": self.waves,
            "explosions": self.explosions,
            "gpu_util": stat(self.gpu_util, lambda a: a[len(a) // 2]),
            "vram_mb": stat(self.vram_mb, lambda a: a[len(a) // 2]),
            "win_gpu_util": stat(self.win_gpu_util, lambda a: a[len(a) // 2]),
            "rss_mb": stat(self.rss_mb, lambda a: a[len(a) // 2]),
        }


# ---------------------------------------------------------------- 主流程
def stands_after_run(txt, fresh_s=STAND_FRESH_S):
    """当前波次、fresh_s 秒内的站定 NPC（带时间戳过滤，防幽灵目标）。

    压力模式战场互射（DPS 结算）静默移除 NPC：只记 `battle: 阵亡 N` 总数、
    无按 ID 的死亡日志，历史"站定"条目会变成幽灵目标（射击全空）。NPC 每次
    重新进入攻击态都会重打站定日志（实测间隔 1-10s），因此最近 fresh_s 秒内
    的条目 ≈ 仍活着且仍在交火的目标；死亡 NPC 的旧条目自然过期。
    """
    base = max(txt.find("run started"), txt.rfind("全量补员开新轮"))
    if base < 0:
        return []
    now = datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
    out = []
    for m in re.finditer(
        r"\[(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})Z[^\]]*\] npc: #(\d+) stand "
        r"\(([-\d.]+), ([-\d.]+), ([-\d.]+)\)",
        txt,
    ):
        if m.start() <= base:
            continue
        try:
            ts = datetime.datetime.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S")
        except ValueError:
            continue
        if (now - ts).total_seconds() > fresh_s:
            continue
        out.append((int(m.group(2)), float(m.group(3)), float(m.group(4)), float(m.group(5))))
    return out


def kill_count(txt):
    return len(re.findall(r"kill:", txt))


def screenshot(tag):
    """截图：优先 XGetImage 直接从 X 服务器抓窗口像素（不依赖键盘焦点，
    WSLg 焦点漂移时 F12 收不到键事件——旧实现轮 1/2 截图因此失败）；
    失败回退 F12 游戏内读回。返回新 PNG 路径或 None。"""
    path = _grab_xgetimage()
    if path:
        return path
    before = set(glob.glob("/tmp/steel_front_*.png"))
    kc = keycode(0xFFC9)  # XK_F12
    if not kc:
        kc = 96  # 标准 X keymap F12 keycode 兜底（/tmp/shot_fixed.py 实测有效）
    x11.XMapRaised(d, win)
    x11.XRaiseWindow(d, win)
    flush()
    time.sleep(0.3)
    refocus_window()
    size = window_size_from_log()
    rx = ctypes.c_int()
    ry = ctypes.c_int()
    child = ctypes.c_ulong()
    if x11.XTranslateCoordinates(d, win, root, 0, 0, ctypes.byref(rx), ctypes.byref(ry), ctypes.byref(child)):
        cx0, cy0 = rx.value + size[0] // 2, ry.value + size[1] // 2
        xtst.XTestFakeMotionEvent(d, -1, cx0, cy0, 0)
        flush()
        time.sleep(0.15)
        xtst.XTestFakeButtonEvent(d, 1, 1, 0)
        xtst.XTestFakeButtonEvent(d, 1, 0, 0)
        flush()
        time.sleep(0.25)
    for attempt in range(3):
        xtst.XTestFakeKeyEvent(d, kc, 1, 0)
        flush()
        xtst.XTestFakeKeyEvent(d, kc, 0, 0)
        flush()
        t0 = time.time()
        while time.time() - t0 < 3.0:
            new = [p for p in glob.glob("/tmp/steel_front_*.png") if p not in before]
            if new:
                return new[0]
            time.sleep(0.3)
        time.sleep(0.5)
    return None


class _XImageHdr(ctypes.Structure):
    """XImage 前导字段（64 位布局，libX11 实测）——读前 15 个字段足够。"""
    _fields_ = [
        ("width", ctypes.c_int), ("height", ctypes.c_int), ("xoffset", ctypes.c_int),
        ("format", ctypes.c_int), ("data", ctypes.c_void_p), ("byte_order", ctypes.c_int),
        ("bitmap_unit", ctypes.c_int), ("bitmap_bit_order", ctypes.c_int),
        ("bitmap_pad", ctypes.c_int), ("depth", ctypes.c_int),
        ("bytes_per_line", ctypes.c_int), ("bits_per_pixel", ctypes.c_int),
        ("red_mask", ctypes.c_ulong), ("green_mask", ctypes.c_ulong),
        ("blue_mask", ctypes.c_ulong),
    ]


def _write_png(path, w, h, rows):
    def chunk(typ, data):
        return (struct.pack(">I", len(data)) + typ + data
                + struct.pack(">I", zlib.crc32(typ + data) & 0xffffffff))
    raw = b"".join(b"\x00" + row for row in rows)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 6)))
        f.write(chunk(b"IEND", b""))


def _grab_xgetimage():
    """XGetImage 抓取游戏窗口像素（ZPixmap）并写成 PNG；失败返回 None。"""
    size = window_size_from_log()
    rx = ctypes.c_int()
    ry = ctypes.c_int()
    child = ctypes.c_ulong()
    if not x11.XTranslateCoordinates(d, win, root, 0, 0, ctypes.byref(rx),
                                     ctypes.byref(ry), ctypes.byref(child)):
        return None
    imgp = x11.XGetImage(d, win, 0, 0, size[0], size[1], 0xFFFFFFFF, 2)  # 2=ZPixmap
    if not imgp:
        return None
    try:
        img = ctypes.cast(imgp, ctypes.POINTER(_XImageHdr)).contents
        if not img.data or img.width < 16 or img.height < 16 or img.bits_per_pixel < 24:
            return None
        bpp = img.bits_per_pixel // 8
        endian = "little" if img.byte_order == 0 else "big"
        data = ctypes.string_at(img.data, img.bytes_per_line * img.height)

        def sh(mask):
            s = 0
            while mask and not (mask & 1):
                mask >>= 1
                s += 1
            return s

        rs, gs, bs = sh(img.red_mask), sh(img.green_mask), sh(img.blue_mask)
        rows = []
        for y in range(img.height):
            base = y * img.bytes_per_line
            row = bytearray()
            for x in range(img.width):
                off = base + x * bpp
                px = int.from_bytes(data[off:off + bpp], endian)
                if img.red_mask and img.green_mask and img.blue_mask:
                    row += bytes(((px & img.red_mask) >> rs,
                                  (px & img.green_mask) >> gs,
                                  (px & img.blue_mask) >> bs))
                elif bpp == 4 and endian == "little":  # 常见兜底：32bpp 小端 BGRA
                    row += bytes((data[off + 2], data[off + 1], data[off]))
                elif bpp >= 3:
                    row += bytes((data[off + 2], data[off + 1], data[off]))
                else:
                    return None
            rows.append(bytes(row))
        out = f"/tmp/steel_front_x11_{int(time.time())}.png"
        _write_png(out, img.width, img.height, rows)
        return out
    finally:
        x11.XDestroyImage(imgp)


def fire_at(npc, s):
    """持续补射同一目标至死：最多 4 轮点射（12 发）。

    波次靠后的满血 NPC 需 4-6 发命中（M1 单发伤害 ~20-25），旧版每目标最多
    2 轮（6 发）就打残不打死。命中持续则继续补枪（免重瞄直射）；连续两轮
    零命中才放弃（目标已被互射打死/移出=幽灵）。命中即收手。
    返回 (本目标新增命中, 本目标击杀数)，供调用方做波次消耗期判定。"""
    hits_start = len(re.findall(r"projectile hit", log_tail()))
    kills_start = kill_count(log_tail())
    yaw, pitch, dist = aim_point(npc)
    print(f"  aim npc #{npc[0]} pos=({npc[1]:.0f},{npc[2]:.0f},{npc[3]:.0f}) dist={dist:.0f}m "
          f"yaw={yaw:.1f} pitch={pitch:.1f}", flush=True)
    fired = 0
    last_gained = 0
    zero_streak = 0
    for burst in (3, 3, 3, 3):
        if fired > 0 and last_gained == 0:
            zero_streak += 1
            if zero_streak >= 2:
                break  # 连续两轮零命中：幽灵/已移出，换目标省弹药
        if fired == 0 or last_gained == 0:
            err = aim_at(yaw, pitch)
            if err is None or abs(err[0]) > 30.0 or abs(err[1]) > 30.0:
                # 两轮补正后仍大幅脱靶（Xwayland 偶发放大）：放弃，省弹药换目标
                if err is not None:
                    print(f"  aim 脱靶放弃: err=({err[0]:.0f},{err[1]:.0f})°", flush=True)
                break
        hits0 = len(re.findall(r"projectile hit", log_tail()))
        kills0 = kill_count(log_tail())
        fire_with_compensation(burst)
        fired += burst
        s["shots"] += burst
        gained = len(re.findall(r"projectile hit", log_tail())) - hits0
        s["hits"] += gained
        if kill_count(log_tail()) - kills0 > 0:
            s["kills"] += 1
            break
        last_gained = gained
    print(f"  fired {fired} shots -> hits={s['hits']} (累计 kills={s['kills']})", flush=True)
    return (s["hits"] - hits_start, s["kills"] - kills_start)


def run_stress(sampler):
    """时长制压力测试：跑满 TOTAL_SECS 秒，持续采样 AI 行为/性能/硬件占用。

    压力模式 = 红蓝各 64 NPC 互射 + 一队团灭全量补员（无限波次）。脚本只做
    两件事：① 持续瞄准点射（击杀为附带指标，不阻塞完成；M1 150 发耗尽后
    自动转旁观，AI 互射继续）；② 周期截图 + 后台采样（fps/CPU/GPU/AI 状态
    与战术分布）。返回 (统计, 最终截图路径)。"""
    t0 = time.time()
    s = {"kills": 0, "hits": 0, "shots": 0}
    tried = {}  # id -> 尝试次数（最多 2 次）
    last_capture = time.time()
    last_refocus = time.time()
    while time.time() - t0 < TOTAL_SECS:
        # 游戏进程意外退出则提前收尾（保留已采样数据）
        if subprocess.run(["pgrep", "-x", "steel-front"],
                          capture_output=True).returncode != 0:
            print("  WARN: 游戏进程已退出，提前结束测试", flush=True)
            break
        # 周期截图：观察 AI 战场推进 / 贴图 / 剔除 / 阴影
        if time.time() - last_capture > SCREENSHOT_EVERY:
            screenshot(f"t{int(time.time() - t0)}")
            last_capture = time.time()
        # 周期夺焦：WSLg 焦点漂移后尽力维持玩家输入（拖拽注入本身不依赖焦点）
        if time.time() - last_refocus > REFOCUS_EVERY:
            refocus_window()
            last_refocus = time.time()
        # M1 共 150 发，耗尽后转旁观（AI 互射与采样继续）
        if confirmed_shots() >= 150:
            time.sleep(1.0)
            continue
        latest = {}
        for st in stands_after_run(log_tail()):
            latest[st[0]] = st
        cands = [st for st in latest.values()
                 if tried.get(st[0], 0) < 2 and math.hypot(st[1], st[3]) <= MAX_AIM_DIST]
        cands.sort(key=lambda t: math.hypot(t[1], t[3]))
        if not cands:
            time.sleep(1.0)
            continue
        npc = cands[0]
        tried[npc[0]] = tried.get(npc[0], 0) + 1
        fire_at(npc, s)
    shot = screenshot("final")
    return s, shot


def pct(arr, q):
    a = sorted(arr)
    if not a:
        return None
    return a[min(len(a) - 1, int(len(a) * q))]


def confirmed_shots():
    """游戏日志确认的实际射击发数（weapons: shot #N 的最大序号；日志随游戏
    重启截断，故为当前游戏实例的值）。"""
    m = re.findall(r"weapons: shot #(\d+)", log_tail())
    return int(m[-1]) if m else 0


def current_pid():
    out = subprocess.run(["pgrep", "-f", "target/release/steel-front"],
                         capture_output=True, text=True).stdout.strip()
    return int(out.splitlines()[0]) if out else 0


def start_battle():
    """StartMenu 开局（Space/点击兜底）并等待开战；返回是否成功。"""
    txt = log_tail()
    if "run started" not in txt:
        warp_to_center()
        time.sleep(0.5)
        press_release(0x20)  # Space
        time.sleep(0.3)
        xtst.XTestFakeButtonEvent(d, 1, 1, 0)
        flush()
        xtst.XTestFakeButtonEvent(d, 1, 0, 0)
        flush()
        time.sleep(1.5)
    t0 = time.time()
    while "开战" not in log_tail() and time.time() - t0 < 20:
        time.sleep(0.5)
    return "开战" in log_tail()


def main():
    ensure_game()
    if not activate():
        sys.exit(1)
    if not start_battle():
        print("开战超时", flush=True)
        sys.exit(1)
    sampler = PerfSampler(current_pid())
    sampler.start()
    print(f"采样启动: pid={sampler.pid} 灵敏度={SENS:.5f} rad/px 压枪={COMP_PX}px/发", flush=True)
    print(f"压力测试: 总时长 {TOTAL_SECS}s | 击杀为附带指标，跑满时长即完成", flush=True)
    s, shot_path = run_stress(sampler)
    sampler.running = False

    # ---- 报告 ----
    print("\n===================== 压力与 AI 智能实测报告 =====================", flush=True)
    acc = (s["hits"] / s["shots"] * 100.0) if s["shots"] else 0.0
    print(f"总时长 {TOTAL_SECS}s | 玩家击杀 {s['kills']}（附带指标）| 命中 {s['hits']}/{s['shots']} "
          f"({acc:.0f}%) | 最终截图 {shot_path or '失败'}", flush=True)
    sm = sampler.summary()
    fps_p50, fps_min, fps_max = sm["fps"]
    print(f"\n[帧数] min {fps_min:.0f} | p50 {fps_p50:.0f} | p95 "
          f"{pct(sampler.fps, 0.95):.0f} | max {fps_max:.0f} fps", flush=True)
    print(f"[帧耗时] frame {sm['frame_us']:.0f}µs | wait_fence {sm['wait_fence_us']:.0f}µs | "
          f"present {sm['present_us']:.0f}µs", flush=True)
    print(f"[CPU 侧] ai {sm['ai_us']:.0f}µs | phys {sm['phys_us']:.0f}µs | cull {sm['cull_us']:.0f}µs | "
          f"cycle {sm['cycle_us']:.0f}µs (update {sm['update_us']:.0f}µs / render {sm['render_us']:.0f}µs)",
          flush=True)
    print(f"[剔除] visible p50 {sm['visible']:.0f}/65536 (near {sm['near']:.0f} / far {sm['far']:.0f}) | "
          f"marker {sm['marker']:.0f} | npc {sm['npc']:.0f}", flush=True)
    if sm["gpu_util"] is not None:
        win_util = f" | Windows GPU Engine {sm['win_gpu_util']:.0f}%" if sm["win_gpu_util"] is not None else ""
        rss = f"{sm['rss_mb']:.0f}MB" if sm["rss_mb"] is not None else "N/A"
        print(f"[GPU] nvidia-smi util {sm['gpu_util']:.0f}% | 显存 {sm['vram_mb']:.0f}MB{win_util}"
              f" | 游戏进程 RSS {rss}", flush=True)
    # AI 智能/互射行为：状态分布 + 战术分布 + 阵亡/波次/爆炸
    if sm["ai_npcs"] is not None:
        total = sum(sm["tactics"])
        parts = []
        for i, name in enumerate(TACTIC_NAMES):
            if total > 0 and sm["tactics"][i]:
                parts.append(f"{name} {sm['tactics'][i] / total * 100:.0f}%")
        print(f"[AI 行为] NPC 均值 {sm['ai_npcs']:.0f} | 追击 {sm['ai_chase']:.0f} / "
              f"攻击 {sm['ai_attack']:.0f} | 互射阵亡 {sm['deaths']} | 波次 {sm['waves']} 轮 | "
              f"爆炸 {sm['explosions']}", flush=True)
        print(f"[AI 战术] {' | '.join(parts) if parts else '无战术采样'}", flush=True)
    txt = log_tail()
    vuid = len(re.findall(r"VUID", txt))
    panics = len(re.findall(r"panicked", txt))
    errs = len(re.findall(r" ERROR ", txt))
    print(f"[健康] VUID={vuid} panics={panics} errors={errs}", flush=True)
    hp_vals = sorted(set(re.findall(r"hp=([0-9.]+)/", txt)))
    print(f"[玩家] 血量采样 {hp_vals}（压力模式应恒 100）", flush=True)
    print(f"[阴影] RV3D_NO_SHADOW={'1' if NO_SHADOW else '0'}（A/B 对照见截图）", flush=True)

    # 通过标准：跑满时长 + 无 VUID/panic + 玩家未掉血（击杀数不设门槛）
    ok = True
    reasons = []
    if vuid != 0:
        ok = False
        reasons.append(f"VUID 校验失败 ×{vuid}")
    if panics:
        ok = False
        reasons.append(f"panic ×{panics}")
    if hp_vals and any(float(h) < 90 for h in hp_vals):
        ok = False
        reasons.append(f"玩家掉血（{hp_vals}）——压力模式预期不受伤害，可能 NPC 投射物误伤")
    if fps_p50 is not None and fps_p50 < FPS_P50:
        print(f"[参考] 帧数中位数偏低（p50={fps_p50:.0f} < {FPS_P50:.0f}，压力测试仅供参考）", flush=True)
    if ok and not reasons:
        print("\nALL-OK（压力测试跑满时长，无 VUID/panic，玩家无恙）", flush=True)
    else:
        print("\nFAIL: " + "；".join(reasons), flush=True)
    return 0 if ok else 2


if __name__ == "__main__":
    sys.exit(main())
