#!/usr/bin/env bash
# ============================================================================
#  steel_front_launch.sh —— 桌面图标的**入口**（双击就走这里）
# ============================================================================
#  为什么不是直接 Exec 指到 SteelFront.sh：
#    `.desktop` 是 `Terminal=false`，双击之后**没有任何窗口** —— 而首次启动要先编译
#    （几分钟），构建失败的错误也只打到 stderr ⇒ 用户看到的是"双击了，什么也没发生"。
#    这里做两件事：
#      1) 有终端模拟器就**在终端里**跑（编译进度、报错都看得见）；
#      2) 没有终端就直接跑，并用 `notify-send` 报一句失败原因（不静默）。
#
#  两个开关（给脚本化验证用，正常双击用不到）：
#    STEEL_LAUNCH_IN_TERMINAL=0  强制不开终端（无头验证必须这样，别弹窗）
#    STEEL_LAUNCH_DRYRUN=1       只打印将要执行的命令，不真的启动
#
#  退出码按仓库三态约定：0=成功 / 1=失败 / 2=没跑成（缺前置条件，教训 46）
# ============================================================================
set -uo pipefail

src="${BASH_SOURCE[0]}"
while [ -L "$src" ]; do
    dir="$(cd -P -- "$(dirname -- "$src")" >/dev/null 2>&1 && pwd)"
    src="$(readlink -- "$src")"
    [[ $src != /* ]] && src="$dir/$src"
done
REPO="$(cd -P -- "$(dirname -- "$src")/.." >/dev/null 2>&1 && pwd)"
LAUNCHER="$REPO/SteelFront.sh"

if [ ! -x "$LAUNCHER" ]; then
    if [ ! -f "$LAUNCHER" ]; then
        echo "steel_front_launch: 找不到启动器 $LAUNCHER" >&2
        command -v notify-send >/dev/null && notify-send -a "钢铁前线" "启动失败" "找不到 $LAUNCHER"
        exit 2
    fi
    chmod +x "$LAUNCHER" || true
fi

# 终端模拟器：优先 KDE（本机就是 KDE），再退化到通用名
pick_terminal() {
    for t in konsole gnome-terminal xfce4-terminal alacritty kitty x-terminal-emulator xterm; do
        if command -v "$t" >/dev/null 2>&1; then
            echo "$t"
            return 0
        fi
    done
    return 1
}

want_terminal="${STEEL_LAUNCH_IN_TERMINAL:-1}"
terminal=""
if [ "$want_terminal" = "1" ]; then
    terminal="$(pick_terminal || true)"
fi

# 终端各自的"执行命令"写法不一样，别混用
term_exec() {
    local t="$1"; shift
    case "$t" in
        konsole)               exec konsole --hold -e "$@" ;;
        gnome-terminal)        exec gnome-terminal -- "$@" ;;
        xfce4-terminal)        exec xfce4-terminal -x "$@" ;;
        alacritty|kitty|xterm) exec "$t" -e "$@" ;;
        x-terminal-emulator)   exec x-terminal-emulator -e "$@" ;;
        *)                     exec "$@" ;;
    esac
}

if [ "${STEEL_LAUNCH_DRYRUN:-0}" = "1" ]; then
    if [ -n "$terminal" ]; then
        echo "将会执行: $terminal <- $LAUNCHER play $*"
    else
        echo "将会执行: $LAUNCHER play $*（无终端）"
    fi
    exit 0
fi

if [ -n "$terminal" ]; then
    # 终端里跑：`--hold` 让窗口在游戏退出后保留，报错能被看见
    term_exec "$terminal" "$LAUNCHER" play "$@"
else
    # 没有终端：直接跑；失败要能看见（否则就是"双击没反应"）
    "$LAUNCHER" play "$@"
    rc=$?
    if [ "$rc" -ne 0 ]; then
        command -v notify-send >/dev/null && \
            notify-send -a "钢铁前线" "启动失败（退出码 $rc）" "在终端里跑 ./SteelFront.sh play 看完整报错"
        command -v zenity >/dev/null && \
            zenity --error --text="钢铁前线启动失败（退出码 $rc）\n请在仓库目录运行 ./SteelFront.sh play 查看完整报错" 2>/dev/null || true
    fi
    exit "$rc"
fi
