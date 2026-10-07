#!/usr/bin/env bash
# install_desktop.sh —— 把 Steel Front 装进 Linux 桌面（应用菜单 / 启动器）
#
# 为什么是"装 .desktop"而不是移植 launcher/
# ----------------------------------------
# `launcher/` 是一个 **986 行、零依赖的 Win32 原生 GUI**（`#![cfg(windows)]` +
# `#![windows_subsystem]`，自带安装向导/自动更新/桌面快捷方式）。
# 把它搬到 Linux 要么引入 GUI 依赖（**违反「不新增第三方依赖」这条硬约束**），
# 要么用 X11/Wayland 原语重写一遍 —— 都不划算。
# 而 Linux 的"启动器"本来就是**桌面环境提供的**：一份 `.desktop` + 一个图标，
# 就得到了 launcher 里"双击就能启动"那一项，且零代码依赖。
#
# 本脚本只动**你自己的家目录**（`~/.local/share/**`），不碰 /etc、不碰 systemd、
# 不需要 sudo。完整卸载：`scripts/install_desktop.sh --uninstall`。
#
# 关于工作目录（易错点）
# --------------------
# 引擎按**相对路径**读 `assets/`（`package_release.sh` 头部也记了这个坑）。
# 好在 `SteelFront.sh` 开头的 `cd -- "$(dirname -- "$src")"` 已经把自己锚定到
# **脚本真实所在目录**（还解了软链接）⇒ 从应用菜单启动时 CWD 是别的目录也没关系。
# 所以这里的 `Exec=` 只要给**绝对路径**即可，不需要再包一层 `cd`。
#
# 用法：
#   scripts/install_desktop.sh                # 安装（或覆盖安装）
#   scripts/install_desktop.sh --check        # 只检查前置条件与将要写哪些文件，不落盘
#   scripts/install_desktop.sh --uninstall    # 卸载（含图标与缓存项）
#
# 退出码：0=成功 / 1=失败 / 2=没跑成（前置条件不满足，教训 46）
set -euo pipefail

APP_ID="steel-front"
APP_NAME="钢铁前线"
# 🔴 必须加引号：值里有**空格**，不加的话 bash 会把 `FPS（Rust` 当成命令去执行
# （实测报错 `FPS（Rust: 未找到命令`，退出码 127）。同文件里所有赋值一律带引号。
APP_COMMENT="大规模战场 FPS（Rust + Vulkan）"

SIZES="16 24 32 48 64 128 256"

MODE=install
case "${1:-}" in
    --check)     MODE=check ;;
    --uninstall) MODE=uninstall ;;
    -h|--help)   sed -n '2,32p' "$0"; exit 0 ;;
    '')          ;;
    *) echo "install_desktop: 不认识的参数 $1（-h 看用法）" >&2; exit 2 ;;
esac

