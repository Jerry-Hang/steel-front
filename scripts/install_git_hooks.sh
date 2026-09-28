#!/usr/bin/env bash
# ============================================================================
#  install_git_hooks.sh —— 启用仓库内的提交守卫（.githooks/），并且**链式保留**
#  原有的钩子目录，而不是把它替换掉。（对应 Windows 侧 scripts/install_git_hooks.ps1）
# ============================================================================
#  用法：bash scripts/install_git_hooks.sh [--uninstall]
#
#  为什么要链式：`core.hooksPath` 只能指向**一个**目录。DSH 的密钥门住在
#  ~/.dsh/gates/hooks（它的 pre-push 拦"把凭据推到公网远端"）。把 hooksPath 直接改指
#  .githooks 会**静默关掉**那道门 —— 而那道门正是 2026-08-21 真实泄漏（DeepSeek key
#  被推进公开仓库）之后才加的（AGENTS 铁律 G）。所以顺序是：
#    1) 把原 hooksPath 记进 `steelfront.baseHooksPath`；
#    2) hooksPath 指向 .githooks；
#    3) .githooks/pre-commit 与 .githooks/pre-push 再显式转交给原目录的同名钩子。
#
#  退出码（三态，AGENTS.md 教训 46）：
#    0 = 装好/卸载完成（**包括**下面的巡检有命中：那只是巡检，不是门禁）
#    1 = 仓库缺少钩子文件（守卫根本不存在，装上去就是假绿灯）
#    2 = 没跑成（读不到仓库根 / 参数不认识）
# ============================================================================
set -euo pipefail

case "${1:-}" in
    "") ;;
    --uninstall) UNINSTALL=1 ;;
    -h | --help)
        echo "用法: bash scripts/install_git_hooks.sh [--uninstall]"
        exit 0
        ;;
    *)
        echo "install_git_hooks: 不认识的参数 '$1'（只认 --uninstall）⇒ 什么都没做（exit 2）" >&2
        exit 2
        ;;
esac

if ! repo="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    echo "install_git_hooks: 当前目录不在 git 工作树里，读不到仓库根 ⇒ 什么都没做（exit 2）" >&2
    exit 2
fi
cd -- "$repo"

# 🔴 `git config --get` 在"这个键没设过"时返回 **1**；而 `set -e` 下 `var=$(...)` 的退出码
# 就是命令替换的退出码 ⇒ 不写 `|| true` 的话脚本会在第一行就静默终止（现象：什么都没打印，
# 退出码 1，看着像 git 报错）。PS 版不需要管这个（非零退出码不影响 $base 的取值）。
base="$(git config --get steelfront.baseHooksPath || true)"
cur="$(git config --get core.hooksPath || true)"

# ---- 卸载：还原/清除，并抹掉我们自己的记录 ---------------------------------
if [ "${UNINSTALL:-0}" = 1 ]; then
    # `base` 必须在上面就读到手：一旦 unset 了就再也想不起来原来指向哪。
    # ⚠️ `git config` 默认读写 **local**（.git/config）。若原 hooksPath 来自 global，
    #    卸载时会把那个值落成 local 的一份拷贝 —— 与 PS 版行为一致，这里不额外处理。
    if [ -n "$base" ]; then
        git config core.hooksPath "$base"
        echo "restored core.hooksPath = $base"
    else
        # 键本来就不存在时 `--unset` 返回 5；这里要的语义是"确保它没了"，不是"必须删掉一个"
        git config --unset core.hooksPath || true
        echo "core.hooksPath cleared (back to .git/hooks)"
    fi
    git config --unset steelfront.baseHooksPath 2>/dev/null || true
    exit 0
fi

# ---- 前置检查：钩子文件不在就**先别改配置**（fail-closed，且不留半装状态）----
# PS 版是在改完 `core.hooksPath` **之后**才检查 pre-commit 的 ⇒ 缺文件时它留下的是一份
# "已经指过去、但守卫文件根本不存在"的配置：git 从此不再跑任何 pre-commit，
# 而 `core.hooksPath` 看起来配得好好的 —— 又一个静默失效的安全门。
# pre-push 同样必须查：它转交的是 DSH 的密钥门，缺了它 = 原目录的 pre-push 再也收不到推送。
for h in .githooks/pre-commit .githooks/pre-push; do
    if [ ! -f "$h" ]; then
        echo "FAIL: $h 不存在 ⇒ 不往下走（装上去只会是假绿灯）。exit 1" >&2
        exit 1
    fi
