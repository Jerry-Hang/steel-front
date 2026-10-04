#!/usr/bin/env bash
# ab_pair.sh —— 两个二进制之间的**交替** A/B 驱动器（对应 Windows 的 scripts/ab_pair.ps1）
#
# 为什么需要它
# -----------
# 教训 24：单次 A/B 什么都证明不了。在会漂移的机器上唯一站得住的设计是
#   (a) **逐对交替**两个臂，让漂移对两边的影响相同；
#   (b) 重复；
#   (c) 用**配对差的中位数**排名；
#   (d) 拿同一个 exe 在两个臂里各跑一次，量出 A/A 底噪。
# 教训 43/45 再补两条：尺子用引擎自己的 window-fps 列；**正对照臂不动 = 整批作废**。
# 在这之前这套东西每次都是现写的（2026-09-26 那次重测就是），这个脚本把它固下来。
#
# 尺子：**原样复用 `scripts/perf_run.sh`**（同样的 env、同样的日志、同样的稳态规则
# `t >= 3s`），这样数字与之前每一次 perf_run / aa_probe 的测量可比。
#
# 用法
# ----
#   # 1) 先摆好两个二进制（怎么构建随你，比如改一个常量再重编）
#   #    logs/ab/old  logs/ab/new
#   # 2) 先量 A/A 底噪（同一个文件给两次）—— 3 对足以看出散布
#   scripts/ab_pair.sh -Pairs 3 -ExeA logs/ab/new -ExeB logs/ab/new -LabelA newA -LabelB newB
#   # 3) 再跑真的，并且**两个方向都跑**（教训 24：把两个臂对调，看结论还成不成立）
#   scripts/ab_pair.sh -Pairs 5
#   scripts/ab_pair.sh -Pairs 3 -ExeA logs/ab/new -ExeB logs/ab/old -LabelA new129 -LabelB old257
#
#   成本地图常用形态：**同一个 exe，只有 `-ExtraB` 不同**（一个臂关掉某项工作）。
#   ⇒ 那个"少干活"的臂**必须**明显更快；不动就说明整批在量漂移（教训 45）。
#
# 怎么读输出：「MEDIAN PAIRED DELTA」是效应，「arm range」是漂移，符号检验是几对同向。
# **落在 A/A 底噪之内的差不是效应。**
#
# 退出码（三态，教训 46）
#   0 = 每一对都跑完了
#   1 = 批次根本没起来（缺 exe / 没有可用的对）
#   2 = **部分对被跳过**（某一臂的 perf_run 非零）⇒ 批次已降级，
#       **不许当成教训 45 要的「n >= 5 对」证据**。统计仍会打出来供查看。
set -euo pipefail

PAIRS=4
SECS=25
EXEA="logs/ab/old"
EXEB="logs/ab/new"
LABELA="A"
LABELB="B"
EXTRA_A=""
EXTRA_B=""

while [ $# -gt 0 ]; do
    case "$1" in
        -Pairs|--pairs)   PAIRS="${2:?-Pairs 后面要给对数}"; shift 2 ;;
        -Secs|--secs)     SECS="${2:?-Secs 后面要给秒数}"; shift 2 ;;
        -ExeA|--exe-a)    EXEA="${2:?-ExeA 后面要给路径}"; shift 2 ;;
        -ExeB|--exe-b)    EXEB="${2:?-ExeB 后面要给路径}"; shift 2 ;;
        -LabelA|--label-a) LABELA="${2:?-LabelA 后面要给名字}"; shift 2 ;;
        -LabelB|--label-b) LABELB="${2:?-LabelB 后面要给名字}"; shift 2 ;;
        -ExtraA|--extra-a) EXTRA_A="${2:?-ExtraA 后面要给 K=V,...}"; shift 2 ;;
        -ExtraB|--extra-b) EXTRA_B="${2:?-ExtraB 后面要给 K=V,...}"; shift 2 ;;
        -h|--help)        sed -n '2,42p' "$0"; exit 0 ;;
        *) echo "ab_pair: 不认识的参数 $1（-h 看用法）" >&2; exit 2 ;;
    esac
