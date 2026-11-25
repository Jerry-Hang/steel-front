#!/usr/bin/env python3
"""判据自检：抓「判据自己失效」的几种形态（2026-11-25 一整晚踩了十几次）。

**为什么需要它**：那一晚真正贵的事故不是代码 bug，而是**判据通过了但它测的不是
我以为的东西**：恒真断言、测错对象、注释声称"注入会红"而实际不会。这些只写在
PROGRESS 里没人会读，下一轮照样犯。

**三种结局（本仓教训 46）**：
  0 = 扫过，无发现
  1 = 有发现（逐条列出）
  2 = **根本没扫成**（目录不存在 / 没找到任何测试）—— 不是"通过"

检查项：
  A. 声称"反向验证"的测试里，**一条断言都没有**（注释在说谎的直接形态）。
  B. 恒真/近似恒真的断言：`assert!(true`、`>= 0` / `<= 0` 与 `.len()` 的组合。
  C. 测试函数体是空的（只有签名）。

用法：
  python3 tools/judge_audit.py            # 扫 src/
  python3 tools/judge_audit.py --verbose  # 连带打印通过项计数
"""

import os
import re
import sys
from pathlib import Path

# 扫哪个仓库：默认本文件所在仓库；`JUDGE_AUDIT_ROOT` 可覆盖（**为了能测"扫不成"那一态**）
ROOT = Path(os.environ.get("JUDGE_AUDIT_ROOT") or Path(__file__).resolve().parent.parent)
SRC = ROOT / "src"

# 断言形态：任一出现即算"有断言"
ASSERT_PAT = re.compile(
    r"\b(assert!|assert_eq!|assert_ne!|assert_matches!|expect_err|should_panic|panic!|unreachable!)\s*[(!]"
)
# 恒真形态
TRIVIAL_PAT = [
    (re.compile(r"assert!\(\s*true\s*[,)]"), "assert!(true)"),
    (re.compile(r"\.len\(\)\s*>=\s*0\b"), ".len() >= 0（无符号数恒真）"),
    (re.compile(r"\.len\(\)\s*<\s*0\b"), ".len() < 0（无符号数恒假）"),

]


def strip_line_comments(body):
    """去掉行注释（`//` 之后），避免把"注释里提到的反模式"当成代码命中。

    实测过一次：一条注释写着"原来这里挂着 `npcs.len() >= 0`，已删掉"，
    工具却把它报成恒真断言 —— 那会让真发现被噪声淹没。
    """
    out = []
    for line in body.splitlines():
        i = line.find("//")
        out.append(line if i < 0 else line[:i])
    return "\n".join(out)


def test_functions(text):
    """切出 `#[test]` 标注的测试：返回 [(名字, 起始行, 函数体文本, 文档文本)]。

    **不数花括号**。理由：字符串（尤其测试夹具里的多行 JSON `r#"…"#`）与字符字面量
    里的花括号会让配平彻底失效 —— 这个工具前四版都栽在这上面
    （把明明有 `assert_eq!` 的测试判成"没有断言"，或把 55 个测试整段漏掉）。

    规则改成：从 `#[test]` 起到**与 `fn` 同缩进的 `}`** 为止 —— 这是 rustfmt 的
    闭合括号对齐约定，对任意嵌套层数都成立（写死"4 空格"会在嵌套 mod 里漏掉测试：
    实测漏了 34 个文件的 `#[test]`）。
    """
    out = []
    lines = text.splitlines()
    i = 0
    n = len(lines)
    while i < n:
        if lines[i].strip() != "#[test]":
            i += 1
            continue
        # 名字
        j = i
        while j < n and "fn " not in lines[j]:
            j += 1
        if j >= n:
            break
        m = re.search(r"fn\s+(\w+)", lines[j])
        name = m.group(1) if m else "<anon>"
        # 文档：往上收集连续的注释行
        start = i
        while start > 0 and (lines[start - 1].strip().startswith("///")
                             or lines[start - 1].strip().startswith("//")):
            start -= 1
        # 结束：下一个**与 fn 同缩进的** `}`（rustfmt 把闭合括号对齐到 fn）
        indent = len(lines[j]) - len(lines[j].lstrip())
        closer = " " * indent + "}"
        k = j
        while k < n and lines[k] != closer:
            k += 1
        body = "\n".join(lines[j:k])
        doc = "\n".join(lines[start:i])
        out.append((name, start + 1, body, doc))
        i = k + 1
    return out


