#!/usr/bin/env bash
# resize_probe.sh —— 交换链重建路径探针（对应 Windows 的 scripts/run_resize_probe.ps1）
#
# 为什么需要它（2026-10-03）
# -----------------------
# 铁律 B 说「改 pipeline / swapchain / 同步 / 描述符前先开 RV3D_VALIDATION=1 跑一轮」。
# 而**交换链重建路径**（destroy + 重建 swapchain / framebuffers / 信号量 / 命令缓冲）
# 恰恰是"改过、但从没在验证层下真的缩放过"的那一段 —— 冒烟跑的是固定尺寸，
# 一次 `WindowEvent::Resized` 都不会发生。
#
# Linux 侧这条尤其要紧：`b3874af` 修的正是「Wayland 下 `currentExtent` 恒为
# `UINT32_MAX` ⇒ 交换链永远停在 1280x720」（实测本机
# `swapchain diag: current_extent=4294967295x4294967295`）。那个修复只有**真的缩放**
# 才能证明它还成立。
#
# 两条从 Windows 版照抄的硬教训
# ---------------------------
#   1. **必须证明"缩放路径真的走过"**：本探针存在的唯一目的就是驱动交换链重建，
#      所以「一次窗口大小变化都没有」时 `VUID=0` 只意味着**什么都没发生**
#      （教训 27 / §21.48：扫描面为 0 不算通过）。判据里有这一条，且单独一个退出码。
#   2. `-PT` 时必须证明 PT 真的启用（`PT-RESIDENT` ≥1）—— 否则「VUID: 0」同样可能只是
#      "我想测的东西压根没跑"。这条正是那个回归的复现器：PT 上屏 blit 曾把目标范围写死
#      2560x1600，只有在默认窗口尺寸下才没越界（判据 `blit_regions_never_hardcode_pixel_extents`）。
#
# 与 Windows 侧的差异（**不要互相照抄**）
# -----------------------------------
#   * **缩放靠 KWin 脚本**，不是 `SetWindowPos`：Wayland 下客户端不能改自己的位置/尺寸，
#     由合成器说了算。做法是 `qdbus6 org.kde.KWin /Scripting loadScript <js>` + `start`，
#     脚本里设 `w.frameGeometry`。**全程不抢焦点、不碰指针**，天然符合鼠标安全协议。
#   * **没有 RTSS / GamePP 那两个隐式层**，所以不设 DISABLE_*_LAYER。
#   * **没有 F12 截图步骤**：Linux 上截图的唯一触发是 F12 按键，而在 Wayland 上注入按键
#     需要抢焦点/用 ydotool 之类，正好违反那条协议。画面取证另有记账（见文档 §10 末尾）。
#   * ⚠️ **KWin 的 frameGeometry 是逻辑坐标**（本机 `scale_factor=1.25`）：
#     脚本里设 1280x720，引擎日志里会看到 **1600x900**。别把它当成 bug ——
#     正好相反，这是"逻辑 → 物理"换算正确的证据。
#
# 用法：
#   scripts/resize_probe.sh
#   scripts/resize_probe.sh -Tag pt_resize -PT
#   scripts/resize_probe.sh -Sizes "1280x720,1024x768" -AfterSecs 5
#
# 退出码（三态，教训 46）
#   0 = ALL-OK（缩放路径走过 + VUID/panic/device-lost 全 0）
#   1 = 跑了但有问题（有 VUID / panic / device lost）
#   2 = **没跑成**（没有日志 / 一次窗口大小变化都没有 ⇒ 路径根本没被走到）
set -euo pipefail

TAG="resize_probe"
WARMUP=15
AFTER=8
SIZES="1280x720,1600x900,1024x768,2560x1600,1280x800"
PT=0
NOSHOT=0

while [ $# -gt 0 ]; do
    case "$1" in
        -Tag|--tag)             TAG="${2:?-Tag 后面要给名字}"; shift 2 ;;
        -WarmupSec|--warmup)    WARMUP="${2:?-WarmupSec 后面要给秒数}"; shift 2 ;;
        -AfterSecs|--after)     AFTER="${2:?-AfterSecs 后面要给秒数}"; shift 2 ;;
        -Sizes|--sizes)         SIZES="${2:?-Sizes 后面要给 WxH,...}"; shift 2 ;;
        -PT|--pt)               PT=1; shift ;;
        -NoShot|--no-shot)      NOSHOT=1; shift ;;
        -h|--help)              sed -n '2,50p' "$0"; exit 0 ;;
        *) echo "resize_probe: 不认识的参数 $1（-h 看用法）" >&2; exit 2 ;;
    esac
done
for n in "$WARMUP" "$AFTER"; do
    case "$n" in (*[!0-9]*|'') echo "resize_probe: 秒数必须是正数，收到 '$n'" >&2; exit 2 ;; esac
done

repo="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo"
EXE="$repo/target/release/steel-front"
LOG="$repo/logs/$TAG.log"
LOGERR="$LOG.err"

