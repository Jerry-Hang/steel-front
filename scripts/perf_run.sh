#!/usr/bin/env bash
# perf_run.sh —— Linux 版性能尺子（对应 Windows 的 scripts/perf_run.ps1）
#
# 为什么是"另写"而不是移植 playtest_perf.py
# ----------------------------------------
# `scripts/playtest_perf.py` 是长期的性能尺子，但它是 **X11 专有**的：
# libX11/XTest 注入、XImage 截屏、pgrep/pkill、/proc/<pid>/status。
# 移植它不是"改路径"，是重写。本脚本改为复脚本所在仓库**已经实测过**的两样东西：
#   * 引擎每秒往 `logs/perf_<stamp>.log` 写一行 fps 与各渲染阶段微秒（见 src/perf_log.rs）；
#   * 启停就是 `pkill -x`（精确进程名）。
# ⇒ **不需要输入注入，也不需要截屏**，于是它天然不碰鼠标（Linux 侧的"鼠标安全协议"）。
#
# 压力模式默认开（RV3D_STRESS_AI=128 每方）：量 8 个 NPC 的教学关，
# 对真正要紧的帧预算说明不了任何事。
#
# 用法：
#   scripts/perf_run.sh                      # 60 秒
#   scripts/perf_run.sh -Secs 30
#   scripts/perf_run.sh -Stress 0            # 传统波次模式
#   scripts/perf_run.sh -NoShadow            # RV3D_NO_SHADOW=1（阴影成本 A/B）
#   scripts/perf_run.sh -Cam "0,0:0,0"       # RV3D_CAM 固定机位（可复现取景）
#   scripts/perf_run.sh -CullDiag            # RV3D_CULL_DIAG=1（剔除的 CPU 成本）
#   scripts/perf_run.sh -Extra "RV3D_NO_PROPS=1,RV3D_PROC_TEX=0"
#
# 退出码（教训 46：尺子必须能说"我没跑成"）
# ----------------------------------------
#   0 = 产出了稳态统计（t >= 3s 的窗口里至少 3 个样本）
#   1 = 没有 perf 日志，或样本行太少 ⇒ 没东西可报
#   2 = 统计打出来了，但 t >= 3s 的窗口不足 3 个样本 ⇒ 冷缓存行被算进来了，
#       **这些数不是稳态臂**。调用方（ab_pair / aa_probe）把任何非零都当失败的一轮。
#
# 与 Windows 侧的差异（**不要互相照抄**）
# -----------------------------------
#   * Windows 版在 finally 里调 `release_input.ps1` 解 ClipCursor；
#     Linux 侧**不需要**，因为本脚本让游戏在后台跑（永远拿不到焦点 ⇒
#     `capture_wanted` 恒为 false），且额外设了 `RV3D_NO_CAPTURE=1` 把
#     "不夺指针"变成**代码级保证**，而不是"碰巧没夺"。
#   * Windows 用 `Get-Process/Stop-Process`；这里是 `pkill -x`。
#     🔴 **只许 `-x`（精确进程名），绝不许 `-f`** —— 本仓库目录名就叫 `steel-front`，
#     用 `-f` 会命中**正在运行的脚本自己**（详见 SteelFront.sh 里的同一段教训）。
#   * Windows 版用 `-Filter perf_*.log` 找新日志；这里同样要**排除本脚本自己的
#     stdout 重定向**（`logs/perf_run.log`），否则会分析错文件、报"parsed 0 rows"
#     却把锅甩给这一轮跑动（ps1 里记录了同一个坑）。
#
# 噪声底（**别把小于它的差当结论**）
# --------------------------------
# 2026-09-15：同一二进制连跑两次（stress=128, 25s）中位 fps 69.7 / 71.8 —— 2.8% 的散布。
# 2026-09-26：那散布大部分是 fps 列本身的假象（它曾记「某一帧的 1/dt」，而 frame_us 记的是
# **另一帧**的耗时 ⇒ 同一份二进制能"差 48%"）。改用 perf_log.rs::window_fps 后稳定到 ~0.2%。
# ⇒ 先量当前二进制与参数的噪声底，**低于它的差一律写"没测到"**；单次一对仍然不是证据（教训 24/45）。
set -euo pipefail

