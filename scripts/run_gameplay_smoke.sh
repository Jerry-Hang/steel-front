#!/bin/bash
cd /home/jerry-huang/Rust/Rust_Vulkan_3D
LOG=/tmp/gameplay_smoke.log
DIR="$(cd "$(dirname "$0")" && pwd)"
pkill -x steel-front 2>/dev/null; sleep 2          # 清场：无残留进程/窗口
python3 "$DIR/release_keys.py" 2>&1                # 清 X server 卡键/卡按钮
rm -f "$LOG"
setsid env -u WAYLAND_DISPLAY -u XDG_RUNTIME_DIR WINIT_UNIX_BACKEND=x11 ./target/release/steel-front > "$LOG" 2>&1 < /dev/null &
sleep 3
python3 "$DIR/gameplay_smoke.py" "$LOG" 2>&1
RC=$?
pkill -x steel-front; sleep 2
echo "=== log tail (last 25 lines) ==="
tail -25 "$LOG"
exit $RC
