#!/usr/bin/env bash
# net_pair.sh —— 联网「双进程真机验证」（对应未结案 #13 里那句「仍未做」）
#
# 为什么需要它（2026-10-07）
# ------------------------
# `net.rs` / `game.rs` 里的联机验证**全部是单测**（UDP 环回、进程内 server+client，
# 见 `net.rs` 的 `udp_loopback_handshake_input_snapshot_roundtrip` 与 `game.rs` 的
# `net_loopback_demo_join_roundtrip`）。那些证的是**协议编解码与状态机**，
# 而 `main.rs:4327` 的注释自己写着：
#
#     未做：NAT 双进程真机验证、输入预测/回滚
#
# ⇒ 从来没有人真的起过**两个独立进程**、让它们走真实的 UDP 握手并交换快照。
#   单测能覆盖的最远到「同一个进程里两个 Socket 互发」，而真机路径多出的是：
#   真实的 `Server::bind` / `Client::connect`、主循环里的 tick 与广播节奏、
#   两端各自的 winit/Vulkan 生命周期、以及**两端都在跑渲染**时的时序竞争。
#
# 判据（三态，教训 46）
# -------------------
#   0 = 真连上了：服务端起来了 + 客户端连上了 + **服务端打出了「远端玩家 #N 加入」**
#   1 = 跑了但没连上（服务端起来了，但没等到握手）
#   2 = **没跑成**：exe 缺失 / 端口占用 / 两边的日志根本没产出（不许当通过）
#
# 🔴 判据只用**真路径**的标记，不用 `net: remote_players=` ——
#    那一行在 `game.rs` 的 `demo` 块里，是**进程内环回演示**专用（`init_network_demo`），
#    `RV3D_NET=client` 这条真路径**不会**打它。拿它当判据会在真机上一次都命中不了。
#
# 鼠标安全（铁律 C）
# ----------------
# 两个实例都会建窗口。`RV3D_NO_CAPTURE=1` 是**代码级**保证「不抓光标」，
# 所以全程不需要抢焦点、不需要注入按键 —— 这条协议在这里同样成立。
#
# 用法：
#   scripts/net_pair.sh
#   scripts/net_pair.sh -Secs 30 -Tag mynet
#   scripts/net_pair.sh -Port 27099 -Validation 1
#
# 退出码：见上（0/1/2）
set -uo pipefail

EXE=target/release/steel-front
PORT=27015
SECS=20
TAG=netpair
VALIDATION=0

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help)   sed -n '2,45p' "$0"; exit 0 ;;
        -Port)       PORT="$2"; shift 2 ;;
        -Secs)       SECS="$2"; shift 2 ;;
        -Tag)        TAG="$2"; shift 2 ;;
        -Validation) VALIDATION="$2"; shift 2 ;;
        *) echo "net_pair: 不认识的参数 $1（-h 看用法）" >&2; exit 2 ;;
    esac
done
case "$SECS" in (*[!0-9]*|'') echo "net_pair: -Secs 必须是正数，收到 '$SECS'" >&2; exit 2 ;; esac
case "$PORT" in (*[!0-9]*|'') echo "net_pair: -Port 必须是数字，收到 '$PORT'" >&2; exit 2 ;; esac

[ -x "$EXE" ] || { echo "net_pair: 缺少 $EXE（先 cargo build --release）" >&2; exit 2; }

# ---- 图形会话（agent shell 不继承这两个）----------------------------------
if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -S /run/user/1000/wayland-0 ]; then
    export WAYLAND_DISPLAY=wayland-0
fi
export DISPLAY="${DISPLAY:-:0}"

# ---- 端口占用检查（占用了就别硬跑，直接 exit 2 —— 那是"没跑成"）------------
if ss -lun 2>/dev/null | grep -q ":${PORT} "; then
    echo "net_pair: 端口 $PORT 已被占用 —— 没跑成（换个 -Port）" >&2
    exit 2
fi

SLOG="logs/${TAG}_server.log.err"
CLOG="logs/${TAG}_client.log.err"
SPID=""
CPID=""

cleanup() {
    # 只杀自己起的两个 PID —— 绝不用 pkill -f（仓库目录名就是 steel-front，会误伤）
    [ -n "$CPID" ] && kill "$CPID" 2>/dev/null
    [ -n "$SPID" ] && kill "$SPID" 2>/dev/null
    sleep 1
    [ -n "$CPID" ] && kill -9 "$CPID" 2>/dev/null
    [ -n "$SPID" ] && kill -9 "$SPID" 2>/dev/null
    return 0
}
trap cleanup EXIT INT TERM