# ---- 锚定仓库根：解软链接，与 SteelFront.sh 同一套做法 ---------------------
src="${BASH_SOURCE[0]}"
while [ -L "$src" ]; do
    link="$(readlink -- "$src")"
    case "$link" in
        /*) src="$link" ;;
        *)  src="$(dirname -- "$src")/$link" ;;
    esac
done
REPO="$(cd -- "$(dirname -- "$src")/.." && pwd)"
LAUNCHER="$REPO/SteelFront.sh"
ICON_SVG="$REPO/desktop/$APP_ID.svg"

DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
APPS_DIR="$DATA/applications"
ICON_BASE="$DATA/icons/hicolor"
DESKTOP_FILE="$APPS_DIR/$APP_ID.desktop"

[ -f "$LAUNCHER" ] || { echo "install_desktop: 缺少 $LAUNCHER" >&2; exit 2; }
[ -f "$ICON_SVG" ] || { echo "install_desktop: 缺少图标 $ICON_SVG" >&2; exit 2; }
[ -x "$LAUNCHER" ] || { echo "install_desktop: $LAUNCHER 不可执行（chmod +x）" >&2; exit 2; }

# ---- 卸载 ------------------------------------------------------------------
if [ "$MODE" = uninstall ]; then
    n=0
    [ -f "$DESKTOP_FILE" ] && { rm -f "$DESKTOP_FILE"; n=$((n+1)); }
    for s in $SIZES; do
        f="$ICON_BASE/${s}x${s}/apps/$APP_ID.png"
        [ -f "$f" ] && { rm -f "$f"; n=$((n+1)); }
    done
    f="$ICON_BASE/scalable/apps/$APP_ID.svg"
    [ -f "$f" ] && { rm -f "$f"; n=$((n+1)); }

    command -v update-desktop-database >/dev/null && \
        update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
    command -v gtk-update-icon-cache >/dev/null && \
        gtk-update-icon-cache -f -t "$ICON_BASE" >/dev/null 2>&1 || true

    if [ "$n" -eq 0 ]; then
        echo "install_desktop: 本来就没装（没有可删的文件）—— 没跑成" >&2
        exit 2
    fi
    echo "install_desktop: 已卸载 $n 个文件（桌面菜单里的条目可能要注销/重登才消失）"
    exit 0
fi

# ---- 前置条件（--check 也走这一段）-----------------------------------------
have_rsvg=0
command -v rsvg-convert >/dev/null && have_rsvg=1

echo "install_desktop: 仓库      = $REPO"
echo "install_desktop: 启动器    = $LAUNCHER"
echo "install_desktop: 图标源    = $ICON_SVG"
echo "install_desktop: 将写入    = $DESKTOP_FILE"
if [ "$have_rsvg" = 1 ]; then
    echo "install_desktop: 图标尺寸  = $SIZES（rsvg-convert 已就绪）"
else
    echo "install_desktop: 图标尺寸  = 只有 scalable/（**没有 rsvg-convert**，不出 PNG）" >&2
fi

if [ "$MODE" = check ]; then
    echo "install_desktop: --check 结束，没有写任何文件"
    exit 0
fi

# ---- 写 .desktop -----------------------------------------------------------
mkdir -p "$APPS_DIR" "$ICON_BASE/scalable/apps"

# Exec 用绝对路径（带引号：路径可能有空格）
#   %U = 接受文件/URL 参数（本游戏不读文件，但留着不碍事，且让"用……打开"不报错）
#   Terminal=false：游戏不该弹终端。⚠️ 代价是**首次构建失败时看不到原因** ——
#   所以首次请先在终端跑一次 ./SteelFront.sh（要 cargo build --release），
#   之后从菜单点就不会再构建失败。这一条写进 docs/linux-native.md §17。
cat >"$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=$APP_NAME
Name[en]=Steel Front
Comment=$APP_COMMENT
Comment[en]=Large-battlefield FPS (Rust + Vulkan)
Exec="$LAUNCHER" play
Path=$REPO
Icon=$APP_ID
Terminal=false
Categories=Game;ActionGame;
Keywords=FPS;Vulkan;Rust;shooter;射击;钢铁前线;
StartupNotify=true
StartupWMClass=steel-front
EOF
chmod 0644 "$DESKTOP_FILE"

# ---- 图标：scalable 一定装；PNG 有 rsvg-convert 才出 -----------------------
install -m 0644 "$ICON_SVG" "$ICON_BASE/scalable/apps/$APP_ID.svg"
# 计数从 2 起：.desktop 一个 + scalable 图标一个（原先写 1，少算了 .desktop 本体）
made=2
if [ "$have_rsvg" = 1 ]; then
    for s in $SIZES; do
        d="$ICON_BASE/${s}x${s}/apps"
        mkdir -p "$d"
        rsvg-convert -w "$s" -h "$s" "$ICON_SVG" -o "$d/$APP_ID.png"
        made=$((made+1))
    done
fi

# ---- 刷缓存（都是 best-effort：没有这些工具也不影响"文件已就位"）----------
command -v update-desktop-database >/dev/null && \
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null && \
    gtk-update-icon-cache -f -t "$ICON_BASE" >/dev/null 2>&1 || true

# ---- 自检：装了就要能验，不能只说"写完了"（教训 46）------------------------
echo
echo "=== 自检 ==="
ok=1
if [ ! -s "$DESKTOP_FILE" ]; then
    echo "  ❌ .desktop 没写出来" >&2; ok=0
fi
# 关键字段逐条核（用 -qx 精确匹配，别用子串：Exec 里也可能含 Path 的值）
for kv in "Type=Application" "Icon=$APP_ID" "Terminal=false"; do
    if grep -qxF -- "$kv" "$DESKTOP_FILE" 2>/dev/null; then
        echo "  ✅ $kv"
    else
        echo "  ❌ 缺字段：$kv" >&2; ok=0
    fi
done
if grep -qxF -- "Exec=\"$LAUNCHER\" play" "$DESKTOP_FILE" 2>/dev/null; then
    echo "  ✅ Exec 指向绝对路径且带 play"
else
    echo "  ❌ Exec 不对" >&2; ok=0
fi

if command -v desktop-file-validate >/dev/null; then
    if desktop-file-validate "$DESKTOP_FILE" 2>/dev/null; then
        echo "  ✅ desktop-file-validate 通过"
    else
        echo "  ❌ desktop-file-validate 报错：" >&2
        desktop-file-validate "$DESKTOP_FILE" >&2 || true
        ok=0
    fi
else
    echo "  ⚠️  没有 desktop-file-validate，跳过格式校验（**这不等于通过**）"
fi

[ -s "$ICON_BASE/scalable/apps/$APP_ID.svg" ] && echo "  ✅ 图标 scalable 已装" || { echo "  ❌ 图标缺失" >&2; ok=0; }
if [ "$have_rsvg" = 1 ]; then
    miss=""
    for s in $SIZES; do
        [ -s "$ICON_BASE/${s}x${s}/apps/$APP_ID.png" ] || miss="$miss $s"
    done
    [ -z "$miss" ] && echo "  ✅ 图标 PNG 全部就位（$SIZES）" || { echo "  ❌ 缺尺寸:$miss" >&2; ok=0; }
fi

if [ "$ok" != 1 ]; then
    echo "install_desktop: exit 1 —— 有自检项没过（见上）" >&2
    exit 1
fi

echo
echo "install_desktop: 完成（共 $made 个文件）。"
echo "  · 菜单里搜「$APP_NAME」或 Steel Front 即可"
echo "  · 卸载：scripts/install_desktop.sh --uninstall"
echo "  · ⚠️ 首次运行前请在终端先跑一次 ./SteelFront.sh（要编译；菜单启动看不到构建报错）"
exit 0