done
case "$PAIRS" in (*[!0-9]*|'') echo "ab_pair: -Pairs 必须是正整数" >&2; exit 2 ;; esac
[ "$PAIRS" -ge 1 ] || { echo "ab_pair: -Pairs 至少 1" >&2; exit 2; }

repo="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo"
TARGET="$repo/target/release/steel-front"
PERF="$repo/scripts/perf_run.sh"
PATH_A="$repo/$EXEA"
PATH_B="$repo/$EXEB"
for p in "$PATH_A" "$PATH_B"; do
    [ -f "$p" ] || { echo "ab_pair: 缺少 $p" >&2; exit 1; }
done
[ -x "$PERF" ] || { echo "ab_pair: 找不到可执行的 $PERF" >&2; exit 2; }

# 跑一个臂：把该臂的二进制拷到引擎路径，再交给 perf_run 量。
# 🔴 **失败一律 fail-closed**（返回空）—— perf_run 的退出码已经区分了
# "没数据"(1) 与 "有统计但没有稳态窗口"(2)，两种都不是可用的测量。
run_arm() {  # $1=exe $2=label $3=extra
    local exe="$1" label="$2" extra="$3" out rc line mean median
    cp -f "$exe" "$TARGET" || { echo "ab_pair: $label 无法把 exe 拷到 $TARGET" >&2; return 1; }
    # ⚠️ Extra 为空时**整个省略这个参数**。ps1 里记着同一个坑：空串会让参数绑定失败、
    # perf_run 根本没跑、每一对都报 incomplete（第一次成本地图就这么白转了十分钟）。
    if [ -n "$extra" ]; then
        out="$("$PERF" -Secs "$SECS" -Extra "$extra" 2>&1)" && rc=0 || rc=$?
    else
        out="$("$PERF" -Secs "$SECS" 2>&1)" && rc=0 || rc=$?
    fi
    if [ "$rc" -ne 0 ]; then
        echo "ab_pair: $label 的 perf_run 退出码 $rc；末尾几行：" >&2
        printf '%s\n' "$out" | tail -6 | sed 's/^/    /' >&2
        return 1
    fi
    line=$(printf '%s\n' "$out" | grep -E '^[[:space:]]*fps[[:space:]]+mean' | head -1)
    [ -n "$line" ] || { echo "ab_pair: $label 没有 fps 行" >&2; return 1; }
    median=$(printf '%s' "$line" | sed -n 's/.*median[[:space:]]\+\([0-9.]\+\)[[:space:]]*.*/\1/p')
    [ -n "$median" ] || { echo "ab_pair: $label 的 fps 行解析不出中位数：$line" >&2; return 1; }
    printf '%s' "$median"
}

median_of() {  # 可移植：sort + awk，不依赖 gawk 的 asort
    sort -n | awk '{v[NR]=$1} END{ n=NR; if(n==0) exit; print (n%2)?v[(n+1)/2]:(v[n/2]+v[n/2+1])/2 }'
}
median_of_args() { for x in "$@"; do printf '%s\n' "$x"; done | median_of; }

