#!/bin/bash
# bench2560.sh — 2560x1600 + 64v64 全压力基准：游戏日志 + WSL硬件 + Windows GPU 三路采样
set -u
cd /home/jerry-huang/Rust/Rust_Vulkan_3D
SECS="${BENCH_SECS:-60}"
RES="${RES:-2560x1600}"
CFG="$HOME/.steel_front.cfg"
BAK=/tmp/steel_front.cfg.bak
GAME_LOG=/tmp/perf_${RES}.log
HW_LOG=/tmp/hw_${RES}.log
WIN_LOG=/tmp/win_gpu_${RES}.log
BOT_LOG=/tmp/bot_${RES}.log

pkill -x steel-front 2>/dev/null; pkill -f npc_bot.py 2>/dev/null; sleep 2
cp "$CFG" "$BAK"
sed -i "s/^resolution=.*/resolution=$RES/" "$CFG"
echo "cfg: $(grep '^resolution=' "$CFG")"

rm -f "$GAME_LOG" "$HW_LOG" "$WIN_LOG" "$BOT_LOG"
python3 /tmp/hw_mon.py 1 "$SECS" > "$HW_LOG" 2>&1 &
HW_PID=$!
bash /tmp/gpu_mon.sh "$SECS" 1 > "$WIN_LOG" 2>&1 &
WIN_PID=$!

setsid env -u WAYLAND_DISPLAY -u XDG_RUNTIME_DIR WINIT_UNIX_BACKEND=x11 \
    RUST_LOG=info RV3D_STRESS_AI=1 ./target/release/steel-front > "$GAME_LOG" 2>&1 < /dev/null &
GAME_PID=$!
sleep 4
python3 "${BOT_CMD:-/tmp/npc_bot.py}" "$GAME_LOG" "$SECS" > "$BOT_LOG" 2>&1
sleep 2
kill "$GAME_PID" "$HW_PID" "$WIN_PID" 2>/dev/null
sleep 1
pkill -x steel-front 2>/dev/null; sleep 1
cp "$BAK" "$CFG"
echo "cfg restored: $(grep '^resolution=' "$CFG")"
echo "=== BENCH-DONE game_lines=$(wc -l < "$GAME_LOG") hw_lines=$(wc -l < "$HW_LOG") win_lines=$(wc -l < "$WIN_LOG") ==="