SELF_TEST_CASES = [
    # (题目, 片段, 期望命中)
    ("注释声称反向验证但无断言",
     '/// 反向验证：删掉 x ⇒ 红。\n#[test]\nfn t() {\n    let _ = 1;\n}\n', "A"),
    ("恒真断言",
     '#[test]\nfn t() {\n    let v: Vec<u8> = vec![];\n    assert!(v.len() >= 0);\n}\n', "B"),
    ("正常的带断言测试（不该命中）",
     '/// 反向验证：删掉 x ⇒ 红。\n#[test]\nfn t() {\n    assert_eq!(1 + 1, 2);\n}\n', None),
    ("恒真写法只出现在注释里（不该命中）",
     '#[test]\nfn t() {\n    // 原来这里是 len() >= 0，已删\n    assert!(true || false);\n}\n',
     None),
]


def self_test():
    """证明这个工具**能报出来**。永远只报 OK 的检查器等于没有检查器。"""
    ok = True
    for title, src, want in SELF_TEST_CASES:
        got = []
        for name, line, body, doc in test_functions(src):
            claims = "反向验证" in doc or "反向验证" in body
            if claims and not ASSERT_PAT.search(body):
                got.append("A")
            for pat, _why in TRIVIAL_PAT:
                if pat.search(strip_line_comments(body)):
                    got.append("B")
        hit = want in got if want else not got
        print(f"  {'OK ' if hit else 'FAIL'} {title}（期望 {want}，实际 {sorted(set(got)) or '无'}）")
        ok = ok and hit
    print("SELF TEST:", "通过" if ok else "失败")
    return 0 if ok else 1


def main():
    if "--self-test" in sys.argv:
        return self_test()
    verbose = "--verbose" in sys.argv
    if not SRC.is_dir():
        print(f"judge_audit: 扫不成 —— 目录不存在 {SRC}")
        return 2
    files = sorted(SRC.rglob("*.rs"))
    if not files:
        print("judge_audit: 扫不成 —— src/ 下没有任何 .rs")
        return 2

    total_tests = 0
    with_claim = 0
    findings = []
    for f in files:
        text = f.read_text(encoding="utf-8", errors="replace")
        if "#[test]" not in text:
            continue
        for name, line, body, doc in test_functions(text):
            total_tests += 1
            rel = f.relative_to(ROOT)
            claims = "反向验证" in doc or "反向验证" in body
            if claims:
                with_claim += 1
            has_assert = bool(ASSERT_PAT.search(body))
            if claims and not has_assert:
                findings.append(
                    f"A {rel}:{line} {name} —— 注释声称“反向验证”，函数里一条断言都没有"
                )
            for pat, why in TRIVIAL_PAT:
                if pat.search(strip_line_comments(body)):
                    findings.append(f"B {rel}:{line} {name} —— 恒真断言：{why}")
            stripped = body.strip()
            if stripped.endswith("{}") or stripped.endswith("{ }"):
                findings.append(f"C {rel}:{line} {name} —— 函数体是空的")

    if total_tests == 0:
        # 扫到 0 个测试 ≠ 通过
        print("judge_audit: 扫不成 —— 一个 #[test] 也没找到（不是“通过”）")
        return 2

    if verbose:
        print(f"judge_audit: 扫了 {len(files)} 个文件、{total_tests} 个测试，"
              f"其中 {with_claim} 个声称做过反向验证")
    if findings:
        for x in findings:
            print(x)
        print(f"JUDGE AUDIT: {len(findings)} 处需要复核")
        return 1
    print(f"JUDGE AUDIT: OK（{total_tests} 个测试，{with_claim} 个声称反向验证，无前述形态）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