done

# ---- 1) 记录原 hooksPath（与 PS 版逐条同义）--------------------------------
if [ -n "$cur" ] && [ "$cur" != ".githooks" ]; then
    git config steelfront.baseHooksPath "$cur"
    echo "chained base hooks = $cur"
elif [ -n "$base" ]; then
    echo "already chained to = $base"
else
    echo "no previous hooksPath (nothing to chain)"
fi

# ---- 2) 指向 .githooks 并回读 ----------------------------------------------
git config core.hooksPath .githooks
echo "core.hooksPath = $(git config --get core.hooksPath)"
echo "steelfront.baseHooksPath = $(git config --get steelfront.baseHooksPath || echo '(未设置)')"

# ---- 3) 自检 ---------------------------------------------------------------
echo "hook present   = True"

# 🔴🔴 Linux 独有的坑，必须在这里处理（Windows 上这条**从来不成立**）：
# 本仓这两个钩子在 git index 里的模式是 **100644（不可执行）** ——
# 判据：`git ls-files -s .githooks/` 两行都以 `100644` 开头。
# 而 Linux 上 **git 会静默跳过一个不可执行的钩子**：它只在 stderr 留一句
# "hook was ignored because it's not set as executable" 就走了，提交/推送照常发生。
# 于是 clone 出来的人：`core.hooksPath` = .githooks、文件都在、`hook present = True`，
# 但守卫**一次都没跑过** —— 白名单与密钥扫描全部失效，而且没有任何一处报错。
# 为什么在 Windows 上没人发现：Git for Windows 一律经 `sh` 执行钩子，**不看权限位**，
# 所以这个缺陷一路漂到 Linux 才露头。
# ⇒ 装的时候顺手打上可执行位，并且**真的验一遍权限位**（`test -x`），
#   判据取权限位本身，不取"文件存在"——否则装完还是假绿灯。
chmod +x .githooks/pre-commit .githooks/pre-push
for h in .githooks/pre-commit .githooks/pre-push; do
    if [ ! -x "$h" ]; then
        echo "FAIL: $h 仍然没有可执行位 —— Linux 上 git 会静默跳过它，守卫等于没装。exit 1" >&2
        exit 1
    fi
done
echo "hooks executable = True (pre-commit, pre-push)"

# python 探测：Linux 上**优先 python3**（不少发行版根本没有 `python` 这个名字；
# .githooks/pre-commit 里是先 python 再 python3，顺序反了不影响结果）。
py="$(command -v python3 || command -v python || true)"
if [ -n "$py" ]; then
    echo "python         = $py"
else
    echo "WARN: 找不到 python3/python ⇒ 钩子会跳过密钥扫描（白名单不生效）" >&2
fi

# ---- 巡检（**不是**门禁）---------------------------------------------------
# 退出码**只打印不判断**，最后仍然 exit 0 —— 与 PS 版一致：这是装完之后顺手巡检一遍，
# "装钩子"这件事本身已经成功了；真正拦提交的是钩子，不是这一行。
# 但打印要能读：commit_guard.py 是三态的 ——
#   0 = 扫过、无命中 / 1 = 有命中（有人正在往仓库里放不该放的东西）/ 2 = 根本没扫成。
# ⚠️ 0 与 2 的区别就是"干净"和"没查"的区别（教训 46：空日志不算通过）。
echo "--- guard self-test (whole tree) ---"
if [ -n "$py" ]; then
    rc=0
    "$py" "$repo/tools/commit_guard.py" --scan . || rc=$?
    echo "guard scan exit = $rc   (0=无命中 / 1=有命中 / 2=没扫成)"
else
    echo "guard scan exit = 2   (没跑成：没有 python —— 这**不等于**通过)" >&2
fi

exit 0