SECS=60
STRESS=128
NO_SHADOW=0
CAM=""
RES=""
CULL_DIAG=0
EXTRA=""
ANALYZE_ONLY=""

while [ $# -gt 0 ]; do
    case "$1" in
        -Secs|--secs)     SECS="${2:?-Secs 后面要给秒数}"; shift 2 ;;
        -Stress|--stress) STRESS="${2:?-Stress 后面要给数量}"; shift 2 ;;
        -NoShadow|--no-shadow) NO_SHADOW=1; shift ;;
        -Cam|--cam)       CAM="${2:?-Cam 后面要给角度}"; shift 2 ;;
        -Res|--res)       RES="${2:?-Res 后面要给 WxH}"; shift 2 ;;
        -CullDiag|--cull-diag) CULL_DIAG=1; shift ;;
        -Extra|--extra)   EXTRA="${2:?-Extra 后面要给 K=V,...}"; shift 2 ;;
        -Log|--log)       ANALYZE_ONLY="${2:?-Log 后面要给日志路径}"; shift 2 ;;
        -h|--help)        sed -n '2,60p' "$0"; exit 0 ;;
        *) echo "perf_run: 不认识的参数 $1（-h 看用法）" >&2; exit 2 ;;
    esac
done
case "$SECS" in (*[!0-9]*|'') echo "perf_run: -Secs 必须是正整数，收到 '$SECS'" >&2; exit 2 ;; esac
case "$STRESS" in (*[!0-9]*|'') echo "perf_run: -Stress 必须是非负整数，收到 '$STRESS'" >&2; exit 2 ;; esac

repo="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo"                      # 🔴 引擎所有资产都是 CWD 相对路径
EXE="$repo/target/release/steel-front"
LOG="$repo/logs/perf_run.log"
LOGERR="$LOG.err"

[ -x "$EXE" ] || { echo "perf_run: 缺少 $EXE（先 cargo build --release）"; exit 1; }
mkdir -p "$repo/logs"

# 🔴 图形会话环境预检（2026-10-03 实测踩到）
# ----------------------------------------
# 引擎需要 `WAYLAND_DISPLAY`（或 `DISPLAY`）才能建事件循环。而**从 TTY/自动化 shell 里跑**时
# 这些变量不在（实测 `XDG_SESSION_TYPE=tty`，只有 `XDG_RUNTIME_DIR`），引擎会报
# "neither WAYLAND_DISPLAY nor WAYLAND_SOCKET nor DISPLAY is set"。
# 更糟的是修之前它**以退出码 0 退出**，于是本脚本只看到"游戏提前退出（code 0）"、
# 把它当成一次正常结束（引擎侧已改为非零退出，判据 `fatal_startup_paths_never_exit_zero`）。
# 这里再加一层：能自动补就补，补不上就 fail-closed 退 2 —— "没跑成"必须说出口。
if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${DISPLAY:-}" ]; then
    _rt="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
    if [ -S "$_rt/wayland-0" ]; then
        export WAYLAND_DISPLAY=wayland-0
        export DISPLAY="${DISPLAY:-:0}"
        echo "perf_run: 补上图形会话环境 WAYLAND_DISPLAY=$WAYLAND_DISPLAY DISPLAY=$DISPLAY（$_rt/wayland-0 存在）"
    else
        echo "perf_run: 没跑成 —— 既没有 WAYLAND_DISPLAY/DISPLAY，$_rt/wayland-0 也不存在。" >&2
        echo "          请在图形会话的终端里跑，或先 export WAYLAND_DISPLAY=wayland-0。" >&2
        exit 2
    fi
fi

# -Extra 解析：逗号/分号分隔的 KEY=VALUE。先解析完再启动，别边跑边建环境。
EXTRA_KEYS=()
if [ -n "$EXTRA" ]; then
    IFS=',;' read -r -a _pairs <<< "$EXTRA"
    for kv in "${_pairs[@]}"; do
        kv="${kv#"${kv%%[![:space:]]*}"}"; kv="${kv%"${kv##*[![:space:]]}"}"   # trim
        [ -z "$kv" ] && continue
        case "$kv" in
            *=*) ;;
            *) echo "perf_run: -Extra 项 '$kv' 格式不对（要 KEY=VALUE）" >&2; exit 1 ;;
        esac
        EXTRA_KEYS+=("${kv%%=*}")
        export "${kv?}"
    done
    echo "perf_run: extra env -> ${EXTRA_KEYS[*]}"