[ -x "$EXE" ] || { echo "resize_probe: 缺少 $EXE（先 cargo build --release）"; exit 2; }
command -v qdbus6 >/dev/null 2>&1 || {
    echo "resize_probe: 没跑成 —— 找不到 qdbus6（缩放要靠 KWin 脚本，Wayland 下客户端改不了自己的尺寸）" >&2
    exit 2
}

# 图形会话预检（与 smoke_linux.sh / perf_run.sh 同一段理由）
if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${DISPLAY:-}" ]; then
    _rt="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
    if [ -S "$_rt/wayland-0" ]; then
        export WAYLAND_DISPLAY=wayland-0; export DISPLAY="${DISPLAY:-:0}"
        echo "resize_probe: 补上图形会话环境 WAYLAND_DISPLAY=$WAYLAND_DISPLAY DISPLAY=$DISPLAY"
    else
        echo "resize_probe: 没跑成 —— 既没有 WAYLAND_DISPLAY/DISPLAY，$_rt/wayland-0 也不存在。" >&2
        exit 2
    fi
fi

# KWin 脚本：**每个尺寸单独 load 一次**，不用 setTimeout 链。
#
# 🔴 2026-10-03 实测：**KWin 脚本环境里的 `setTimeout` 回调不会触发** ——
# 一个只做 `print` + `setTimeout(...,500)` 的最小脚本，print 打了、回调没打。
# 第一版探针就是靠 setTimeout 串起全部步骤的，结果**一次都没缩放**，
# 而当时的判据太弱（把启动时那次 `窗口大小变化` 当成了 "路径走过"），
# 于是报出了 ALL-OK —— 正是本探针头部警告的那个反面典型。
# 现在改成：bash 侧逐个尺寸 load+start（我早先的手工实验就是这个形态，可靠）。
# 尺寸列表解析 + 两个临时文件（JS 每步重写、STEPS 存解析后的尺寸）。
# ⚠️ 这两行曾被一次范围过宽的替换误删，症状是 `STEPS: 未绑定的变量`（set -u 直接退出）。
JS="$(mktemp --suffix=.js)"
STEPS="$(mktemp)"
python3 - "$SIZES" "$STEPS" <<'SZBUF'
import sys
sizes = [x.strip() for x in sys.argv[1].split(",") if x.strip()]
bad = [x for x in sizes if "x" not in x.lower()]
if bad or not sizes:
    print("resize_probe: -Sizes 里有非法项 " + repr(bad or "(空)"), file=sys.stderr)
    sys.exit(2)
open(sys.argv[2], "w").write("\n".join(sizes) + "\n")   # 末尾换行：否则 mapfile 少数一行
SZBUF

mk_js() {  # $1=宽 $2=高 $3=输出文件
    cat > "$3" <<JSEOF
// 由 scripts/resize_probe.sh 生成：把 Steel Front 窗口改成 $1x$2（KWin 逻辑坐标）。
// Wayland 下客户端不能改自己的尺寸，只能由合成器改。
function findWin() {
    var ws = workspace.windowList ? workspace.windowList() : workspace.clientList();
    for (var i = 0; i < ws.length; i++) {
        var c = String(ws[i].resourceClass || "");
        var t = String(ws[i].caption || "");
        if (c.indexOf("steel-front") >= 0 || t.indexOf("Steel Front") >= 0) return ws[i];
    }
    return null;
}
var w = findWin();
if (!w) { print("RSPROBE no-window"); }
else {
    var g = w.frameGeometry;
    w.frameGeometry = { x: g.x, y: g.y, width: $1, height: $2 };
    var a2 = w.frameGeometry;
    print("RSPROBE want=$1x$2 got=" + a2.width + "x" + a2.height);
}
JSEOF
}

pkill -x steel-front >/dev/null 2>&1 || true
sleep 1
rm -f "$LOG" "$LOGERR"
mkdir -p "$repo/logs"

export RV3D_AUTOSTART=1
export RV3D_STRESS_AI=1
export RV3D_GPU=dgpu
export RV3D_PRESENT_MODE=mailbox
export RV3D_BG_FPS=0
export RV3D_NO_CAPTURE=1          # 鼠标安全：本探针不注入输入、不夺焦点，双保险
export RV3D_VALIDATION=1
# 本机**没有** RTSS / GamePP 那两个隐式层（Windows 专有），所以不像 ps1 那样设 DISABLE_*_LAYER。
# 截图：**用引擎自带的 RV3D_SHOT_AT，不注入 F12 按键**。
# Windows 侧的同一探针走 PostMessage(WM_KEYDOWN)，Linux 上要等价就得抢焦点/用 ydotool，
# 正好违反铁律 C 的鼠标安全协议 ⇒ 走这个不需要输入的触发（2026-10-03 为此新加）。
# 时刻取"全部缩放做完之后"（预热 + 每步 2s + 1s 余量），这样截到的是缩放后的画面。
SHOT_AT=""
if [ "$NOSHOT" = 0 ]; then
    SHOT_AT=$(( WARMUP + $(grep -c . "$STEPS") * 2 + 1 ))
    export RV3D_SHOT_AT="$SHOT_AT"
    echo "resize_probe: 将在第 ${SHOT_AT}s 自动截一张（RV3D_SHOT_AT，不需要输入注入）"