rm -f "$SLOG" "$CLOG"

# 两端共用的环境：照抄 scripts/smoke_linux.sh 那条**已验证**的路
COMMON=(env
    RV3D_NO_CAPTURE=1          # 鼠标安全，代码级保证（铁律 C）
    RV3D_AUTOSTART=1           # 直接进 Playing —— 不进玩法两端都不发输入/快照
    RV3D_BG_FPS=0              # 两个窗口必然互相失焦，不限速
    RV3D_PRESENT_MODE=mailbox  # 独显长跑用 mailbox（IMMEDIATE 有 TDR 前科）
    RV3D_VALIDATION="$VALIDATION"
)

echo "net_pair: 端口 $PORT，两端各跑 ${SECS}s（验证层=$VALIDATION）"
echo "net_pair: 日志 logs/${TAG}_server.log.err / logs/${TAG}_client.log.err"

# ---- 起服务端 --------------------------------------------------------------
"${COMMON[@]}" RV3D_NET=server RV3D_NET_ADDR="127.0.0.1:${PORT}" \
    "$EXE" >"/dev/null" 2>"$SLOG" &
SPID=$!
echo "net_pair: 服务端 pid=$SPID"

# 等服务端真的开始监听（判据：日志里出现监听行，而不是"睡够 N 秒"）
LISTENED=0
for _ in $(seq 1 60); do
    if grep -qE 'net: (server mode active|服务器模式，监听)' "$SLOG" 2>/dev/null; then
        LISTENED=1; break
    fi
    kill -0 "$SPID" 2>/dev/null || break
    sleep 1
done

if [ "$LISTENED" != 1 ]; then
    echo "net_pair: exit 2 —— **没跑成**：服务端没有进入监听（日志里没有监听行）" >&2
    exit 2
fi
echo "net_pair: 服务端已监听，等 3s 再上客户端"
sleep 3

# ---- 起客户端 --------------------------------------------------------------
"${COMMON[@]}" RV3D_NET=client RV3D_NET_ADDR="127.0.0.1:${PORT}" \
    "$EXE" >"/dev/null" 2>"$CLOG" &
CPID=$!
echo "net_pair: 客户端 pid=$CPID"

sleep "$SECS"
cleanup
trap - EXIT INT TERM

# ---- 判据：只数真路径的标记 ------------------------------------------------
if [ ! -s "$SLOG" ] && [ ! -s "$CLOG" ]; then
    echo "net_pair: exit 2 —— **没跑成**：两端都没有产出日志" >&2
    exit 2
fi
cnt() { grep -cE -- "$2" "$1" 2>/dev/null || true; }

S_ACTIVE=$(cnt "$SLOG" 'net: (server mode active|服务器模式，监听)')
C_ACTIVE=$(cnt "$CLOG" 'net: (client mode active|客户端模式，连接)')
JOIN=$(cnt "$SLOG" 'net: server 远端玩家 #[0-9]+ 加入')
C_FAIL=$(cnt "$CLOG" 'net: 客户端连接 .* 失败')

echo
echo "=== net pair 结果（$TAG）==="
echo "  服务端监听   : $S_ACTIVE 行（必须 >=1）"
echo "  客户端连接   : $C_ACTIVE 行（必须 >=1）"
echo "  服务端见加入 : $JOIN 行（**决定性证据**：只有握完手才会打）"
echo "  客户端连接失败: $C_FAIL 行"
echo "  引擎日志     : $SLOG / $CLOG"

if [ "$S_ACTIVE" -lt 1 ]; then
    echo "net_pair: exit 2 —— **没跑成**：服务端日志里没有监听行，判据不成立" >&2
    exit 2
fi
if [ "$JOIN" -ge 1 ]; then
    echo
    echo "--- 服务端记录的加入（原样）---"
    grep -E 'net: server 远端玩家 #[0-9]+ 加入' "$SLOG" | head -5 | sed 's/^/  /'
    echo "RESULT: ALL-OK（双进程真机握手成功）"
    exit 0
fi
echo "net_pair: exit 1 —— 跑了但没连上（服务端在听，但没等到握手）" >&2
exit 1
