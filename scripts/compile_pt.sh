#!/usr/bin/env bash
# ============================================================================
#  compile_pt.sh —— 编译 PT 全景着色器：assets/rt/pt_panorama.glsl -> .spv
#  （对应 Windows 侧 scripts/compile_pt.ps1）
# ============================================================================
#  用法：bash scripts/compile_pt.sh      （在仓库任意位置都能跑，路径由脚本自身推）
#
#  为什么要有脚本、不许手工拼命令：glslang 的参数必须**逐字**一致
#  （`-V --target-env vulkan1.3 -S comp`）。PT 的 SPIR-V 是**运行时从磁盘读**的
#  （build.rs 只把它读进常量，读盘发生在引擎启动时），所以编错了不会有任何报错，
#  只会"画面发灰发脏"——这类静默缺陷只能靠固定命令 + 严格校验挡。
#
#  退出码（三态，AGENTS.md 教训 46）：
#    0 = glslang 编译成功**且** spirv-val 严格校验通过
#    1 = 编译真的失败，或校验真的拒绝了这份模块
#    2 = 编出了 .spv 但**没能校验**（ex spirv-val 缺失）—— 既不是成功也不是编译失败
#
#  ⚠️ 悬空指针（本次**不改**已有文件，只在这里记一笔）：build.rs:1678 在 GLSL 比 SPV 新时打
#     `cargo:warning=PT GLSL 比 SPV 新，请跑 scripts/compile_pt.ps1 重新编译 pt_panorama.spv`
#  Linux 上对应的就是**本脚本**。看到那条 warning 时跑 `bash scripts/compile_pt.sh`，
#  别去照着 warning 里的 .ps1 名字找文件（那个在 Linux 上不存在）。
# ============================================================================
set -euo pipefail

# `set -euo pipefail` 是 PS 版开头 `$ErrorActionPreference = 'Stop'` 的对应物，
# 而且这里补上了一段历史：PS 那份的 `$ErrorActionPreference = 'Stop'` **曾经被静默注释掉**
# ——行尾中文被 PS 5.1 的 ANSI(GBK) 解码吃掉、把下一行并进注释（2026-09-26 实测，
# 判据 `powershell_scripts_never_end_a_line_with_a_non_ascii_byte`）。也就是说它有一段时间是
# "编译失败也继续往下走"。bash 没有那个编码陷阱，但**每一条外部命令仍要自己判退出码**。

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(dirname -- "$here")" # 脚本住在 scripts/ 下，仓库根 = 它的上一层
glsl="$root/assets/rt/pt_panorama.glsl"
spv="$root/assets/rt/pt_panorama.spv"

# ---- 工具探测 --------------------------------------------------------------
# 顺序与 PS 版相反是**故意的**：Linux 上 glslangValidator 由发行版包提供、就在 PATH 里
# （Arch: `glslang` 包 -> /usr/bin/glslangValidator；spirv-val 在 `spirv-tools` 包里），
# 先查 PATH 才能让"装了包的人"零配置可用；$VULKAN_SDK 只当后备（PS 版只能去
# C:\VulkanSDK\*\Bin 里翻，因为 Windows 上没有这个包管理器路径）。
find_tool() {
    local name="$1" hit
    if hit="$(command -v "$name" 2>/dev/null)"; then
        printf '%s\n' "$hit"
        return 0
    fi
    if [ -n "${VULKAN_SDK:-}" ] && [ -x "$VULKAN_SDK/bin/$name" ]; then
        printf '%s\n' "$VULKAN_SDK/bin/$name"
        return 0
    fi
    return 1
}

if [ ! -f "$glsl" ]; then
    echo "compile_pt: 找不到输入 $glsl ⇒ 没编译（exit 1）" >&2
    exit 1
fi

# 找不到编译器 = 完全没跑成，但 PS 版这里就是 exit 1，保持一致（调用方只关心非零）。
if ! GLSLANG="$(find_tool glslangValidator)"; then
    echo "compile_pt: 找不到 glslangValidator（Arch: pacman -S glslang；或设 VULKAN_SDK 指向 Vulkan SDK）" >&2
    exit 1
fi

# ---- 编译：参数逐字照抄 compile_pt.ps1:21（连顺序都不改）--------------------
# `-V` = 产出 SPIR-V 二进制；`--target-env vulkan1.3` 决定可用的能力集（引擎的
# ray query / 网格着色路径都依赖它）；`-S comp` 声明这是 compute 着色器。
# 改任何一个都可能产出一份"能加载但语义不同"的模块（例如少了 target-env 就丢掉
# vulkan1.3 的能力声明），而 PT 是运行时读盘 ⇒ 不会有编译期报错兜底。
if ! "$GLSLANG" -V --target-env vulkan1.3 -S comp -o "$spv" "$glsl"; then
    echo "compile_pt: glslangValidator 编译失败 —— $spv 没被更新，别把旧的那份当新的用（exit 1）" >&2
    exit 1
fi
echo "OK  $spv"

# ---- 严格校验：这一步是**门**，不是装饰 -------------------------------------
# 🔴 这里修掉了原版的一个真实缺陷：compile_pt.ps1:25-30 用 `if ($v)` 判断 spirv-val 存在与否，
# **找不到就把整段校验跳过**，脚本照样以 0 退出 —— 一个会喊"通过"却根本没校验过的工具，
# 也就是假绿灯（本仓最贵的一类事故，教训 36：工具跑不起来本身就是缺陷）。
# 严格校验在这里不是可选项：本仓历史上就吃过"mesh.spv 过不了严格 spirv-val ⇒
# 一开验证层直接灰屏"（AGENTS 铁律 B），而 PT 模块进的是真设备。
# Linux 版 fail-closed：找不到 spirv-val 一律 **exit 2**（.spv 已经写出去了，但没有任何门
# 看着它 —— 按三态约定这既不是"成功"也不是"编译失败"，而且绝不能是 0）。
if ! SPIRV_VAL="$(find_tool spirv-val)"; then
    echo "compile_pt: 🔴 找不到 spirv-val —— $spv 已写出，但**没有校验过**，不许当它通过（exit 2）。" >&2
    echo "            装它：Arch 的 spirv-tools 包，或设 VULKAN_SDK 指向 Vulkan SDK。" >&2
    exit 2
fi

if ! "$SPIRV_VAL" --target-env vulkan1.3 "$spv"; then
    echo "compile_pt: spirv-val 拒绝了 $spv（严格 vulkan1.3）—— 这份模块不许进设备（exit 1）" >&2
    exit 1
fi
echo "OK  spirv-val (vulkan1.3) passed"
