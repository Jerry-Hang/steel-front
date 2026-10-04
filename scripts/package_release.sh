#!/usr/bin/env bash
# package_release.sh —— 打一个可分发目录 + 压缩包（对应 Windows 的 scripts/package_release.ps1）
#
# 用法：
#   scripts/package_release.sh                 # 构建 + 打包，tag = 当前时间
#   scripts/package_release.sh -SkipBuild      # 不重新构建，用现有 exe
#   scripts/package_release.sh -Tag rc1        # 指定 tag
#
# 产出：
#   dist/steel-front-<tag>/         可运行目录（解开就能跑）
#   dist/steel-front-<tag>.tar.gz   分发包
#
# 装什么：release exe、assets/（着色器 + 地图 + GLB 道具，**运行时从磁盘读**）、
#         README.md、LICENSE，外加一个 run.sh（理由见下）。
# 不装什么：target/、logs/、screenshots/、data/、tools/、src/、docs/、.git/。
#
# 与 Windows 侧的三处差异（**不要互相照抄**）
# ---------------------------------------
#   1) 压缩格式是 **.tar.gz 不是 .zip**：Linux 上 tar/gzip 是必备的，而 zip **本机根本没装**；
#      tar.gz 也是 Linux 分发的惯例。（Windows 侧用 Compress-Archive 出 zip 是对的，
#      因为收包方是 Windows。）
#   2) 多装一个 **run.sh**：引擎**所有资产都是相对 CWD 的路径**（assets/mesh.spv 等），
#      而 Windows 上双击 exe 时系统会把 CWD 设成 exe 所在目录 —— Linux 没有这个默认动作，
#      从文件管理器点、或从别处 `./dist/.../steel-front` 跑都会**找不到资产**。
#      run.sh 只做一件事：cd 到自己所在目录再 exec。
#   3) exe 没有 .exe 后缀，路径一律 `target/release/steel-front`。
#
# 退出码（三态，教训 46）
#   0 = 包已产出
#   1 = 跑了但失败（构建失败 / 必需资产缺失）
#   2 = 没跑成（不在仓库根、没有 cargo、exe 不存在且不许构建）
set -euo pipefail

TAG="$(date +%Y%m%d-%H%M)"
SKIP_BUILD=0
while [ $# -gt 0 ]; do
    case "$1" in
        -Tag|--tag)       TAG="${2:?-Tag 后面要给名字}"; shift 2 ;;
        -SkipBuild|--skip-build) SKIP_BUILD=1; shift ;;
        -h|--help)        sed -n '2,40p' "$0"; exit 0 ;;
        *) echo "package_release: 不认识的参数 $1（-h 看用法）" >&2; exit 2 ;;
    esac
done

repo="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo"
[ -f Cargo.toml ] || { echo "package_release: 没跑成 —— $repo 下没有 Cargo.toml" >&2; exit 2; }

if [ "$SKIP_BUILD" = 0 ]; then
    command -v cargo >/dev/null 2>&1 || { echo "package_release: 没跑成 —— 找不到 cargo" >&2; exit 2; }
    echo '[release] cargo build --release'
    cargo build --release || { echo "package_release: 构建失败" >&2; exit 1; }
fi

EXE="$repo/target/release/steel-front"
if [ ! -x "$EXE" ]; then
    if [ "$SKIP_BUILD" = 1 ]; then
        echo "package_release: 没跑成 —— $EXE 不存在（-SkipBuild 时它必须已经构建好）" >&2
        exit 2
    fi
    echo "package_release: 构建后仍然没有 $EXE" >&2; exit 1
fi

out="steel-front-$TAG"
dist="$repo/dist/$out"
rm -rf "$dist"
mkdir -p "$dist"

install -m 0755 "$EXE" "$dist/steel-front"
for f in README.md LICENSE; do
    [ -f "$repo/$f" ] && install -m 0644 "$repo/$f" "$dist/$f"
done
[ -d "$repo/assets" ] || { echo "package_release: 缺少 assets/" >&2; exit 1; }
cp -a "$repo/assets" "$dist/assets"

# run.sh：把 CWD 钉到包目录 —— 引擎资产全是相对 CWD 的路径，
# 而 Linux 不会像 Windows 双击那样自动把 CWD 设成 exe 目录。
cat > "$dist/run.sh" <<'RUN'
#!/usr/bin/env bash
# 从**本文件所在目录**启动（引擎的 assets/ 是相对 CWD 的，换目录跑会找不到资产）。
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"
exec ./steel-front "$@"
RUN
chmod 0755 "$dist/run.sh"

# --- 自检：**装出来跑不起来的包，比没有包更糟** ------------------------------
# （与 Windows 侧逐条对齐：缺任一必需资产就拒绝出包，而不是产出一个"看着成功"的坏包。）
need="steel-front assets/mesh.spv assets/triangle.vert.spv assets/triangle.frag.spv assets/maps/index.toml run.sh"
missing=""
for f in $need; do [ -e "$dist/$f" ] || missing="$missing $f"; done
if [ -n "$missing" ]; then
    echo "package_release: 包缺少必需文件:$missing" >&2
    rm -rf "$dist"
    exit 1
fi

spv=$(find "$dist/assets" -name '*.spv' | wc -l)
maps=$(find "$dist/assets/maps" -name '*.toml' 2>/dev/null | wc -l)
# ⚠️ 数**整个包**里的 glb，不要只数 assets/props —— 后者会让报告写「道具 24 个」
# 而包里其实有 56 个（props 24 / guns 16 / guns_ext 15 / soldier 1），
# 是一个会让人误判「资产漏拷了」的假数字。（Windows 侧只数 props，这里刻意不同。）
glb=$(find "$dist/assets" -name '*.glb' | wc -l)

tarball="$repo/dist/$out.tar.gz"
rm -f "$tarball"
tar -czf "$tarball" -C "$repo/dist" "$out"

size=$(du -m "$tarball" | cut -f1)
echo
echo "=== PACKAGE OK: $out ==="
echo "  目录   : $dist"
echo "  压缩包 : $tarball  (${size} MB)"
echo "  着色器 : $spv 个 spv"
echo "  地图   : $maps 个 toml"
echo "  模型   : $glb 个 glb（全包）"
echo
echo "接下来：把 tar.gz 拷到别处，解开，然后："
echo "  ./run.sh            # 或用 ./steel-front（必须在本目录下跑）"
echo "首次运行会写 ~/.steel_front.cfg（分辨率只在首次按主显示器宽高比取默认值）。"
