#!/usr/bin/env bash
# smoke_linux.sh —— Linux 侧游戏冒烟门（启动 + 收尾；判据在 smoke_linux.py）
#
# 与 Windows 侧的关系
# -------------------
# 判据**逐字同口径**：`vuid == 0 and panics == 0 and killed >= 1`（无 fps 门槛）。
# 但**驱动方式完全不同**，这不是偷懒，是平台决定的：
#
#   Windows：`run_smoke_pm.ps1` + `gameplay_smoke_pm.py` 用 `PostMessage` 把按键
#            投进目标窗口队列 —— 因为那边**不能抢前台**（抢了 winit 就不发 Focused，
#            光标永不抓取；用户 2026-09-03 报的"鼠标死锁"就是这一类）。
#   Linux  ：**没有 PostMessage 这条路**，XTEST 是全局注入、会抢焦点、还会把指针
#            锁进游戏窗口 —— 正好违反 Windows 侧那条"鼠标安全协议"的本意。
#            ⇒ 改为**零输入注入**：用引擎自己的三个诊断开关把"目标搬到准星上 + 每帧开火"
#            拼出来，一根手指都不用碰。
#
# 三个开关（缺任何一个都到不了 killed>=1，见各自的注释）：
#   RV3D_AUTOSTART=1       进 Playing（绕过菜单）
#   RV3D_DIAG_NPC_FRONT=1  每帧把 npcs[0] 摆到**相机正前方 20m**（与开火弹道同源）
#   RV3D_AUTOFIRE=1        Playing 态每帧 fire_requested=true
# ⚠️ 只有 AUTOFIRE 没有 DIAG_NPC_FRONT 就是"对着空气开枪"，killed 永远是 0。
#
# 🔴 鼠标安全：**永远带 `RV3D_NO_CAPTURE=1`**。这是代码级保证（`capture_wanted` 恒假），
# 不依赖"但愿这个脚本记得别抢焦点"。跑冒烟期间你的指针不会被夺走。
#
# 用法：
#   scripts/smoke_linux.sh [-Secs N]        # 默认 60 秒
# 退出码（三态，教训 46）：
#   0 = 通过    1 = 失败    2 = **没跑成**（exe 不存在 / 引擎没进 Playing / 日志空）
#
# 前置：先构建（`cargo build --release`，或直接跑一次 `./SteelFront.sh fast`）。
set -euo pipefail

