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



# ───────────────────────────────────────────────────────────────────────────
# 从"声称文本"里**编译**注入：很多声称本身就写着「把 X 改成 Y」。
#
# 为什么值得做：欠账 128 条，逐条手写注入是瓶颈。声称文本里已经写明了击穿方式的，
# 机器可以直接试；只在"原文出现次数恰好 1"时才动手（其余跳过并报告，不猜）。
# ───────────────────────────────────────────────────────────────────────────
import re as _re
import pathlib as _pathlib

# 「把 `X` 改成 `Y`」「把 `X` 写成 `Y`」「`X` 改为 `Y`」
CLAIM_PATTERNS = [
    _re.compile(r"把\s*`([^`]+)`\s*(?:改成|写成|改为|换成)\s*`([^`]+)`"),
    _re.compile(r"`([^`]+)`\s*(?:改成|写成|改为|换成)\s*`([^`]+)`"),
]


def proposals_from_claims():
    """扫全部声称 → [(测试名, 文件, 原文, 替换)]。

    🔴 归属用 `judge_audit.py` 的 `test_functions()`：声称写在**它自己的测试上方**，
    "从声称往上找最近的 fn"会**指到上一个测试**（第一版就这么错的，9 条里 8 条张冠李戴）。
    只收"原文在该文件里**恰好出现 1 次**"的，且跳过含 `...` 的占位写法。
    """
    import importlib.util

    root = _pathlib.Path(__file__).resolve().parent.parent
    spec = importlib.util.spec_from_file_location("ja", root / "tools" / "judge_audit.py")
    ja = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ja)
    out = []
    for f in sorted((root / "src").rglob("*.rs")):
        text = f.read_text(encoding="utf-8", errors="replace")
        if "反向验证" not in text:
            continue
        rel = str(f.relative_to(root))
        for name, _line, _body, doc in ja.test_functions(text):
            if "反向验证" not in doc:
                continue
            for pat in CLAIM_PATTERNS:
                m = pat.search(doc)
                if not m:
                    continue
                find, repl = m.group(1), m.group(2)
                if "..." in find or "..." in repl:
                    break
                if text.count(find) != 1:
                    break
                out.append((name, rel, find, repl))
                break
    return out


def propose():
    props = proposals_from_claims()
    for name, rel, find, repl in props:
        print(f"{name}\t{rel}\t{find}\t{repl}")
    print(f"PROPOSALS: {len(props)} 条可从声称文本自动编译")
    return 0


def main():
    global MANIFEST
    if "--manifest" in sys.argv:
        MANIFEST = Path(sys.argv[sys.argv.index("--manifest") + 1])
    if "--propose" in sys.argv:
        return propose()

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
        # 🔴 **注入目标是注释 ⇒ 不算验证**：这个形态已出现三次
        # （`npcs.len() >= 0`、`tx.send`、`m *= local`）。替换"落盘"了但行为没变，
        # 于是得出"判据失效"的错误结论。必须在跑测试前拦下来。
        hit_line = next((l for l in before.splitlines() if find in l), "")
        if hit_line.strip().startswith("//"):
            results.append(
                (name, "COMMENT-ONLY", f"注入目标是注释行：{hit_line.strip()[:60]}")
            )
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

    bad = [
        r for r in results if r[1] in ("NOT-RED", "SKIP", "NO-OP", "BROKEN", "COMMENT-ONLY")
    ]
    red = [r for r in results if r[1] == "RED"]
    print(f"\ninject_check: {len(red)} 条确认变红 / {len(results)} 条；"
          f"{len(bad)} 条需人看（明细见 {REPORT.name}）")
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
