#!/usr/bin/env python3
"""refactor_extract.py -- generic "move code out of a big file" driver (2026-09-28).

Contract (the whole point of this tool)
--------------------------------------
A refactor step must be a **pure move**.  This tool therefore:
  1. takes a spec of `target_file name start_line end_line` rows (1-based, inclusive),
  2. appends those exact line ranges to the target file(s) **byte for byte**,
  3. removes them from the source file,
  4. declares `mod <stem>;` in the source file for every target (unless already declared),
  5. **self-checks** that every range is byte-identical inside its target and that every
     target is declared, and writes `logs/extract_ranges.txt` so that
     `tools/refactor_move_check.py --ranges-from` can re-verify independently against git.

Exit codes: 0 = moved + all checks passed / 1 = a check failed / 2 = could not run.

Usage
-----
  python tools/refactor_extract.py --file src/engine/renderer.rs \
      --spec logs/extract_geometry.txt --ranges-out logs/extract_ranges.txt

Spec rows (whitespace separated, `#` starts a comment):
  target/relative/path.rs   block_name   start   end   [widen=name1,name2]

`start`/`end` are the lines of the ITEM itself; the tool extends `start` upwards over the
item's doc comments (`///`) and attributes (`#[...]`) so a moved item never leaves a dangling
doc comment behind and never loses its own documentation.

`widen=` lists methods whose visibility must be raised to `pub(crate)` **because Rust method
privacy is not like field privacy**: a private `fn` inside a child module's `impl` is invisible
to the parent and to sibling/test modules (fields would be visible).  Widening is a declared
deviation: it is written into the ranges file and printed by tools/refactor_move_check.py.
"""

import argparse
import io
import os
import re
import sys

HEADER = """// 由 {src} 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `{root}` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

"""

# 与 tools/refactor_move_check.py 里的同一条（可见性加宽）。覆盖三种形态：
#   缩进 4 空格的方法（impl 内）、顶格的自由函数、顶格的类型/常量定义（拆 consts 模块时要用）。
WIDEN_RE = re.compile(
    r"^(    |)(?:pub(?:\([^)]*\))? )?"
    r"((?:unsafe )?(?:const )?fn \w+|(?:struct|enum|union|static|type) \w+|const \w+)", re.M)


def read(path):
    with io.open(path, encoding="utf-8") as fh:
        return fh.read()


DOC_COM = ("///", "#[", "//!")


def head_start(lines, a):
    """1-based start of the item at `a`, including its doc-comment/attribute block.

    The block may contain BLANK LINES between the docs and the `fn` -- that is the actual style in
    this repo (measured 2026-09-28: `/// 设置画质…` / blank / `pub fn set_quality`).  Stopping at the
    blank line left the docs behind as a dangling doc comment (compiler: `expected item after doc
    comment`) and silently stripped documentation off the moved method.  A blank line only continues
    the walk when the line above it is itself doc/attribute -- otherwise it is a real separator.
    """
    i = a
    while i - 1 >= 1:
        s = lines[i - 2].lstrip()
        if s.startswith(DOC_COM):
            i -= 1
            continue
        if s == "" and i - 2 >= 1 and lines[i - 3].lstrip().startswith(DOC_COM):
            i -= 1
            continue
        break
    return i


