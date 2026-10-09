#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""smoke_linux.py —— Linux 侧冒烟判据（只读日志、不启动游戏、不注入输入）

为什么判据要单独成一个文件
------------------------
`smoke_linux.sh` 负责"把游戏跑起来再杀掉"，本文件负责"读日志下结论"。
分开是为了让判据可以**离线复核**：任何一次跑完的日志都能重新判一遍，
不需要重跑游戏（Windows 侧 `gameplay_smoke_pm.py` 也是这个分工）。

判据（与 Windows 侧 `gameplay_smoke_pm.py:336` **同一口径**，不许分叉）
------------------------------------------------------------------
    vuid == 0 and panics == 0 and killed >= 1

- `killed` = 本次跑动的**敌方击杀数**。日志里的 `score` 每杀一个敌方 +10
  （`game.rs::damage_npc` 走 `victim_team != Blue` 分支），所以
  `killed = score 增量 / KILL_SCORE`。
  🔴 **不能拿 `kill: npc #N` 的行数当击杀数**：那里面混着 `enemy=false`
  的**友军/自伤**死亡（`survive` 与压力模式的手榴弹都会产生），
  它们**不加分**（AGENTS.md 教训 39）。判据只认 `score` 增量。
- 无 fps 门槛（`fps=` 只用于打印）。**两套口径别混**：时长制是另一套。

🔴 第三种结局：0 = 通过 / 1 = 失败 / 2 = **没跑成**（教训 46）
-------------------------------------------------------------
"没跑成"必须与"跑过了且通过"区分开，否则一个空文件就能骗过闸门
（2026-09-26 `survive_pm` 整整轮读了一份空文件却打 ALL-OK）。
本文件把它落成三条**硬条件**，任一不满足即 exit 2：
  1. 日志文件存在且非空（Windows 侧那条教训的直接对策）；
  2. 至少出现过一行 `game: wave=` —— 引擎真的进过 Playing；
  3. 至少出现过两次 —— 要有可比的前后两个 score 采样点。
  ⚠️ 只检查"没崩"是不够的：`--help` 也能让进程退出码为 0。

日志双读（勿回退）
----------------
`<path>` 与 `<path>.err` **两个都要读并拼接**：引擎的日志走 stderr
（stdout 那个 `.log` 常常是**空文件**，AGENTS.md 教训 11），
而验证层的 VUID 与 Rust 的 panic 也**只在 stderr**。
只读一个 = 拿到一份恒真的空判据。

    python3 scripts/smoke_linux.py logs/smoke_linux.log
