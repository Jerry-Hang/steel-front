#!/usr/bin/env bash
# play_watchdog.sh —— 游玩会话的心跳看门狗（对应 Windows 的 scripts/play_watchdog.ps1）
#
# 为什么不是「睡 N 秒然后杀」
# ------------------------
# 那个显而易见的设计（启动后睡 N 秒、到点就杀）在 2026-09-12 撞过两次：
# 上一轮留下的看门狗在**新一轮跑到一半**时开火，把游戏杀了，
# 现场看起来就是「窗口从来没起来」。教训 19 因此定了规矩：
# **看门狗必须依赖"这一局还活着的证据"，而不是"它自己启动了多久"。**
#
# 它怎么判
# -------
# 只有**三条同时成立**才动手：
#   1. 存在 steel-front 进程；
#   2. 心跳文件**存在**（= 某个 harness 认领了这一局）；
#   3. 心跳比 `-StaleSec` 更旧。
# 也就是「harness 驱动的游戏还开着，但已经没人在驱动它了」——
# 拥有它的那个进程死了、卡住了、或者被杀了。此时杀掉游戏并**核验**交还，
# 而不是假定交还成功。
#
# 🔴 **第 2 条是承重的，不许放宽**：没有它，看门狗也会在**用户自己手动启动**的游戏上
# 于 ~StaleSec 后开火。这个形态的 bug 已经真实发生过一次（教训 19）。
#
# 用法
# ----
#   scripts/play_watchdog.sh &                 # 每会话起一次，当后台任务常驻
#   scripts/play_watchdog.sh -StaleSec 60 -PollSec 5
#   scripts/play_watchdog.sh -Beat             # 喂一次心跳（harness 在跑的时候调它）
#
# 谁该喂心跳：**任何"长期无人值守驱动这一局"的脚本**（Linux 侧目前是手动/长跑场景）。
# 只跑 45~90 秒、自带硬超时与 trap 的脚本（smoke / resize_probe / perf_run）**不需要** ——
# 它们自己会收尾；喂了反而会让看门狗误以为"有人认领"，失去保护意义。
#
# 与 Windows 侧的差异
# -----------------
#   1. Windows 在开火后调 `release_input.ps1` 去解 `ClipCursor`；
#      **Linux 侧不需要**，因为鼠标安全是**代码级**的（`RV3D_NO_CAPTURE=1` ⇒
#      `capture_wanted` 恒假，进程退出后合成器自然收回指针）。
#      但"核验而不是假定"这条纪律照旧：开火后必须确认**进程真的没了**再报成功。
#   2. 心跳文件默认放 `$XDG_RUNTIME_DIR`（按用户隔离、注销即清），
#      而不是 Windows 的 `%TEMP%`。
set -euo pipefail

STALE=30
POLL=5
BEAT="${XDG_RUNTIME_DIR:-/tmp}/sf_play.beat"

while [ $# -gt 0 ]; do
    case "$1" in
        -StaleSec|--stale) STALE="${2:?-StaleSec 后面要给秒数}"; shift 2 ;;
        -PollSec|--poll)   POLL="${2:?-PollSec 后面要给秒数}"; shift 2 ;;
        -Beat|--beat)      BEAT="${2:?-Beat 后面要给路径}"; shift 2 ;;
        --touch)           # 喂一次心跳就退出（供 harness 调用）
            mkdir -p "$(dirname "$BEAT")"; touch "$BEAT"
            exit 0 ;;
        -h|--help)         sed -n '2,40p' "$0"; exit 0 ;;
        *) echo "play_watchdog: 不认识的参数 $1（-h 看用法）" >&2; exit 2 ;;
    esac
done
case "$STALE" in (*[!0-9]*|'') echo "play_watchdog: -StaleSec 必须是正整数" >&2; exit 2 ;; esac
case "$POLL"  in (*[!0-9]*|'') echo "play_watchdog: -PollSec 必须是正整数" >&2; exit 2 ;; esac

mkdir -p "$(dirname "$BEAT")"
echo "play_watchdog: 已启动 stale=${STALE}s poll=${POLL}s beat=$BEAT（心跳文件不存在时它不会碰任何游戏）"

while true; do
    sleep "$POLL"

    # 1) 没有游戏进程 ⇒ 什么都不做
    pgrep -x steel-front >/dev/null 2>&1 || continue

    # 2) 🔴 心跳文件必须存在。这一条同时表达两件事：
    #    "某个 harness 认领了这一局" 与 "那个 harness 还活着"。
    #    用户手动启动的游戏没有心跳文件 ⇒ 看门狗永远不碰它。
    [ -e "$BEAT" ] || continue

    # 3) 心跳太旧 ⇒ 驱动它的那一方已经没了
    now=$(date +%s)
    mtime=$(stat -c %Y "$BEAT" 2>/dev/null || echo 0)
    age=$(( now - mtime ))
    [ "$age" -ge "$STALE" ] || continue

    pids=$(pgrep -x steel-front | tr '\n' ',' | sed 's/,$//')
    echo "play_watchdog: **开火** $(date '+%H:%M:%S')：游戏 pid=$pids 仍在，但心跳已旧 ${age}s ⇒ 杀进程"
    pkill -x steel-front >/dev/null 2>&1 || true
    sleep 1

    # 核验，而不是假定（照抄 Windows 侧那条纪律）。分两种失败，因为它们的下一步不同。
    if pgrep -x steel-front >/dev/null 2>&1; then
        echo "play_watchdog: ⚠️ 杀完之后**仍有残存**：$(pgrep -x steel-front | tr '\n' ' ') —— 再杀一次" >&2
        pkill -9 -x steel-front >/dev/null 2>&1 || true
        sleep 1
        if pgrep -x steel-front >/dev/null 2>&1; then
            echo "play_watchdog: ❌ 进程杀不掉（pid $(pgrep -x steel-front | tr '\n' ' ')）—— 需要人工介入" >&2
        else
            echo "play_watchdog: ✅ 第二次杀掉，已清空"
        fi
    else
        echo "play_watchdog: ✅ 进程已退出；Linux 侧指针捕获是代码级关闭的（RV3D_NO_CAPTURE），无需释放"
    fi
    rm -f "$BEAT"
done