def first_dangling_doc(text):
    """Line number of a `///` block that documents nothing; best-effort lint, reported only."""
    ls = text.split("\n")
    for i, l in enumerate(ls):
        if not l.lstrip().startswith("///"):
            continue
        j = i + 1
        while j < len(ls) and (ls[j].strip() == "" or ls[j].lstrip().startswith(DOC_COM)):
            j += 1
        if j >= len(ls) or ls[j].lstrip().startswith("}"):
            return i + 1
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--file", required=True, help="the big source file to move code out of")
    ap.add_argument("--spec", required=True, help="rows: target_path name start end")
    ap.add_argument("--ranges-out", default="logs/extract_ranges.txt")
    ap.add_argument("--impl-type", metavar="TYPE",
                    help="wrap the appended blocks in `impl TYPE { ... }` (required when moving "
                         "methods: a bare `fn` with `self` is a syntax error). The wrapper is "
                         "scaffolding -- the moved bytes themselves stay identical.")
    ap.add_argument("--raw", action="store_true",
                    help="move whole items verbatim with NO wrapper (e.g. an entire "
                         "`impl Drop for X` or `mod tests { ... }`), and do not trim a trailing `}`")
    ap.add_argument("--no-header-use", action="store_true",
                    help="omit `use super::*;` from the new file header (needed when the moved item "
                         "brings its own imports, e.g. a whole `mod tests { ... }` -- an unused "
                         "glob import is a warning, and the 0-warning gate is a hard red line)")
    ap.add_argument("--also-delete", nargs=2, type=int, action="append", default=[],
                    metavar=("START", "END"),
                    help="also delete this range from the source WITHOUT moving it (scaffolding such "
                         "as a module wrapper `mod X { ... }` whose contents were moved). Recorded as "
                         "a `#` comment row in the ranges file so the deletion stays visible.")
    ap.add_argument("--declare-in", metavar="FILE",
                    help="write the `mod X;` declarations into this file instead of --file. Needed when "
                         "the container is itself a child module: a new sibling module of the PARENT "
                         "must be declared in the parent's file (measured 2026-09-28: declaring "
                         "`tests_vk_side` inside `tests_vk.rs` made rustc look for "
                         "renderer/tests_vk/tests_vk_side.rs -> E0583).")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    try:
        text = read(args.file)
        spec_text = read(args.spec)
    except OSError as exc:
        print(f"cannot read input: {exc}")
        return 2
    # 陈旧范围表会骗过判据（实测：本次自检失败退出后，判据读到的还是上一轮的旧行号）。
    # 所以每次开跑先把输出文件删掉：失败就没有"证据文件"，绝不留下过期数据。
    if os.path.exists(args.ranges_out):
        os.remove(args.ranges_out)

    lines = text.split("\n")
    trailing_newline = text.endswith("\n")
    if trailing_newline:
        lines = lines[:-1]
    total = len(lines)

    rows = []
    for raw in spec_text.splitlines():
        row = raw.split("#", 1)[0].strip()
        if not row:
            continue
        parts = row.split()
        if len(parts) not in (4, 5):
            print(f"bad spec row (want: target name start end [widen=a,b]): {raw!r}")
            return 2
        target, name, a, b = parts[0], parts[1], int(parts[2]), int(parts[3])
        widen = []
        if len(parts) == 5:
            if not parts[4].startswith("widen="):
                print(f"bad 5th column (want widen=a,b): {raw!r}")
                return 2
            widen = [w for w in parts[4][len("widen="):].split(",") if w]
        if not (1 <= a <= b <= total):
            print(f"range {a}..{b} for {name} outside 1..{total}")
            return 2
        rows.append((target, name, a, b, widen))

    # 行号归一化（顺序不能反）：
    #   1) 削尾：区间尾部常含着「下一个条目」的文档注释/属性/空行（行号是按「下一个方法起点-1」推的）
    #      ⇒ 必须留给下一个条目，否则两个区间会重叠、注释会跟着搬错；
    #   2) 带注释：把条目自己的 /// 与 #[...] 一起带走，否则源文件里会留下悬空 doc 注释。
    norm = []
    for target, name, a, b, widen in rows:
        while b > a and (lines[b - 1].strip() == "" or lines[b - 1].lstrip().startswith(DOC_COM)):
            b -= 1
        # 兜底：块尾不允许是顶格的 `}`（那是 impl/模块的收尾括号，不是方法的一部分）。
        # 实测踩过一次：把 impl 的 `}` 一起搬走 ⇒ 源文件 unclosed delimiter，编译期才报。
        trimmed = 0
        while b > a and lines[b - 1] == "}" and not args.raw:
            b -= 1
            trimmed += 1
        if trimmed:
            print(f"note: trimmed {trimmed} closing brace(s) off the end of {name} "
                  f"(they belong to the enclosing impl/module)")
        # 把条目自己的文档/属性一起带走（允许文档与 fn 之间夹空行）
        a = head_start(lines, a)
        if not any(l.strip() for l in lines[a - 1:b]):
            print(f"range {a}..{b} for {name} is blank -- refusing to call that a move")
            return 2
        norm.append((target, name, a, b, widen))
    rows = norm

    if not rows:
        print("empty spec")
        return 2

    rows.sort(key=lambda r: r[2])
    for prev, cur in zip(rows, rows[1:]):
        if cur[2] <= prev[3]:
            print(f"overlapping ranges: {prev[1]} {prev[2]}..{prev[3]} and {cur[1]} {cur[2]}..{cur[3]}")
            return 2

    by_target = {}
    for target, name, a, b, widen in rows:
        by_target.setdefault(target, []).append((name, a, b, widen))

    root = os.path.splitext(os.path.basename(args.file))[0]
    src_label = args.file.replace("\\", "/")

    def apply_widen(block, names, label):
        # trait impl 里的方法不能带可见性限定符（`impl Trait for Type { fn default() … }`）：
        # 加了就是 E0449 visibility qualifiers are not permitted here（实测一次）。
        head = block.lstrip().split("\n", 1)[0]
        if re.match(r"^impl\s+[\w:<>, '\[\]]+\s+for\s+", head):
            print(f"note: {label} is a trait impl ({head.strip()[:50]}) -- skipping widen")
            return block
        if names == ["*"]:
            new_block, n = WIDEN_RE.subn(lambda m: f"{m.group(1)}pub(crate) {m.group(2)}", block)
            if n == 0:
                print(f"widen=* requested for {label} but no method was found")
                return None
            print(f"widened {n} method(s) to pub(crate) in {label}")
            return new_block
        for meth in names:
            old = f"    fn {meth}("
            new = f"    pub(crate) fn {meth}("
            if old not in block:
                print(f"widen requested for {meth} but `{old.strip()}` is not in {label}")
                return None
            block = block.replace(old, new, 1)
        return block

    blocks = {}
    for target, items in by_target.items():
        parts = []
        exists = os.path.exists(target)
        if not exists:
            header = HEADER.format(src=src_label, root=root)
            if args.no_header_use:
                header = header.replace("use super::*;\n\n", "")
            parts.append(header)
        if args.impl_type:
            parts.append(f"impl {args.impl_type} {{\n")
        for name, a, b, widen in items:
            block = "\n".join(lines[a - 1:b])
            if widen:
                block = apply_widen(block, widen, name)
                if block is None:
                    return 2
            blocks[(target, name)] = block
            parts.append(block)
            parts.append("\n")
        if args.impl_type:
            parts.append("}\n")
        body = "".join(parts)
        print(f"{'would write' if args.dry_run else 'wrote'} {target}: "
              f"{len(items)} block(s), {body.count(chr(10))} lines"
              f"{'' if exists else ' (new file)'}")
        if not args.dry_run:
            d = os.path.dirname(target)
            if d and not os.path.isdir(d):
                os.makedirs(d, exist_ok=True)
                print(f"created directory {d}")
            mode = "a" if exists else "w"
            with io.open(target, mode, encoding="utf-8", newline="\n") as fh:
                fh.write(body)

    if not args.raw and not args.impl_type:
        for (target, name), block in blocks.items():
            first = next((l for l in block.split("\n") if l.strip()), "")
            if re.match(r"^\s+(?:pub(?:\([^)]*\))? )?(?:unsafe )?(?:const )?fn \w+", first):
                print(f"refusing: block {name} looks like a METHOD (indented fn) but no --impl-type "
                      f"was given -- it would land as a bare module-level function and every caller "
                      f"would fail with E0599 (measured 2026-09-28). Pass --impl-type <Type>, or "
                      f"--raw if the block really is a whole impl/mod.")
                return 2

    # self-check #1: byte-identical inside the target
    bad = 0
    for (target, name), block in blocks.items():
        body = read(target) if not args.dry_run else block
        if block not in body:
            bad += 1
            print(f"SELF-CHECK FAIL {name}: not byte-identical in {target}")
    if bad:
        print("RESULT: self-check failed -- source left untouched")
        return 1

    # remove the ranges from the source (bottom-up) and declare the new modules。
    # 🔴 所有删除必须在**同一次降序扫描**里完成：先删脚手架再删搬运块会让第二次删除的各行号
    #    相对已漂移的文本（实测踩过：留下一条本该搬走的 `use super::*;`）。
    dropped = set()
    dele = [(a, b) for a, b in args.also_delete]
    for a, b in dele:
        if not (1 <= a <= b <= total):
            print(f"--also-delete range {a}..{b} outside 1..{total}")
            return 2
        for row in rows:
            if not (b < row[2] or a > row[3]):
                print(f"--also-delete {a}..{b} overlaps moved block {row[1]} {row[2]}..{row[3]}")
                return 2
    plan = [(a, b, f"scaffolding {a}..{b} ({b - a + 1} lines, deleted not moved)") for a, b in dele]
    plan += [(a, b, None) for _t, _n, a, b, _w in rows]
    for a, b, label in sorted(plan, key=lambda r: -r[0]):
        del lines[a - 1:b]
        if label:
            print("deleted " + label)
        else:
            dropped.add(b - a + 1)

    decls = []
    decl_file = args.declare_in or args.file
    decl_text = read(decl_file) if args.declare_in else text
    for target in by_target:
        stem = os.path.splitext(os.path.basename(target))[0]
        if re.search(rf"^\s*(?:#\[[^\]]*\]\s*)*(?:pub )?mod {re.escape(stem)}\s*;", decl_text, re.M):
            continue
        # tests 开头的模块只在测试构建里编译：漏掉 #[cfg(test)] 会让测试代码进入 release 构建
        cfg = "#[cfg(test)] " if stem.startswith("tests") else ""
        decls.append(f"{cfg}mod {stem};")
    if decls and args.declare_in:
        block = ["// 子模块（见 docs/refactor-plan.md）"] + decls + [""]
        dlines = decl_text.split("\n")
        marker = next((i for i, l in enumerate(dlines) if l.strip().startswith("#[cfg(test)]")), None)
        at = marker if marker is not None else len(dlines)
        dlines[at:at] = block
        with io.open(decl_file, "w", encoding="utf-8", newline="\n") as fh:
            fh.write("\n".join(dlines))
        print(f"declared {len(decls)} module(s) in {decl_file} (--declare-in)")
        decls = []  # 已在别处落盘
    if decls:
        marker = next((i for i, l in enumerate(lines) if l.strip() == "#[cfg(test)]"), None)
        block = ["// 子模块（见 docs/refactor-plan.md）"] + decls + [""]
        at = marker if marker is not None else len(lines)
        lines[at:at] = block

    if args.dry_run:
        print("dry run: source not written")
        return 0

    with io.open(args.file, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(lines) + ("\n" if trailing_newline else ""))

    # self-check #2: every target is declared in the file that owns the declarations
    # (--declare-in when the container is itself a child module, else the container)
    new_text = read(decl_file)
    missing = []
    for t in by_target:
        stem = os.path.splitext(os.path.basename(t))[0]
        if not re.search(rf"^\s*(?:#\[[^\]]*\]\s*)*(?:pub )?mod {re.escape(stem)}\s*;", new_text, re.M):
            missing.append(stem)
    if missing:
        print(f"SELF-CHECK FAIL: module(s) not declared in {decl_file}: {missing}")
        return 1

    with io.open(args.ranges_out, "w", encoding="utf-8", newline="\n") as fh:
        for a, b in args.also_delete:
            fh.write(f"# scaffolding deleted (not moved): {a}..{b}\n")
        for target, name, a, b, widen in rows:
            extra = (" widen=" + ",".join(widen)) if widen else ""
            fh.write(f"{name} {a} {b} {target}{extra}\n")
    lint = first_dangling_doc(read(args.file))
    if lint:
        print(f"WARNING: source line {lint} looks like a doc comment that documents nothing "
              f"(the compiler will reject it) -- check the block boundary rules")
    moved = sum(dropped) if len(dropped) == 1 else sum(b - a + 1 for _, _, a, b, _ in rows)
    print(f"source: {total} -> {len(lines)} lines (moved {moved}); "
          f"{len(decls)} module declaration(s) added; ranges -> {args.ranges_out}")
    print("SELF-CHECK: OK (every block byte-identical in its target; every target declared)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