fi

# 记录跑之前已有哪些 perf 日志，跑完按**差集**认领本轮产物，不靠时间戳猜。
before="$(mktemp)"
ls -1 "$repo"/logs/perf_*.log 2>/dev/null > "$before" || true

cleanup() {
    pkill -x steel-front >/dev/null 2>&1 || true
    rm -f "$before"
}
trap cleanup EXIT INT TERM

# 只允许一个实例：两个会抢 GPU，两边的 fps 都变噪声。
pkill -x steel-front >/dev/null 2>&1 || true
sleep 2
rm -f "$LOG" "$LOGERR"

export RV3D_AUTOSTART=1
export RV3D_STRESS_AI="$STRESS"
# Linux 侧代码级的"不夺指针"保证（本脚本不注入输入，游戏也拿不到焦点，这里是双保险）。
export RV3D_NO_CAPTURE=1
[ "$NO_SHADOW" = 1 ] && export RV3D_NO_SHADOW=1
[ -n "$CAM" ] && export RV3D_CAM="$CAM"
[ -n "$RES" ] && export RV3D_RES="$RES"
[ "$CULL_DIAG" = 1 ] && export RV3D_CULL_DIAG=1
# ⚠️ **不设 RV3D_PRESENT_MODE**：与 Windows 版一致，量的是引擎默认（IMMEDIATE）。

echo "perf_run: stress=$STRESS secs=$SECS$([ "$NO_SHADOW" = 1 ] && echo ' noshadow')$([ -n "$CAM" ] && echo " cam=$CAM")$([ "$CULL_DIAG" = 1 ] && echo ' culldiag')"

if [ -n "$ANALYZE_ONLY" ]; then
    # -Log：只复核已有日志，**不启动游戏**。用途有二：
    #   ① 旧日志离线重判（Windows 侧 smoke 也有同样的"同一个文件可以被离线复核"约定）；
    #   ② 让"稳态窗口太小 ⇒ exit 2"这条分支**可被确定性验证** ——
    #      引擎每秒一行 ⇒ NROWS=N 必然推出 STEADY_N=N-2，靠真实时长几乎落不进那个角落。
    [ -r "$ANALYZE_ONLY" ] || { echo "perf_run: 读不到 $ANALYZE_ONLY" >&2; exit 1; }
    PERF="$ANALYZE_ONLY"
    echo "perf_run: 只分析 $PERF（不启动游戏）"
else
"$EXE" >"$LOG" 2>"$LOGERR" &
PID=$!
echo "perf_run: pid $PID, 采样 ${SECS}s"
i=0
while [ "$i" -lt "$SECS" ]; do
    sleep 1
    if ! kill -0 "$PID" 2>/dev/null; then
        wait "$PID" 2>/dev/null && rc=0 || rc=$?
        echo "perf_run: 游戏提前退出（code $rc）—— 继续看日志"
        break
    fi
    i=$((i + 1))
done
pkill -x steel-front >/dev/null 2>&1 || true
sleep 1
fi

# 认领本轮产生的 perf 日志（排除本脚本自己的 logs/perf_run.log）。
if [ -z "$ANALYZE_ONLY" ]; then
PERF=""
while IFS= read -r f; do
    [ -z "$f" ] && continue
    case "$(basename "$f")" in perf_run.log) continue ;; esac
    grep -qxF "$f" "$before" 2>/dev/null || PERF="$f"
done < <(ls -1t "$repo"/logs/perf_*.log 2>/dev/null || true)

if [ -z "$PERF" ]; then
    echo "perf_run: FAIL —— 没有产生新的 logs/perf_*.log。看 logs/perf_run.log.err" >&2
    exit 1
fi
fi
echo "perf_run: perf log = $(basename "$PERF")"

# 列（唯一真源 = src/perf_log.rs::PERF_LOG_COLUMNS，有单测钉住下标；
# 改那边就必须同步改这里）：
#   0=t 1=fps 2=dt_us 3=frame_us 4=cull 5=terrain 6=wait 7=acquire 8=record 9=submit 10=present 11=near
ROWS="$(mktemp)"
trap 'cleanup; rm -f "$ROWS"' EXIT INT TERM
awk -F'\t' 'NF>=12 && $1+0==$1 { printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n", $1,$2,$3,$4,$5,$6,$11,$12 }' \
    "$PERF" > "$ROWS"

