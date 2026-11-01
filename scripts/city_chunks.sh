#!/usr/bin/env bash
# ============================================================================
#  city_chunks.sh —— 一键重出「整座程序化城的可见几何」
# ============================================================================
#  链路（2026-11-01 建立，见 docs/PROGRESS.md 续125/续126）：
#    引擎导出布局  →  Blender 按 40m 格重建 + 合并  →  assets/city_chunks/*.glb
#
#  为什么要这条链路：全城是 1230 个程序化构件（每帧约 1.8 万个实例），
#  重建合并后只有 12 个 chunk / 28.7k 三角 —— 而且布局仍由 city.rs 唯一决定
#  （碰撞/寻路/出生点/40+ 条布局判据一行不动）。
#
#  用法：
#    scripts/city_chunks.sh              # 重新导出并重建（会覆盖 assets/city_chunks/）
#    scripts/city_chunks.sh --dry-run    # 只打印将执行什么
#
#  退出码：0=成功 / 1=失败 / 2=没跑成（缺前置条件）
# ============================================================================
set -uo pipefail

src="${BASH_SOURCE[0]}"
while [ -L "$src" ]; do
    dir="$(cd -P -- "$(dirname -- "$src")" >/dev/null 2>&1 && pwd)"
    src="$(readlink -- "$src")"
    [[ $src != /* ]] && src="$dir/$src"
done
REPO="$(cd -P -- "$(dirname -- "$src")/.." >/dev/null 2>&1 && pwd)"
cd -- "$REPO" || exit 2

BLENDER="${BLENDER:-/home/jerry/opt/blender/blender}"
OUT_DIR="assets/city_chunks"
TSV="${TMPDIR:-/tmp}/city_layout.tsv"
CELL="${CITY_CHUNK_CELL:-40}"

if [ ! -x "$BLENDER" ]; then
    echo "city_chunks: 找不到 Blender（$BLENDER）；用 BLENDER=<路径> 指定" >&2
    exit 2
fi
if [ ! -x "./target/release/steel-front" ]; then
    echo "city_chunks: 先 cargo build --release（缺 target/release/steel-front）" >&2
    exit 2
fi

if [ "${1:-}" = "--dry-run" ]; then
    echo "将会：① 无头跑一局导出布局 → $TSV"
    echo "      ② blender city_from_dump.py --cell $CELL --out-dir $OUT_DIR"
    exit 0
fi

echo "[1/2] 导出布局 → $TSV"
env WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}" \
    XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/1000}" \
    RV3D_GPU="${RV3D_GPU:-dgpu}" RV3D_HEADLESS=1 RV3D_HEADLESS_SIZE=640x360 \
    RV3D_AUTOSTART=1 RV3D_EXIT_AFTER=5 RUST_LOG=info \
    RV3D_DUMP_ALL="$TSV" \
    ./target/release/steel-front 2>&1 | grep -E 'RV3D_DUMP_ALL' || {
        echo "city_chunks: 布局导出失败" >&2
        exit 1
    }
[ -s "$TSV" ] || { echo "city_chunks: $TSV 是空的" >&2; exit 1; }

echo "[2/2] Blender 重建（cell=${CELL}m）→ $OUT_DIR/"
mkdir -p "$OUT_DIR"
"$BLENDER" -b --python tools/blender/city_from_dump.py -- \
    "$TSV" "$OUT_DIR/unused.glb" --cell "$CELL" --out-dir "$OUT_DIR" 2>&1 \
    | grep -E '\[city\]|分格' || {
        echo "city_chunks: Blender 重建失败" >&2
        exit 1
    }

n=$(find "$OUT_DIR" -name 'city_cell_*.glb' | wc -l)
echo "city_chunks: 完成，共产出 $n 个 chunk"