fi
if [ "$PT" = 1 ]; then
    export RV3D_PT_LIVE=1 RV3D_PT_SIZE=512 RV3D_PT_SPP=16
    echo "resize_probe: PT 实时路径已开（RV3D_PT_LIVE=1 RV3D_PT_SIZE=512 RV3D_PT_SPP=16）"
fi

"$EXE" >"$LOG" 2>"$LOGERR" &
PID=$!
echo "resize_probe: pid $PID（验证层开）"
TAIL=""
cleanup() { pkill -x steel-front >/dev/null 2>&1 || true; rm -f "$JS" "$STEPS" ${TAIL:+"$TAIL"}; }
trap cleanup EXIT INT TERM

sleep "$WARMUP"
if ! kill -0 "$PID" 2>/dev/null; then
    echo "resize_probe: 引擎在预热期就退出了 —— 看 $LOGERR" >&2
fi

mapfile -t SZ < "$STEPS"
N="${#SZ[@]}"
# 🔴 先记下"预热结束时日志写到第几行"：启动本身会产生一次 `窗口大小变化`，
# 不区分开的话判据会被它满足 ⇒ 报 ALL-OK 而实际什么都没缩放（第一版就这么骗过了自己）。
MARK=$(wc -l < "$LOGERR")
echo "resize_probe: 开始缩放 $N 步（${SZ[*]}；KWin 逻辑坐标，实际物理尺寸见引擎日志）"
for spec in "${SZ[@]}"; do
    w="${spec%%x*}"; h="${spec##*x}"
    mk_js "$w" "$h" "$JS"
    qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "$JS" >/dev/null 2>&1 || true
    qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start >/dev/null 2>&1 || true
    sleep 2
done

sleep "$AFTER"
pkill -x steel-front >/dev/null 2>&1 || true
sleep 1
cleanup
trap - EXIT INT TERM

# ---- 判据：全部从引擎日志里数 ------------------------------------------------
if [ ! -s "$LOGERR" ]; then
    echo "resize_probe: 没跑成 —— 引擎日志 $LOGERR 不存在或是空的" >&2
    exit 2
fi
count() { grep -c -- "$1" "$LOGERR" 2>/dev/null || true; }

TAIL="$(mktemp)"
tail -n +"$((MARK + 1))" "$LOGERR" > "$TAIL"
count_t() { grep -c -- "$1" "$TAIL" 2>/dev/null || true; }

VUID=$(count 'VUID')
PANIC=$(count 'panicked\|panic')
LOST=$(count 'has been lost')
RESIZE=$(count_t '窗口大小变化')       # ⚠️ 只数探测期间的
MISMATCH=$(count 'size mismatch')
PTRES=$(count 'PT-RESIDENT')
SHOTS=$(count '截图已保存')

echo
echo "=== resize probe 结果（$TAG）==="
echo "  窗口大小变化 : $RESIZE 次（**探测期间**的；必须 >=1，启动那次已排除）"
echo "  尺寸不符重建 : $MISMATCH 次"
echo "  VUID         : $VUID"
echo "  PT-RESIDENT  : $PTRES$([ "$PT" = 1 ] && echo '（-PT 时必须 >=1，证明 PT 真的跑起来了）' || echo '（未开 PT）')"
echo "  截图         : $SHOTS 张$([ "$NOSHOT" = 1 ] && echo '（-NoShot，不截图）' || echo "（RV3D_SHOT_AT=$SHOT_AT）")"
echo "  设备丢失     : $LOST ;  panic: $PANIC"
echo "  引擎日志     : $LOGERR"
echo "--- 交换链实际用过的尺寸（去重，证明它真的跟着窗口走了）---"
grep -oE '交换链初始化完成: [0-9]+x[0-9]+' "$LOGERR" | sed 's/.*: //' | sort -u | sed 's/^/  /' || true
echo "--- swapchain diag 首行（current_extent 为 UINT32_MAX = 那条 Wayland 陷阱）---"
grep -oE 'current_extent=[0-9x]+ final=[0-9x]+' "$LOGERR" | head -1 | sed 's/^/  /' || true

if [ "$RESIZE" -lt 1 ]; then
    echo "resize_probe: exit 2 —— **没跑成**：一次窗口大小变化都没有，VUID=0 只说明什么都没发生" >&2
    exit 2
fi
if [ "$PT" = 1 ] && [ "$PTRES" -lt 1 ]; then
    echo "resize_probe: exit 2 —— 没跑成：-PT 给了但 PT 从未驻留（想测的东西没跑起来）" >&2
    exit 2
fi
if [ "$NOSHOT" = 0 ] && [ "$SHOTS" -lt 1 ]; then
    echo "resize_probe: exit 2 —— 没跑成：要了截图但一张都没落盘（取证这一步没发生）" >&2
    exit 2
fi
if [ "$VUID" -eq 0 ] && [ "$LOST" -eq 0 ] && [ "$PANIC" -eq 0 ]; then
    echo "RESULT: ALL-OK"
    exit 0
fi
echo "RESULT: CHECK（VUID=$VUID lost=$LOST panic=$PANIC）" >&2
exit 1
