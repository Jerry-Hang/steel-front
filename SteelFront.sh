#!/usr/bin/env bash
# ============================================================================
#  Steel Front 启动器（Linux）—— SteelFront.bat 的对应版本
# ============================================================================
#  用法：
#    ./SteelFront.sh             构建后启动（正常路径）
#    ./SteelFront.sh play        同无参数
#    ./SteelFront.sh fast        不重新构建，直接用现有 exe 启动
#    ./SteelFront.sh diag        构建后带诊断开关启动（RV3D_AI_PROF=1 RV3D_PROP_STATS=1）
#    ./SteelFront.sh smoke       冒烟门（转交 scripts/smoke_linux.sh，可加 -Secs N）
#    ./SteelFront.sh package     打包   —— 🚫 Linux 侧尚未实现（exit 2）
#    ./SteelFront.sh --help      打印本帮助
#    ./SteelFront.sh <参数…>      构建后启动，并把参数**原样**透传给引擎
#
#  退出码按仓库的三态约定（AGENTS.md 教训 46）：0=成功 / 1=失败 / 2=没跑成。
#  本脚本里 2 只出现在「前置条件不满足」：模式未实现、工具链缺失 —— 绝不拿它冒充成功。
#  状态行走 stdout，诊断/告警走 stderr（bat 只有一条流；分开是为了 `--help | less`
#  这类用法不被警告噪声污染，也方便调用方只收告警）。
# ============================================================================
set -euo pipefail

usage() {
    cat <<'EOF'
Steel Front 启动器（Linux）：./SteelFront.sh [模式] [引擎参数…]

  play（默认）   构建后启动
  fast           不构建，直接用现有 exe 启动
  diag           构建后带诊断开关启动（RV3D_AI_PROF=1 RV3D_PROP_STATS=1）
  smoke          冒烟门：零输入跑 45s，判据 vuid==0 且 panics==0 且 killed>=1
  package        打包   —— 🚫 Linux 侧尚未实现（exit 2）
  -h / --help    打印本帮助

  RV3D_* 环境变量原样透传给引擎（本脚本只在**未设置**时补默认值，不覆盖你的值）。
  退出码：0=成功 / 1=失败 / 2=没跑成。
EOF
}

