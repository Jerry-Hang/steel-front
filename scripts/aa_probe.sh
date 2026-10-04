#!/usr/bin/env bash
# aa_probe.sh —— A/A 噪声底探针：**同一个二进制**跑 N 次，报出散布（对应 Windows 的 scripts/aa_probe.ps1）
#
# 为什么需要它
# -----------
# 2026-09-26：一次阴影 pass 的 A/B 得出「中位 fps 101.8 → 161.2（+58%）」，随后被质疑，
# 用四次**完全相同**的运行复核（一个字符都没改）：
#     run1 均值 131.3   run2 130.6   run3 137.9   run4 146.8
# 即同一份二进制在该指标上能差 ~12%，而"证明"了 +58% 的那一对只是这个散布里的两个样本。
# 教训 24/35 说单次 A/B 什么都证明不了，但仓库里**没有工具去量这个底噪** ——
# 只有一句"两次跑差 2.8%"的注释。这个脚本就是那把尺子：**宣称任何性能差之前先跑它。**
#
# 怎么读结果：打印出来的 spread（(max-min)/min）就是底噪。
# **低于这个底噪的"改善"一律算没测到**，无论两份日志看起来多干净。
#
# 用法：
#   scripts/aa_probe.sh
#   scripts/aa_probe.sh -Runs 6 -Secs 20 -Stress 128        # 常用主干
#   scripts/aa_probe.sh -Extra "RV3D_SHADOW_EVERY=1"        # 针对某套配置的 A/A
#
# 退出码（三态，教训 46）
#   0 = 每一次请求的运行都产出了稳态 fps 行（散布**就是**本配置的底噪）
#   1 = 可用运行 < 2（散布无从谈起）
#   2 = **部分批次**：散布打出来了，但它**不是**这份二进制+这套参数的底噪
#       —— 缺的那几次可能正好落在散布的端点外侧，拿它当底噪会低估。
set -euo pipefail

RUNS=4
SECS=20
STRESS=128
EXTRA=""
while [ $# -gt 0 ]; do
    case "$1" in
        -Runs|--runs)     RUNS="${2:?-Runs 后面要给次数}"; shift 2 ;;
        -Secs|--secs)     SECS="${2:?-Secs 后面要给秒数}"; shift 2 ;;
        -Stress|--stress) STRESS="${2:?-Stress 后面要给数量}"; shift 2 ;;
        -Extra|--extra)   EXTRA="${2:?-Extra 后面要给 K=V,...}"; shift 2 ;;
        -h|--help)        sed -n '2,40p' "$0"; exit 0 ;;
        *) echo "aa_probe: 不认识的参数 $1（-h 看用法）" >&2; exit 2 ;;
    esac
done
case "$RUNS" in (*[!0-9]*|'') echo "aa_probe: -Runs 必须是正整数" >&2; exit 2 ;; esac
[ "$RUNS" -ge 2 ] || { echo "aa_probe: -Runs 至少 2（1 次无从谈散布）" >&2; exit 2; }

repo="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo"
PERF="$repo/scripts/perf_run.sh"
OUT="$repo/logs/aa_probe_raw.txt"
[ -x "$PERF" ] || { echo "aa_probe: 找不到可执行的 $PERF" >&2; exit 2; }

: > "$OUT"
MEANS=""
MEDIANS=""
FAILED=0
rc=0
for i in $(seq 1 "$RUNS"); do
    args=(-Secs "$SECS" -Stress "$STRESS")
    [ -n "$EXTRA" ] && args+=(-Extra "$EXTRA")
    # ⚠️ 不要写成 `if out=$("$PERF" ...)`：那样拿不到退出码。
    out="$("$PERF" "${args[@]}" 2>&1)" && rc=0 || rc=$?
    perf=$(printf '%s\n' "$out" | sed -n 's/.*perf log = \(.*\)$/\1/p' | tail -1)
    # 取 **fps** 那一行（它是 stat_line 的第一行）。同 Windows 侧：正则撞上的第一处就是它。
    line=$(printf '%s\n' "$out" | grep -E '^[[:space:]]*fps[[:space:]]+mean' | head -1)
    mean=$(printf '%s' "$line" | sed -n 's/.*mean[[:space:]]\+\([0-9.]\+\)[[:space:]]\+median.*/\1/p')
    median=$(printf '%s' "$line" | sed -n 's/.*mean[[:space:]]\+[0-9.]\+[[:space:]]\+median[[:space:]]\+\([0-9.]\+\)[[:space:]]*.*/\1/p')

    if [ "$rc" -ne 0 ] || [ -z "$mean" ] || [ -z "$median" ]; then
        FAILED=$((FAILED + 1))
        echo "aa_probe: 第 $i 次不可用（perf_run 退出码 $rc）；末尾几行："
        printf '%s\n' "$out" | tail -6 | sed 's/^/    /'
        printf 'run %s\tUNUSABLE\trc=%s\t%s\n' "$i" "$rc" "$perf" >> "$OUT"
        continue
    fi
    MEANS="$MEANS $mean"
    MEDIANS="$MEDIANS $median"
    printf 'run %s\tmean %s\tmedian %s\t%s\n' "$i" "$mean" "$median" "$perf" >> "$OUT"
    printf 'aa_probe: 第 %s/%s 次   均值 %s   中位 %s   (%s)\n' "$i" "$RUNS" "$mean" "$median" "${perf:-?}"
done

# 用一个 awk 统一算 min/max/mean/spread（与 Windows 侧同一口径：(max-min)/min）
spread() {  # $1=名称 $2=数值串
    # shellcheck disable=SC2086
    printf '%s\n' $2 | awk -v name="$1" '
        { v[NR]=$1; s+=$1; if (NR==1||$1<mn) mn=$1; if (NR==1||$1>mx) mx=$1 }
        END {
            if (NR==0) exit
            pct = (mn>0) ? 100.0*(mx-mn)/mn : 0.0
            printf "  %-8s min %7.2f  max %7.2f  mean %7.2f  spread %.1f%%\n", name, mn, mx, s/NR, pct
        }'
}
n_mean=$(printf '%s\n' $MEANS | grep -c . || true)

echo
printf '==== A/A 噪声底：%s 次可用 / 请求 %s 次（secs=%s stress=%s%s）====\n' \
    "$n_mean" "$RUNS" "$SECS" "$STRESS" "$([ -n "$EXTRA" ] && echo " extra=$EXTRA")"
if [ "$n_mean" -lt 2 ]; then
    echo "aa_probe: FAIL —— 可用运行少于 2 次，对散布说不出任何话" >&2
    exit 1
fi
spread "mean" "$MEANS"
spread "median" "$MEDIANS"
echo "  **低于上面这个 spread 的「改善」一律算没测到**（教训 24/35）"
echo "  原始记录 $OUT"

if [ "$FAILED" -gt 0 ]; then
    echo "aa_probe: exit 2 —— $RUNS 次里有 $FAILED 次不可用；上表的散布**不是**本配置的底噪" >&2
    exit 2
fi
exit 0
