#!/usr/bin/env python3
"""move_verify_normalized.py —— "移动 + 可见性" 重构的判据（2026-11-10）。

为什么需要它
------------
`tools/refactor_move_check.py` 要求**逐字节一致**。但把一段代码从 `lib.rs` 搬进子模块时，
几乎必然要给符号加 `pub(crate)` 才能被 `pub(crate) use 模块::*;` 再导出 —— 那就被它判成
"编辑了代码"。这不是工具错，是它表达不了这类改动。

本工具补这一环：**只允许一类差异**（`pub(crate) ` 前缀），其余必须逐字节一致。
如果搬移时顺手改了逻辑/常量/路径，这里同样会红 —— 它不是放松，而是把"允许的改动"缩小到一条。

用法
----
    python tools/move_verify_normalized.py --rev HEAD --old src/lib.rs \
        --range 227 243 --range 414 452 --in src/app_gun.rs

退出码（仓库教训 46：闸门要有"没跑成"这一态）
    0 = 每个块在归一化后都逐字节命中
    1 = 有块不一致（打印首个差异位置）
    2 = 没跑成（取不到旧版本 / 目标文件不存在 / 参数不全）
"""

import argparse
import subprocess
import sys


def sh(*args):
    return subprocess.run(args, capture_output=True, text=True)


def normalize(text: str) -> str:
    """抹掉**唯一允许的差异**：可见性前缀。"""
    return text.replace("pub(crate) ", "")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rev", default="HEAD")
    ap.add_argument("--old", required=True)
    ap.add_argument("--range", nargs=2, action="append", required=True,
                    metavar=("START", "END"), help="1-based，含首含尾")
    ap.add_argument("--in", dest="target", required=True, action="append")
    a = ap.parse_args()

    old = sh("git", "show", f"{a.rev}:{a.old}")
    if old.returncode != 0:
        print(f"没跑成：取不到 {a.rev}:{a.old}", file=sys.stderr)
        return 2
    old_lines = old.stdout.split("\n")

    targets = []
    for t in a.target:
        try:
            targets.append(normalize(open(t, encoding="utf-8").read()))
        except OSError as e:
            print(f"没跑成：读不到 {t}: {e}", file=sys.stderr)
            return 2

    ok = True
    for start, end in [(int(s), int(e)) for s, e in a.range]:
        if start < 1 or end < start or end > len(old_lines):
            print(f"没跑成：范围 {start}..{end} 越界（旧文件 {len(old_lines)} 行）", file=sys.stderr)
            return 2
        block = normalize("\n".join(old_lines[start - 1:end]).strip())
        if not block:
            print(f"没跑成：范围 {start}..{end} 是空的", file=sys.stderr)
            return 2
        if any(block in t for t in targets):
            print(f"  OK   {start}..{end}（{block.count(chr(10)) + 1} 行，归一化后逐字节命中）")
        else:
            ok = False
            print(f"  FAIL {start}..{end} 未命中；块首行：{block.splitlines()[0][:70]}")
    print("  RESULT: " + ("全部命中（差异仅为 pub(crate) 前缀）" if ok else "有块不一致 —— 搬移时改动了代码"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