"""

import os
import re
import sys

# 每杀一个敌方加多少分（`game.rs` 的 KILL_SCORE）。判据用它把 score 增量折成击杀数。
KILL_SCORE = 10

# `game: wave=1 enemies=255 enemy_hp=100 hp=100/100 score=0 pos=(0.0,0.0) ...`
# 只锚定到 score 为止：后面的字段（phys_us/ai_us/...）会随版本增减，
# 锚进去就等于把判据绑死在无关字段的格式上。
GAME_LINE = re.compile(r"game: wave=\d+ enemies=(\d+) .*? score=(\d+)")

# 🔴 **已知噪声**：这些 VUID 不是引擎用错了，不该让闸门红（教训 26：会喊狼来了的闸门
#    会训练人不再当回事）。与 Windows 侧 `gameplay_smoke_pm.py` 的 `known_driver` **同步**。
KNOWN_VUID = {
    "VUID-VkImageViewCreateInfo-usage-02275": "驱动回写 STORAGE（见 renderer/swapchain.rs 注释）",
    "VUID-VkSwapchainCreateInfoKHR-imageFormat-01778": "驱动回写 STORAGE（见 renderer/swapchain.rs 注释）",
    # 已结案 #23：RTSS / GamePP 两个**隐式层**给交换链塞 MUTABLE_FORMAT，不是引擎用法错误。
    # ⚠️ Linux 侧通常没有这两个层；留着是为了两侧判据**逐条对齐**，便于对照日志。
    "VUID-VkSwapchainCreateInfoKHR-flags-parameter": "RTSS/GamePP 隐式层注入，非引擎问题（已结案 #23）",
}
SCORE_ONLY = re.compile(r" score=(\d+)")


def read_logs(path):
    """读 `<path>` 与 `<path>.err` 并拼接。

    返回 (text, 实际读到的文件列表)。文件不存在**不算错误**（两个都可能没写），
    但调用方要能凭第二个返回值分辨"读了但为空"与"根本没这个文件"。
    """
    out, got = "", []
    for p in (path, path + ".err"):
        try:
            with open(p, encoding="utf-8", errors="replace") as f:
                out += f.read()
            got.append(p)
        except (FileNotFoundError, IsADirectoryError, PermissionError):
            pass
    return out, got


def score_track(txt):
    """按出现顺序返回所有 `score=N`（不止 `game:` 行 —— 死亡/结算行也带 score）。"""
    return [int(m.group(1)) for m in SCORE_ONLY.finditer(txt)]


def judge(txt, require_kill=False, validation_on=True):
    """⚠️ `validation_on=False` 时 VUID **不参与判定**（并在结论里明说不适用）。
    默认 True：自检用例里那些杜撰的 VUID 要能被判红。"""
    """纯函数：日志文本 -> (exit_code, 给人看的一行结论, 明细 dict)。

    抽成纯函数是为了能在没有游戏、没有 GPU 的机器上测三条分支（含 exit 2），
    也让"为什么判成这样"可以逐项打出来 —— 一个只说 PASS/FAIL 的闸门没法排障。

    🔴 **两档判据**（`require_kill`），因为"击杀"那一档在本机是**不确定的**：

    * 默认（`require_kill=False`）= **确定性闸门**：`vuid == 0 and panics == 0`
      **且真的进过 Playing**。它抓的是"崩了 / 出 VUID / 根本没进玩法"这三类真回归，
      而这三项在**每一次**跑动里都稳定成立（实测 8/8）。
    * `--require-kill` = **与 Windows 侧逐字同口径**（`killed >= 1`），
      但 Linux 的**零输入**驱动做不到稳定击杀：实测 8 次里 1 次为 0 杀
      （45s：3/2/2/2/1 杀；60s：0/3 杀）⇒ **约 88% 通过率**。
      ⇒ 它是**可选**的，且调用方必须知道这个数字。理由见 AGENTS.md 教训 26：
      **会喊狼来了的闸门会训练人不再当回事**，那种假警报与漏报一样有害。
      （Windows 侧能稳定要求 `killed>=1`，是因为它有 `aim` 闭环瞄准；
      Linux 这条零输入路径没有瞄准，只靠"在 255 个目标里蒙中几个"。）

    三态：0 = 通过 / 1 = 失败 / 2 = **没跑成**（教训 46）。
    ⚠️ 即使 `require_kill=False`，"没跑成"那三条也**永远**是硬条件 ——
    `vuid==0` 在一份**空日志**上同样是 0，不挡住就又是一个恒真判据
    （Windows 侧 `survive_pm` 就是这么骗过一整轮的）。
    """
    # 🔴 VUID 判据必须区分「真扫过 0 条」与「**验证层根本没开**」（教训 46 同形）：
    #    验证层不开时日志里永远不会有 VUID 字样 ⇒ `vuid == 0` 是**结构性恒真**、什么都证明不了。
    #    （Linux 侧 `smoke_linux.sh` 默认把它打开，所以这个洞平时被掩盖着；
    #      但 `smoke_linux.py` 能被直接调用，且 `RV3D_VALIDATION=0` 一设就退化。）
    codes = re.findall(r"VUID-[A-Za-z0-9-]+", txt)
    known_hits = {c: codes.count(c) for c in KNOWN_VUID if c in codes}
    unexpected = [c for c in codes if c not in KNOWN_VUID]
    # `vuid` 保留"总出现次数"，只用于打印（判据用 unexpected，不是它）
    vuid = len(re.findall(r"VUID", txt))
    panics = len(re.findall(r"panic", txt, re.I))
    # "没跑成"的三条硬条件
    if not txt.strip():
        return 2, "没跑成：日志是空的（读到了文件但没有内容）", {"vuid": vuid, "panics": panics}
    waves = GAME_LINE.findall(txt)
    if not waves:
        return 2, "没跑成：日志里没有 game: wave= 行 —— 引擎没进过 Playing", {"vuid": vuid, "panics": panics}
    scores = score_track(txt)
    if len(scores) < 2:
        return 2, "没跑成：只有 %d 个 score 采样点，无法比较前后" % len(scores), {"vuid": vuid, "panics": panics}

    delta = scores[-1] - scores[0]
    killed = delta // KILL_SCORE
    detail = {
        "vuid": vuid,
        "vuid_unexpected": sorted(set(unexpected)),
        "vuid_known": known_hits,
        "validation_on": validation_on,
        "panics": panics,
        "score_first": scores[0],
        "score_last": scores[-1],
        "delta": delta,
        "killed": killed,
        "enemies_last": int(waves[-1][0]),
        "wave_lines": len(waves),
        "require_kill": require_kill,
    }
    if panics:
        return 1, "FAIL：panics=%d" % panics, detail
    if validation_on and unexpected:
        return 1, "FAIL：**未知** VUID %s（已知噪声不计：%s）" % (
            sorted(set(unexpected)), ", ".join(sorted(known_hits)) or "无"), detail
    # 验证层没开 ⇒ VUID 不参与判定，但**必须明说**，不许假装查过（这正是本函数存在的理由）
    not_applicable = (not validation_on)
    if vuid and not validation_on:
        note = "（VUID 判据不适用：验证层未开；日志里那 %d 处 VUID 字样不计）" % vuid
        detail["vuid_note"] = note
    if require_kill and delta < KILL_SCORE:
        return 1, "FAIL（-RequireKill）：score 增量 %d < %d（击杀数 0）" % (delta, KILL_SCORE), detail
    if not validation_on:
        return 0, "ALL-OK（⚠️ VUID 判据**不适用**：验证层未开，这一项恒为 0，不代表查过）", detail
    if known_hits:
        return 0, "ALL-OK（VUID 已知噪声 %s，不计）" % ", ".join(
            "%s x%d" % (c, n) for c, n in sorted(known_hits.items())), detail
    return 0, "ALL-OK", detail


def self_check():
    """自带三态自检：**证明这个闸门会红**，而不是只会说 ALL-OK。

    为什么要有它（AGENTS.md 教训 46）：一个判定类工具如果"永远通过"，它比没有更糟 ——
    它会训练人相信绿灯。三条分支里最难被发现坏掉的恰恰是 exit 2 那几条：
    把空日志判成"通过"在 Windows 侧**真实发生过**（`survive_pm` 整整读了一份空文件
    却打 ALL-OK）。

    每个用例都是**合成日志**（不依赖真跑一局），所以它能在任何机器上跑。
    返回 (用例数, 失败描述列表)。
    """
    G = "game: wave=1 enemies=255 enemy_hp=100 hp=100/100 score=%d pos=(0.0,0.0) hits=0\n"
    # (用例名, 日志, 期望退出码, 是否要求击杀)
    cases = [
        # --- 确定性闸门（默认）：exit 2 的三条硬条件 ---
        ("空日志", "", 2, False),
        ("只有空白", "   \n\n", 2, False),
        ("非空但从没进 Playing", "初始化完成\n渲染器就绪\n", 2, False),
        ("只有 1 个 score 采样点", G % 0, 2, False),
        # --- 确定性闸门：能抓住的真回归 ---
        ("正常局 0->30，无 VUID 无 panic", (G % 0) + (G % 30), 0, False),
        ("0 杀也通过（默认档不要求击杀）", (G % 0) + (G % 0), 0, False),
        ("有 VUID => 失败", (G % 0) + (G % 30) + "VUID-vkCmdDraw-None-0000\n", 1, False),
        ("有 panic => 失败", (G % 0) + (G % 30) + "thread 'main' panicked at 'boom'\n", 1, False),
        ("vuidless 这种同形串不算误伤", (G % 0) + (G % 30) + "vuidless text\n", 0, False),
        # --- -RequireKill 档：与 Windows 同口径 ---
        ("RequireKill：恰好 1 杀 => 通过", (G % 0) + (G % 10), 0, True),
        ("RequireKill：0 杀 => 失败", (G % 0) + (G % 0), 1, True),
        ("RequireKill：不够一杀 0->3 => 失败", (G % 0) + (G % 3), 1, True),
        ("RequireKill：VUID 仍然优先于击杀", (G % 0) + (G % 30) + "VUID-x\n", 1, True),
        ("RequireKill：没进 Playing 仍是 exit 2（不被击杀档吞掉）", "初始化完成\n", 2, True),
        # --- VUID 判据本身的两条（与 Windows 侧对齐；这正是「结构性恒 0」的补丁）---
        ("已知噪声 VUID（驱动回写 02275）不算失败",
         (G % 0) + (G % 30) + "VUID-VkImageViewCreateInfo-usage-02275\n", 0, False, True),
        ("已知噪声 VUID（flags-parameter）不算失败",
         (G % 0) + (G % 30) + "VUID-VkSwapchainCreateInfoKHR-flags-parameter\n", 0, False, True),
        ("已知噪声 + 未知 VUID 混在一起 => 仍然失败",
         (G % 0) + (G % 30) + "VUID-VkImageViewCreateInfo-usage-02275\nVUID-vkCmdDraw-None-9999\n", 1, False, True),
        ("验证层没开 => 未知 VUID 也不参与判定（但结论会说不适用）",
         (G % 0) + (G % 30) + "VUID-vkCmdDraw-None-9999\n", 0, False, False),
    ]
    bad = []
    for case in cases:
        name, txt, want, rk = case[0], case[1], case[2], case[3]
        von = case[4] if len(case) > 4 else True
        got, verdict, _ = judge(txt, require_kill=rk, validation_on=von)
        if got != want:
            bad.append("%s：期望 exit %d，实得 %d（%s）" % (name, want, got, verdict))
    return len(cases), bad


def main(argv):
    if len(argv) == 2 and argv[1] == "--self-check":
        n, bad = self_check()
        for b in bad:
            print("  x %s" % b)
        print("SELF-CHECK: %d/%d 用例通过" % (n - len(bad), n))
        # 自检自己也要三态：跑到且有失败 = 1；用例数为 0 = 没跑成 = 2
        if n == 0:
            return 2
        return 1 if bad else 0
    args = [a for a in argv[1:] if a != "--require-kill"]
    require_kill = len(args) != len(argv[1:])
    if len(args) != 1:
        print("用法: smoke_linux.py <日志路径> [--require-kill]", file=sys.stderr)
        print("      smoke_linux.py --self-check", file=sys.stderr)
        return 2
    path = args[0]
    txt, got = read_logs(path)
    print("读取: %s" % (", ".join(got) if got else "(两个文件都不存在)"))
    print("判据档: %s" % ("-RequireKill（与 Windows 同口径，约 88% 稳定）" if require_kill
                          else "默认（确定性：只看 vuid/panics/进过 Playing）"))
    # 与 Windows 侧同一条判据：验证层没开时 VUID 项不成立，必须显式告知（教训 46）
    validation_on = os.environ.get("RV3D_VALIDATION", "").strip() not in ("", "0", "false", "False")
    if not validation_on:
        print("VUID 判据: **不适用** —— 验证层未开（RV3D_VALIDATION 未设/为 0）。"
              "这一项恒为 0，不代表查过；要它生效请 RV3D_VALIDATION=1")
    code, verdict, d = judge(txt, require_kill=require_kill, validation_on=validation_on)
    if d:
        print(
            "VUID=%s panics=%s score %s -> %s (delta %s = %s 杀) enemies_last=%s"
            % (
                d.get("vuid"),
                d.get("panics"),
                d.get("score_first"),
                d.get("score_last"),
                d.get("delta"),
                d.get("killed"),
                d.get("enemies_last"),
            )
        )
    print("RESULT: %s" % verdict)
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