# --- 定位仓库根（脚本可能被软链接到别处调用）---
src="${BASH_SOURCE[0]}"
while [ -L "$src" ]; do
    dir="$(cd -P "$(dirname "$src")" && pwd)"
    src="$(readlink "$src")"
    [[ "$src" != /* ]] && src="$dir/$src"
done
cd "$(cd -P "$(dirname "$src")" && pwd)/.."
repo="$(pwd)"

SECS=60
REQUIRE_KILL=0
while [ $# -gt 0 ]; do
    case "$1" in
        -Secs|--secs) SECS="${2:?-Secs 后面要给秒数}"; shift 2 ;;
        -RequireKill|--require-kill) REQUIRE_KILL=1; shift ;;
        -h|--help) sed -n '2,52p' "$0"; exit 0 ;;
        *) echo "smoke_linux: 不认识的参数 $1（-h 看用法）" >&2; exit 2 ;;
    esac
done
case "$SECS" in (*[!0-9]*|'') echo "smoke_linux: -Secs 必须是正整数，收到 '$SECS'" >&2; exit 2 ;; esac

# 🔴 图形会话环境预检（2026-10-03 实测踩到）
# 引擎需要 WAYLAND_DISPLAY（或 DISPLAY）才能建事件循环，而从 TTY/自动化 shell 里跑时
# 这些变量不在（实测 XDG_SESSION_TYPE=tty）⇒ 引擎报 "neither WAYLAND_DISPLAY nor ..."。
# 修之前它还会**以 0 退出**，把调用方骗过去（引擎侧已改为非零退出）。
# 这里能自动补就补，补不上就 fail-closed 退 2 —— "没跑成"必须说出口。
if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${DISPLAY:-}" ]; then
    _rt="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
    if [ -S "$_rt/wayland-0" ]; then
        export WAYLAND_DISPLAY=wayland-0
        export DISPLAY="${DISPLAY:-:0}"
        echo "smoke_linux: 补上图形会话环境 WAYLAND_DISPLAY=$WAYLAND_DISPLAY DISPLAY=$DISPLAY"
    else
        echo "smoke_linux: 没跑成 —— 既没有 WAYLAND_DISPLAY/DISPLAY，$_rt/wayland-0 也不存在。" >&2
        echo "             请在图形会话的终端里跑，或先 export WAYLAND_DISPLAY=wayland-0。" >&2
        exit 2
    fi
fi

EXE="$repo/target/release/steel-front"
LOG="$repo/logs/smoke_linux.log"

if [ ! -x "$EXE" ]; then
    echo "smoke_linux: 找不到 $EXE —— 先 cargo build --release（或 ./SteelFront.sh fast）" >&2
    echo "smoke_linux: exit 2（没跑成，不是失败）" >&2
    exit 2
fi

mkdir -p "$repo/logs"
# 清场：残留实例会抢设备/占日志。首次运行时没有进程，别让 set -e 把脚本打断。
pkill -x steel-front 2>/dev/null || true
for _ in $(seq 20); do pgrep -x steel-front >/dev/null 2>&1 || break; sleep 0.1; done
# 旧日志必须删干净：判据读的是这两个文件，留着上一轮的就是"读到别人的成绩"
rm -f "$LOG" "$LOG.err"

# --- 环境变量：只在未设置时补默认值，绝不覆盖调用方显式给的值 ---
# 理由与 SteelFront.sh 一致：调用方可能在做 A/B（例如想关掉验证层量帧率）。
export RV3D_NO_CAPTURE="${RV3D_NO_CAPTURE:-1}"          # 鼠标安全，代码级保证
# 🔴 `RV3D_STRESS_AI=1`（压力模式）——**与 Windows 侧的 run_smoke_pm.ps1 相反**，这是实测结论。
#
# Windows 脚本设 `=0`（波次模式），它那边的注释理由是「压力模式下玩家无敌、killed 不可达」。
# 但在 Linux 这条**零输入**路径上实测下来是**反过来**的（2026-09-28，见下），
# 所以这里不能照抄 —— 照抄的结果是闸门 `2/2` 全红：
#
#   压力模式（255 敌人 + 玩家无敌）：3/3 通过，score 增量 30 / 20 / 20（3 / 2 / 2 杀）
#   波次模式（6 敌人，无论加不加 RV3D_INVINCIBLE=1）：0/2 通过，score 增量**恒为 0**
#
# 机制（不是猜的，是从日志读出来的）：零输入路径**没法修正瞄准** ——
# `RV3D_AUTOFIRE` 每帧开火会累积后坐力与散布，`RV3D_DIAG_NPC_FRONT` 只把 npcs[0] 摆在
# 20m 正前方，子弹并不保证命中。所以它靠的是"**在足够多的目标里蒙中几个**"：
#   - 压力模式：255 个敌人 + 玩家无敌 ⇒ 整局 45s 都活着、一直有机会蒙中 ⇒ 稳定 ≥2 杀；
#   - 波次模式：只有 6 个敌人（样本太少），且玩家会在 ~27s 被打死 ⇒
#     `game: player down ... (GameOver: gameplay frozen, projectiles coast without kills)`
#     ⇒ 之后**再也打不出击杀**（日志里 224 发 `weapons: shot` 全部 `hits=0`）。
# ⚠️ 想让 Linux 侧也跑波次模式，得先有一条**闭环瞄准**的驱动（Windows 侧是
# `gameplay_smoke_pm.py` 的 `aim` 闭环），那是另一个工作量，不在本次范围。
#
# ⇒ 代价说清楚：本闸门跑的是**压力模式**，不是默认玩法路径。它证明的是
# "渲染/物理/AI/开火/命中/计分/验证层在这台机器上整条链路是通的"，
# 不证明"波次模式的关卡推进正常"（那需要闭环瞄准或人工试玩）。
export RV3D_STRESS_AI="${RV3D_STRESS_AI:-1}"
export RV3D_AUTOSTART="${RV3D_AUTOSTART:-1}"            # 进 Playing
export RV3D_DIAG_NPC_FRONT="${RV3D_DIAG_NPC_FRONT:-1}"  # 把目标搬到准星上（与弹道同源）
export RV3D_AUTOFIRE="${RV3D_AUTOFIRE:-1}"              # 每帧开火
# 判据里含 vuid==0 ⇒ 验证层必须真的开着，否则那一项是恒真的。
# DISABLE_RTSS_LAYER / DISABLE_GAMEPP_LAYER 是 **Windows 专有**的隐式层压制，Linux 不设。
export RV3D_VALIDATION="${RV3D_VALIDATION:-1}"
# 失焦上限关掉：本脚本是后台跑法，若不关，引擎会在开局后被静默压到 20fps
# （引擎默认 0 = 不限，正是为了让失焦的自动化跑法不被静默改变语义）。
export RV3D_BG_FPS="${RV3D_BG_FPS:-0}"
# 玩家路径用的呈现模式（独显长跑下 FIFO 会锁死；IMMEDIATE 在真显示器上持续撕裂）。
export RV3D_PRESENT_MODE="${RV3D_PRESENT_MODE:-mailbox}"

echo "smoke_linux: 跑 ${SECS}s，日志 -> logs/smoke_linux.log(.err)"
echo "smoke_linux: RV3D_NO_CAPTURE=$RV3D_NO_CAPTURE（指针不会被捕获）VALIDATION=$RV3D_VALIDATION"

"$EXE" >"$LOG" 2>"$LOG.err" &
pid=$!
# 硬超时 + 收尾：**必须在 trap 里 kill**，否则一次卡死会留下一个抓不到光标的僵尸进程
# （Windows 侧 cap_safe.ps1 的 finally + 硬超时就是为这件事存在的）。
cleanup() {
    kill -9 "$pid" 2>/dev/null || true
    pkill -x steel-front 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# 等它跑满，或提前退出（崩溃/自己关了）
for _ in $(seq $((SECS * 10))); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
done
alive=0; kill -0 "$pid" 2>/dev/null && alive=1
if [ "$alive" = 1 ]; then
    echo "smoke_linux: 跑满 ${SECS}s，收尾"
else
    # 提前退出不一定是失败（可能是面板关闭），但如果日志里没有 Playing 行，判据会判 exit 2。
    echo "smoke_linux: 进程提前退出（引擎自己结束了）—— 继续判据"
fi
cleanup
trap - EXIT

# 判据交给 python（同一个文件可以被离线复核：任何一次跑完的日志都能重判一遍）
if [ "$REQUIRE_KILL" = 1 ]; then
    python3 "$repo/scripts/smoke_linux.py" "$LOG" --require-kill
else
    python3 "$repo/scripts/smoke_linux.py" "$LOG"
fi