NROWS=$(wc -l < "$ROWS")
if [ "$NROWS" -lt 5 ]; then
    echo "perf_run: FAIL —— 只解析出 $NROWS 行样本；跑得太短，或引擎根本没渲染" >&2
    exit 1
fi

# 稳态窗口 = t >= 3s。首样本不代表稳态：SPIR-V 重生成会让驱动在最初几帧 JIT 管线，
# 那里的 fps 是冷缓存假象（AGENTS.md 记的"首帧窗口"陷阱）。
# 🔴 这个回退**必须打印出来**：旧版 PS 脚本静默回退，于是"跑到一半就死的轮次"看起来
# 与正常测量一模一样，被 ab_pair 当成一条合格的臂用了。
STEADY_N=$(awk -F'\t' '$1>=3.0' "$ROWS" | wc -l)
MODE=steady
if [ "$STEADY_N" -lt 3 ]; then
    MODE=all
fi

# $1=列号 $2=名称 $3=单位
stat_line() {
    awk -F'\t' -v col="$1" -v name="$2" -v unit="$3" -v mode="$MODE" '
    { t[NR]=$1; v[NR]=$col }
    END {
        m=0
        for (i=1;i<=NR;i++) if (mode=="all" || t[i]>=3.0) { m++; s[m]=v[i] }
        if (m==0) { printf "  %-9s (无样本)\n", name; exit }
        for (i=2;i<=m;i++) { key=s[i]; j=i-1; while (j>0 && s[j]>key) { s[j+1]=s[j]; j-- } s[j+1]=key }
        sum=0; for (i=1;i<=m;i++) sum+=s[i]
        mean=sum/m
        med=(m%2==1)? s[int((m-1)/2)+1] : (s[m/2]+s[m/2+1])/2
        pi=int(0.95*m); if (pi<1) pi=1; if (pi>m) pi=m
        printf "  %-9s mean %8.2f%-2s  median %8.2f%-2s  p95 %8.2f%-2s  min %.2f  max %.2f\n", \
               name, mean, unit, med, unit, s[pi], unit, s[1], s[m]
    }' "$ROWS"
}

echo
LAST_T=$(tail -1 "$ROWS" | cut -f1)
if [ "$MODE" = steady ]; then
    printf '==== perf_run 结果: %s 个样本 / %s 秒 (稳态 = t >= 3s, n=%s) ====\n' "$NROWS" "$LAST_T" "$STEADY_N"
else
    printf '==== perf_run 结果: %s 个样本 / %s 秒 ====\n' "$NROWS" "$LAST_T"
    printf '  ⚠️ 警告: t >= 3s 的样本只有 %s 个（< 3）—— 下面的统计**包含冷缓存行**\n' "$STEADY_N"
fi
stat_line 2 fps ""
stat_line 3 dt_us us
stat_line 4 frame_us us
stat_line 5 cull us
stat_line 6 terrain us
stat_line 7 present us
printf '  npc boxes  near=%s\n' "$(tail -1 "$ROWS" | cut -f8)"
printf '  full log   %s\n' "$PERF"
if [ "$MODE" = steady ]; then
    printf '  (首样本 t=%ss fps=%s 已作为冷缓存排除)\n' \
        "$(head -1 "$ROWS" | cut -f1)" "$(head -1 "$ROWS" | cut -f2)"
fi

# 把机器交还，并且**明说**做了什么，而不是假定它成功了。
if [ -n "$ANALYZE_ONLY" ]; then
    :   # -Log 模式没启动过游戏，别打"指针未被捕获"那种没发生过的保证
elif pgrep -x steel-front >/dev/null 2>&1; then
    echo "  残留进程：仍在运行（已再杀一次）" >&2
    pkill -x steel-front >/dev/null 2>&1 || true
else
    echo "  鼠标：本脚本全程未注入输入、未夺焦点；进程已退出，指针未被捕获"
fi

if [ "$MODE" = all ]; then
    echo "perf_run: exit 2 —— 稳态窗口太小，这些数**不能**当作 A/B 的一条臂" >&2
    exit 2
fi
exit 0