# ---- 先 cd 到脚本真正所在的目录（bat: `cd /d "%~dp0"`）---------------------
# 🔴 这一步不是可选的：引擎**所有**资产都是相对 CWD 的路径 —— `assets/mesh.spv`、
# `assets/props`、`assets/guns/*.glb`、`assets/maps/*.toml`（判据：源码里全是裸相对
# 路径，如 main.rs 的 `PropSet::load_dir("assets/props")`、`assets/soldier/soldier.glb`）。
# 从别处启动 ⇒ 渲染器初始化失败直接退出，而不是"少了几个模型"这种能看出来的症状。
#
# 顺带解掉软链接：`~/bin/steelfront -> 仓库/SteelFront.sh` 这种用法下，`$BASH_SOURCE`
# 给的是**链接**所在目录 ⇒ cd 到 ~/bin ⇒ 资产全找不到。bat 的 `%~dp0` 同样只看启动器
# 自己的位置，但 Windows 上没人把 .bat 软链到 PATH 里，Linux 上这是常规操作。
src="${BASH_SOURCE[0]}"
while [ -L "$src" ]; do
    link="$(readlink -- "$src")"
    case "$link" in
        /*) src="$link" ;;
        *) src="$(dirname -- "$src")/$link" ;;
    esac
done
cd -- "$(dirname -- "$src")"

# bat 用 `if /i` 做大小写无关的模式比较；这里也归一化一次（透传的仍是原始参数）。
MODE="${1:-play}"
MODE_LC="${MODE,,}"

case "$MODE_LC" in
    -h|--help|help)
        usage
        exit 0
        ;;
esac

# ---- smoke：转交给 Linux 版冒烟门；package 仍未实现，明确报"没跑成" ----------
# bat 里这两个模式是**转交**给 PowerShell 脚本的：
#   smoke   -> scripts/run_smoke_pm.ps1（PostMessage 注入 + 判据 vuid==0/panics==0/killed>=1）
#   package -> scripts/package_release.ps1（build + 组装 dist/steel-front-<tag>.zip）
# Linux 上这两个 .ps1 **跑不了**。
#
# smoke 现在有 Linux 版了：`scripts/smoke_linux.sh` —— 判据**逐字同口径**
# （vuid==0 and panics==0 and killed>=1），但驱动方式不同：Linux 没有 PostMessage，
# 而 XTEST 是全局注入、会抢焦点并把指针锁进游戏窗口，正好违反 Windows 侧那条
# 「鼠标安全协议」的本意 ⇒ 改用引擎自己的诊断开关做**零输入**驱动，
# 并强制 `RV3D_NO_CAPTURE=1` 让"不夺指针"成为**代码级保证**。
# 这里只**转交**，退出码原样带回（0/1/2 三态由那个脚本自己负责）。
#
# package 仍然没有 Linux 版 ⇒ 必须 exit 2：
#   * 不能 exit 0 —— 那是"假装成功"，调用方会以为包打好了（假绿灯，本仓最贵的一类事故）；
#   * 不能指向不存在的脚本 —— 那只是把一个失败推给下一层；
#   * 也不能 exit 1 —— 1 表示"跑了但失败"，而我们**根本没跑**（三态要分得清）。
# 三态约定见 AGENTS.md 教训 46。
case "$MODE_LC" in
    smoke)
        exec "$(dirname "$0")/scripts/smoke_linux.sh" "${@:2}"
        ;;
    package)
        echo "[steel-front] package 在 Linux 侧尚未实现（Windows 侧是 scripts/package_release.ps1，Linux 上跑不了）。" >&2
        echo "[steel-front] 现在没有任何可转交的 Linux 脚本 ⇒ exit 2（没跑成），不是成功、也不是失败。" >&2
        exit 2
        ;;
esac

# ---- 陈旧实例是"窗口起不来"的头号原因（bat 的 tasklist + taskkill）----------
# 🔴 只许 `pkill -x`（精确进程名），**绝不许 `pkill -f`**：`-f` 匹配整条命令行，而
# 本仓库目录名就叫 `steel-front`（/home/.../steel-front/SteelFront.sh）⇒ 用 `-f`
# 时这个模式会命中**正在运行的启动器自己**，脚本半路被自己杀掉，现象是"什么都没输出就没了"。
# 首次运行时没有进程，pgrep/pkill 返回 1 —— bat 那份也是忽略失败的（`not errorlevel 1`），
# 所以 `set -e` 下必须自己把非零吃掉。
if pgrep -x steel-front >/dev/null 2>&1; then
    echo "[steel-front] an old instance is running - closing it first."
    pkill -x steel-front >/dev/null 2>&1 || true
    # bat 用 `ping -n 2` 等窗口消失；这里轮询到进程**真的**走掉（上限 2s），
    # 比固定 sleep 更准：残留窗口会跟新实例抢同一个 Vulkan 交换链/输入抓取。
    for _ in $(seq 1 20); do
        pgrep -x steel-front >/dev/null 2>&1 || break
        sleep 0.1
    done
fi

# Linux 没有 .exe 后缀（bat: target\release\steel-front.exe）
EXE="target/release/steel-front"

if [ "$MODE_LC" != "fast" ]; then
    # ---- touch 列表（bat:67-68，AGENTS 铁律 D）----------------------------
    # cargo 靠 **mtime** 判断要不要重编；只改着色器/构建脚本时 `.rs` 源码一个字节都没动，
    # cargo 会**静默跳过**重编 ⇒ 你会以为在测新着色器，实际启动的是上一个二进制
    # ——最贵的形态是：一整轮验证验的是一个根本不存在的改动。
    #
    # `build_spv_rt.rs` 是这里最隐蔽的一条：build.rs 只声明了
    #   `rerun-if-changed=build.rs`（build.rs:1637）
    #   `rerun-if-changed=assets/rt/pt_panorama.{spv,glsl}`（build.rs:1667-1668）
    # **没有** `rerun-if-changed=build_spv_rt.rs`。一旦构建脚本声明过任何 rerun-if-changed，
    # cargo 就只盯这些路径 + build.rs 自己 ⇒ 单独改 build_spv_rt.rs，连构建脚本都不会重跑，
    # 烘进二进制的那份 RT SPIR-V 常量还是旧的。这就是 touch 不可省的**真正**原因。
    echo "[steel-front] touching build inputs..."
    for f in build.rs build_spv_rt.rs; do
        if [ -e "$f" ]; then touch -- "$f"; fi
    done
    # 反过来说：`assets/*.spv` 是**运行时**从磁盘读的（不是 build.rs 烘进二进制的），
    # 所以它们**不需要** touch —— AGENTS 铁律 D 只强制上面那两个文件。
    # （bat 那段注释里的 `copy /b FILE +,,` 陷阱是 cmd 特有的：带引号时它按源文件 basename
    #  在当前目录**新建一个残留副本**，实测 7 个垃圾文件；Linux 上 touch 没这个问题。）

    if ! command -v cargo >/dev/null 2>&1; then
        echo "[steel-front] ERROR: 找不到 cargo ⇒ 构建不可能开始（exit 2 = 没跑成）。" >&2
        echo "             装了 rustup 再来；只想跑已构建好的 exe 就用 ./SteelFront.sh fast" >&2
        exit 2
    fi

    echo "[steel-front] building (release)..."
    if ! cargo build --release; then
        echo "[steel-front] BUILD FAILED - not launching." >&2
        exit 1
    fi
fi

# ---- 起跑前先确认二进制真的在（bat:83-88）----------------------------------
if [ ! -f "$EXE" ]; then
    echo "[steel-front] ERROR: $EXE not found." >&2
    echo "             不带「fast」再跑一次 ./SteelFront.sh 先构建。" >&2
    exit 1
fi
# Linux 专有的一条：文件存在但**没有可执行位**（tar/scp/网盘同步都会丢权限位）时，
# 执行只会得到一句 `Permission denied`（rc=126），比"没构建"难懂得多。提前说清楚。
if [ ! -x "$EXE" ]; then
    echo "[steel-front] ERROR: $EXE 存在但没有可执行位（下载/解包丢了权限位？）。" >&2
    echo "             修：chmod +x $EXE   （或重新 cargo build --release）" >&2
    exit 1
fi

# ---- 明确告诉用户"马上要跑的是哪一个二进制"（bat:91）------------------------
# 有 mtime + 字节数才能事后证明"那局跑的是哪个构建"，这正是本仓要求的 provenance
# （教训 5：取证必须带 provenance）。
printf '[steel-front] exe: %s  %s bytes\n' \
    "$(date -r "$EXE" '+%Y-%m-%d %H:%M:%S')" "$(wc -c < "$EXE")"

# ---- 缺资产要吭声，否则表现成"莫名其妙一个空世界"（bat:93-96）---------------
if [ ! -d assets/props ]; then
    echo "[steel-front] WARN: assets/props missing - city will be procedural only." >&2
fi
if [ ! -d assets/maps ]; then
    echo "[steel-front] WARN: assets/maps missing - no TOML levels." >&2
fi
if [ ! -f assets/soldier/soldier.glb ]; then
    echo "[steel-front] note: no soldier.glb - NPCs use the 18-box path." >&2
fi

if [ "$MODE_LC" = "diag" ]; then
    echo "[steel-front] diagnostics on: RV3D_AI_PROF=1 RV3D_PROP_STATS=1"
    # bat 在这里是**无条件** set；本脚本只补未设置的（与上面两条默认值同一规矩：
    # 不覆盖用户显式给的值）。两者在这里**等价**，因为引擎对这两个变量读的是
    # `std::env::var(..).is_ok()` —— 只看"在不在"，**不看值**
    # （判据：game.rs:6057 与 renderer.rs:6057）⇒ 连 `RV3D_AI_PROF=0` 都是"开"。
    : "${RV3D_AI_PROF:=1}"; export RV3D_AI_PROF
    : "${RV3D_PROP_STATS:=1}"; export RV3D_PROP_STATS
fi

# ---- 真正玩的时候用 MAILBOX（bat:113，`if not defined` 同义）---------------
# 引擎默认是 IMMEDIATE（不限帧、不等垂直同步）—— 那是给基准/压测的最稳模式，
# 但在真显示器上表现为**持续撕裂**，快速转视角时读起来正好像"残影/鬼影"（2026-09-13 报告），
# 而且**抓图抓不到它**（PrintWindow 拿的是已合成帧）⇒ 别用静态截图去证伪。
# MAILBOX 既不撕裂也不阻塞；FIFO 在独显直连/手动模式下等不到 vblank 中断 ⇒ 主循环冻结
# （2026-09-25 实测 TDR，LiveKernelEvent P1=141）。
# 要退回旧行为：RV3D_PRESENT_MODE=immediate ./SteelFront.sh
: "${RV3D_PRESENT_MODE:=mailbox}"; export RV3D_PRESENT_MODE

# ---- 失焦帧率上限（bat:124，2026-09-27 实测）-------------------------------
# 实测：窗口一旦失焦，引擎照样全速 ~165fps 并把 GPU 3D 引擎占用吃到 99–105%，dwm 只剩
# 0–1% ⇒ 拖窗口/打字/开任务管理器全排在游戏后面 —— 就是用户报的"游戏跑着整台机器像卡死"。
# 那不是 CPU 降频：同一探针 3.1–4.8GHz、45–70W/115W、节流标志全 Not Active；
# 窄的是**核显那条跨适配器拷贝+合成链**（本机是混合输出，面板挂在 AMD 610M 上）。
# 失焦限 20fps 既让游戏活着、又把 GPU 还给桌面。要关掉：RV3D_BG_FPS=0
# 引擎默认是 0（不限）而且是**故意的**：perf_run/冒烟这些跑法本来就是失焦的，
# 一个默认上限会把每一次基准静默变成 20fps 的测量（判据：纯函数 effective_frame_cap 的测试）。
: "${RV3D_BG_FPS:=20}"; export RV3D_BG_FPS

# ---- 日志目录必须在重定向**之前**建好（bat:126-132）-------------------------
# `2> logs/play_latest.log.err` 是 shell 在 fork **之前**就打开的：目录不存在 ⇒ 重定向
# 直接失败，**游戏根本不会启动**（bash 只报一句 No such file or directory），
# 现象是"点了没反应"而不是"日志丢了"。所以 mkdir 必须在启动那一行之前。
mkdir -p logs
PLAYLOG="logs/play_latest.log.err"
# 每次启动前删掉上一局的日志：留着的话"这一局很干净"和"上一局的崩溃"读起来一模一样
# （教训 9/11：游戏日志看 .log.err，而且先看日志再看图）。
rm -f -- "$PLAYLOG"

echo "[steel-front] launching...  (log: $PLAYLOG)"
# 参数**原样**透传，包括模式词本身 —— bat 用的是 `%*`（`SteelFront.bat play --help`
# 会把 `play --help` 两个都交给 exe）。引擎只解析 `--inspect[=N]`（main.rs:811），
# 其余参数忽略，所以多透传一个 `play`/`fast` 不改变行为。
#
# bat 那边必须写 `start "" /b cmd /c "..."`：`start` 不带 `/b` 会**新开一个控制台**并抢前台，
# 游戏窗口拿不到 `Focused(true)`，而 main.rs::sync_cursor 要求 `self.focused` 才抓光标
# ⇒ 鼠标视角完全没反应、且哪儿都不报错（2026-09-13 报告，也就是用户报的"鼠标死锁"那一类）。
# Linux 上这个坑不存在：终端里 fork 出来的进程不新建控制台/不抢前台，焦点由窗口管理器
# 给新的顶层窗口；这里也刻意**前台**运行（不做后台作业），好处是引擎的退出码能原样带回来 ——
# bat 的 `start /b` 是异步的，那个退出码它拿不到。
rc=0
"$EXE" "$@" 2> "$PLAYLOG" || rc=$?
echo "[steel-front] exited rc=$rc  (log: $PLAYLOG)"
exit "$rc"
