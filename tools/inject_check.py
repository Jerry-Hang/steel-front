#!/usr/bin/env python3
"""批量验证"反向验证"声称：真的改坏、确认变红、再改回来。

**为什么必须自动化**：这一晚我三次栽在"注入没落盘"上 ——
字符串替换的缩进差两个空格、删掉格式参数导致编译失败、`to_cols_array_2d`
展平恰好等价。每次都得出"这条声称是假的"的错误结论。

所以本工具在跑测试**之前**先做两件事：
  1. 替换必须真的改变文件（`before != after`），否则直接判 **NO-OP**（不算验证过）；
  2. `git diff --stat` 必须非空（双保险，防止我改错了文件）。

**三种结局**（本仓教训 46）：
  0 = 全部按声称变红（通过）
  1 = 有声称没变红（**判据或声称有问题**，需要人看）
  2 = 根本没跑成（清单不存在 / cargo 起不来 / 全程被杀）

清单格式（`tools/injections.txt`，`#` 开头是注释）：
```
测试名 | 文件 | 原文（必须唯一出现） | 替换成
```
字段用 ` | ` 分隔；`原`/`替换` 里不要出现 ` | `。
"""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# `--manifest` 可覆盖（**为了能测“跑不成”与“无效注入”这两态**）
MANIFEST = ROOT / "tools" / "injections.txt"
REPORT = ROOT / "tools" / "injections_report.txt"


def load(manifest):
    entries = []
    for raw in manifest.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = [p.strip() for p in line.split(" | ")]
        # 第 5 段可选：`all` = 替换全部出现（用于"同一行在实现里出现多次"的注入）
        if len(parts) == 5 and parts[4] == "all":
            parts = parts[:4] + ["all"]
        elif len(parts) == 4:
            parts = parts + ["one"]
        else:
            print(f"inject_check: 清单格式错（需要 4 段或 5 段含 all，实际 {len(parts)}）：{raw[:80]}")
            return None
        entries.append(parts)
    return entries


def run_test(name):
    """跑单个测试，返回 (是否通过, 摘要)。"""
    p = subprocess.run(
        ["cargo", "test", "--release", "--", name],
        cwd=ROOT, capture_output=True, text=True, timeout=1800,
    )
    out = p.stdout + p.stderr
    passed = "test result: ok." in out
    # 编译失败也算"没通过"，但要与"断言失败"区分开
    compiled = "error[E" not in out and "could not compile" not in out
    summary = ""
    for line in out.splitlines():
        if "test result:" in line:
            summary = line.strip()
            break
    if not compiled:
        summary = "编译失败"
    return (passed and compiled), summary


def main():
    global MANIFEST
    if "--manifest" in sys.argv:
        MANIFEST = Path(sys.argv[sys.argv.index("--manifest") + 1])
    if not MANIFEST.exists():
        print(f"inject_check: 跑不成 —— 清单不存在 {MANIFEST}")
        return 2
    entries = load(MANIFEST)
    if entries is None:
        return 2
    if not entries:
        print("inject_check: 跑不成 —— 清单里没有一条（不是“通过”）")
        return 2

    only = None
    if "--only" in sys.argv:
        only = sys.argv[sys.argv.index("--only") + 1]

    results = []
    for name, rel, find, repl, mode in entries:
        if only and only not in name:
            continue
        path = ROOT / rel
        before = path.read_text(encoding="utf-8")
        cnt = before.count(find)
        if cnt == 0 or (mode != "all" and cnt != 1):
            results.append(
                (name, "SKIP", f"原文出现 {cnt} 次（one 模式要求恰好 1 次，all 模式要求 ≥1）")
            )
            continue
        after = before.replace(find, repl) if mode == "all" else before.replace(find, repl, 1)
        if after == before:
            results.append((name, "NO-OP", "替换没改变文件（这就是“无效注入”）"))
            continue
        path.write_text(after, encoding="utf-8")
        try:
            # 双保险：文件确实变了
            diff = subprocess.run(
                ["git", "diff", "--stat", rel], cwd=ROOT, capture_output=True, text=True
            ).stdout.strip()
            if not diff:
                results.append((name, "NO-OP", "git diff 为空"))
                continue
            passed, summary = run_test(name)
            if summary == "编译失败":
                # 🔴 **编译失败不是"变红"**：判据根本没跑。当成 RED 会把
                # "注入写坏了" 记成 "判据有效" —— 与假账等价。
                results.append((name, "BROKEN", "注入导致编译失败（判据没跑，不能算验证过）"))
            elif passed:
                results.append((name, "NOT-RED", f"注入后仍然通过 —— {summary}"))
            else:
                results.append((name, "RED", summary))
        finally:
            path.write_text(before, encoding="utf-8")

    lines = []
    for name, status, note in results:
        lines.append(f"{status:8s} {name} — {note}")
    text = "\n".join(lines)
    print(text)
    REPORT.write_text(text + "\n", encoding="utf-8")

    bad = [r for r in results if r[1] in ("NOT-RED", "SKIP", "NO-OP", "BROKEN")]
    red = [r for r in results if r[1] == "RED"]
    print(f"\ninject_check: {len(red)} 条确认变红 / {len(results)} 条；"
          f"{len(bad)} 条需人看（明细见 {REPORT.name}）")
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