echo "ab_pair: $PAIRS 对，每臂 ${SECS}s；A=$LABELA ($EXEA)$([ -n "$EXTRA_A" ] && echo " [$EXTRA_A]")  B=$LABELB ($EXEB)$([ -n "$EXTRA_B" ] && echo " [$EXTRA_B]")"
DELTAS=(); AS=(); BS=(); INCOMPLETE=0
for i in $(seq 1 "$PAIRS"); do
    # 教训 45 要求轮转，**包括一对之内的先后**：奇数对 A→B、偶数对 B→A（ABBA…）。
    # 两个臂仍然按"同一对的配对差"比较，所以轮转只去掉"每对里先跑的那个系统性偏慢/偏快"
    # 这个固定 A→B 顺序会烘进每一对的残差。
    if [ $((i % 2)) -eq 0 ]; then order="BA"; else order="AB"; fi
    if [ "$order" = "BA" ]; then
        b=$(run_arm "$PATH_B" "$LABELB" "$EXTRA_B") || b=""
        a=$(run_arm "$PATH_A" "$LABELA" "$EXTRA_A") || a=""
    else
        a=$(run_arm "$PATH_A" "$LABELA" "$EXTRA_A") || a=""
        b=$(run_arm "$PATH_B" "$LABELB" "$EXTRA_B") || b=""
    fi
    if [ -z "$a" ] || [ -z "$b" ]; then
        echo "ab_pair: 第 $i 对不完整，跳过"
        INCOMPLETE=$((INCOMPLETE + 1))
        continue
    fi
    AS+=("$a"); BS+=("$b")
    d=$(awk -v a="$a" -v b="$b" 'BEGIN{printf "%.4f", b-a}')
    DELTAS+=("$d")
    awk -v i="$i" -v la="$LABELA" -v a="$a" -v lb="$LABELB" -v b="$b" -v o="$order" \
        'BEGIN{printf "  第 %s 对 [%s]: %s=%.2f  %s=%.2f  delta=%+.2f (%+.1f%%)\n", i,o,la,a,lb,b,b-a,100.0*(b-a)/a}'
done

[ "${#DELTAS[@]}" -gt 0 ] || { echo "ab_pair: 一对都没跑完" >&2; exit 1; }

MA=$(median_of_args "${AS[@]}"); MB=$(median_of_args "${BS[@]}"); MD=$(median_of_args "${DELTAS[@]}")
range_of() { printf '%s\n' "$@" | sort -n | awk -v m="$1" '{v[NR]=$1} END{ if(NR<1) exit; printf "%.1f", 100.0*(v[NR]-v[1])/v[1] }'; }
RA=$(printf '%s\n' "${AS[@]}" | sort -n | awk '{v[NR]=$1} END{printf "%.1f", 100.0*(v[NR]-v[1])/v[1]}')
RB=$(printf '%s\n' "${BS[@]}" | sort -n | awk '{v[NR]=$1} END{printf "%.1f", 100.0*(v[NR]-v[1])/v[1]}')

echo
printf '==== ab_pair 结果（%s 对完整 / 请求 %s 对，跳过 %s）====\n' "${#DELTAS[@]}" "$PAIRS" "$INCOMPLETE"
printf '  %s: 中位 %.2f fps（臂内极差 %s%%，n=%s）\n' "$LABELA" "$MA" "$RA" "${#AS[@]}"
printf '  %s: 中位 %.2f fps（臂内极差 %s%%，n=%s）\n' "$LABELB" "$MB" "$RB" "${#BS[@]}"
printf '  配对差 (B-A): '; printf '%+.2f  ' "${DELTAS[@]}"; echo
awk -v md="$MD" -v ma="$MA" -v la="$LABELA" \
    'BEGIN{printf "  **MEDIAN PAIRED DELTA = %+.2f fps（%+.2f%% of %s）**\n", md, 100.0*md/ma, la}'
POS=0; for d in "${DELTAS[@]}"; do awk -v d="$d" 'BEGIN{exit !(d>0)}' && POS=$((POS + 1)); done
printf '  符号检验: %s / %s 对偏向 %s\n' "$POS" "${#DELTAS[@]}" "$LABELB"
echo "  ⚠️ 拿 |MEDIAN PAIRED DELTA| 与 A/A 底噪比 —— 底噪要用 -ExeA == -ExeB 另跑一批量"
echo "     （Linux 侧可先跑 scripts/aa_probe.sh；本机实测底噪约 1%）"
echo "  ⚠️ 正对照臂若不动，整批作废（教训 45：那是在量漂移，不是在量效应）"

if [ "${#DELTAS[@]}" -lt "$PAIRS" ]; then
    printf 'ab_pair: exit 2 —— %s 对里只完成 %s 对；降级的批次**不是**教训 45 要的 n>=5 证据。\n' "$PAIRS" "${#DELTAS[@]}" >&2
    printf '         引数字之前重跑。\n' >&2
    exit 2
fi
exit 0
