#!/usr/bin/env python3
"""refactor_move_check.py -- judge for "move-only" refactors (2026-09-28).

Why this exists
---------------
Splitting a 16k-line file into modules is only safe if a *move* cannot silently
become an *edit*.  Reviewing a 10 000-line diff by eye does not scale, and
`cargo test` cannot tell you that the moved code is the same code -- it only
tells you the behaviour you happen to test is unchanged.

So the contract is: **the moved text must be byte-identical to the text it was
moved from**.  This tool extracts line ranges from a git revision of the old
file and asserts each extracted block appears verbatim in the new file(s).

Usage
-----
  # one block: old lines 1726..12930 must appear verbatim somewhere in the new file
  python tools/refactor_move_check.py --rev HEAD --old src/engine/renderer.rs \
      --range 1726 12930 --in src/engine/renderer/pt.rs

  # several ranges, several targets (any range may live in any target)
  python tools/refactor_move_check.py --rev HEAD --old src/engine/game.rs \
      --range 7113 9000 --range 9001 10716 --in src/engine/game/tests.rs

  # list the `mod NAME { ... }` blocks inside a range, to plan a split
  python tools/refactor_move_check.py --rev HEAD --old src/engine/renderer.rs \
      --list-mod-blocks --range 13392 16265

Exit codes (repo lesson 46: a gate needs a third outcome)
  0 = every requested block was found verbatim
  1 = at least one block is missing or differs (the move edited something)
  2 = could not run (bad revision/path/range, git failed, no target files)
"""

import argparse
import re
import subprocess
import sys


def git_show(rev: str, path: str) -> str:
    try:
        out = subprocess.run(
            ["git", "show", f"{rev}:{path}"],
            capture_output=True,
            check=False,
        )
    except OSError as exc:  # git not on PATH
        print(f"cannot run git: {exc}")
        sys.exit(2)
    if out.returncode != 0:
        print(f"git show {rev}:{path} failed (rc={out.returncode})")
        tail = out.stderr.decode("utf-8", "replace").strip().splitlines()[-1:] or [""]
        print(f"  {tail[0]}")
        sys.exit(2)
    return out.stdout.decode("utf-8", "replace")


def read_text(path: str) -> str:
    try:
        with open(path, encoding="utf-8") as fh:
            return fh.read()
    except OSError as exc:
        print(f"cannot read {path}: {exc}")
        sys.exit(2)


def first_difference(a: str, b: str) -> str:
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return f"first difference at char {i}: {x!r} vs {y!r}"
    return f"identical for {min(len(a), len(b))} chars, length {len(a)} vs {len(b)}"


def list_mod_blocks(text: str, lo: int, hi: int):
    """Yield (name, start_line, end_line) for top-level `mod NAME {` blocks.

    The end of a block is the line before the NEXT top-level `mod` declaration
    (or `hi`).  Counting braces instead would be fooled by `{`/`}` inside string
    literals and comments -- measured 2026-09-28: `vk_failure_path_tests`
    (779 lines) was reported as 1023 lines because it swallowed its three
    following sibling modules.
    """
    lines = text.split("\n")
    starts = []
    for i in range(lo - 1, min(hi, len(lines))):
        m = re.match(r"^(?:#\[cfg\(test\)\]\s*)?(?:pub )?mod (\w+) \{", lines[i])
        if m:
            # an attribute line directly above belongs to the same block
            begin = i + 1
            if i - 1 >= lo - 1 and lines[i - 1].strip().startswith("#[cfg(test)]"):
                begin = i
            starts.append((m.group(1), begin))
    for k, (name, begin) in enumerate(starts):
        end = starts[k + 1][1] if k + 1 < len(starts) else hi
        yield name, begin + 1, end, end - begin


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rev", default="HEAD", help="git revision holding the OLD file")
    ap.add_argument("--old", required=True, help="path of the old file in that revision")
    ap.add_argument("--range", nargs=2, type=int, action="append", default=[],
                    metavar=("START", "END"), help="1-based inclusive line range (repeatable)")
    ap.add_argument("--in", dest="targets", action="append", default=[],
                    help="new file that should contain the moved text (repeatable)")
    ap.add_argument("--list-mod-blocks", action="store_true",
                    help="only print `mod NAME { ... }` blocks found in --range, then exit 0")
    ap.add_argument("--list-impl-blocks", action="store_true",
                    help="only print `impl Type ...` blocks found in --range, then exit 0")
    ap.add_argument("--allow-reextract", type=int, default=0, metavar="N",
                    help="tolerate up to N ranges that are NOT found (default 0: fail closed)")
    ap.add_argument("--normalize", action="append", default=[], metavar="OLD=NEW",
                    help="apply this substitution to the OLD block before matching, and report it. "
                         "⚠️ the value contains ASCII quotes, which PowerShell strips when passing to "
                         "a native command -- prefer --normalize-preset from a shell.")
    ap.add_argument("--normalize-preset", action="append", default=[],
                    metavar="NAME", help="named substitution, no shell quoting involved. "
                    "Known: include-str-renderer")
    ap.add_argument("--ranges-from", metavar="FILE",
                    help="read `name start end target [widen=*|widen=a,b]` lines from FILE instead of "
                         "--range/--in (single source of truth: the splitter script emits this file)")
    ap.add_argument("--list-test-blocks", action="store_true",
                    help="structure-driven listing: doc comments + #[cfg(test)] + `mod NAME {` "
                         "up to the next block (or the end of the range)")
    args = ap.parse_args()

    if args.ranges_from:
        try:
            rows = [l.split() for l in read_text(args.ranges_from).splitlines() if l.strip()]
        except SystemExit:
            return 2
        for row in rows:
            if row[0].startswith("#"):  # 记账行（例如被删掉的脚手架），不是搬运块
                print(" ".join(row))
                continue
            if len(row) not in (4, 5):
                print(f"bad range row in {args.ranges_from}: {row}")
                return 2
            name, a, b, target = row[0], int(row[1]), int(row[2]), row[3]
            args.range.append([a, b])
            args.targets.append(target)
            if len(row) == 5:
                if not row[4].startswith("widen="):
                    print(f"bad 5th column in {args.ranges_from}: {row}")
                    return 2
                spec = row[4][len("widen="):]
                if spec == "*":
                    # 搬运工具对整块方法统一加宽到 pub(crate)（Rust 的方法私有性不同于字段：
                    # 子模块 impl 里的私有 fn，父模块与测试模块都看不见）⇒ 显式记账
                    args.widen_all = True
                else:
                    for meth in spec.split(","):
                        if meth:
                            args.normalize.append(f"    fn {meth}(=    pub(crate) fn {meth}(")
                print(f"loaded range {a}..{b} -> {target}  [{row[4]}]")
            else:
                print(f"loaded range {a}..{b} -> {target}")
        print(f"loaded {len(rows)} range(s) from {args.ranges_from}")

    if not args.range:
        print("no --range given")
        return 2
    old_text = git_show(args.rev, args.old)

    if args.list_test_blocks:
        lines = old_text.split("\n")
        found = 0
        for lo, hi in args.range:
            starts = []
            for i in range(lo - 1, min(hi, len(lines))):
                if lines[i].strip() == "#[cfg(test)]":
                    begin = i
                    while begin - 1 >= lo - 1 and lines[begin - 1].lstrip().startswith("///"):
                        begin -= 1
                    starts.append((i + 1, begin))
            for k, (decl, begin) in enumerate(starts):
                end = (starts[k + 1][1] - 1) + 1 if k + 1 < len(starts) else hi
                m = re.match(r"mod (\w+)", lines[decl])
                print(f"{m.group(1) if m else '?'}\t{begin + 1}\t{end}\t{end - begin}")
                found += 1
        if found == 0:
            print("no `#[cfg(test)]` block found in the requested range(s)")
            return 1
        return 0

    if args.list_mod_blocks or args.list_impl_blocks:
        pat = (r"^(?:#\[cfg\(test\)\]\s*)?(?:pub )?mod (\w+) \{"
               if args.list_mod_blocks else r"^impl(?:<[^>]*>)? (\w+)")
        found = 0
        for lo, hi in args.range:
            lines = old_text.split("\n")
            starts = []
            for i in range(lo - 1, min(hi, len(lines))):
                m = re.match(pat, lines[i])
                if m:
                    begin = i
                    if i - 1 >= lo - 1 and lines[i - 1].strip().startswith("#[cfg(test)]"):
                        begin = i - 1
                    starts.append((m.group(1), begin))
            for k, (name, begin) in enumerate(starts):
                end = starts[k + 1][1] if k + 1 < len(starts) else hi
                print(f"{name}\t{begin + 1}\t{end}\t{end - begin}")
                found += 1
        if found == 0:
            print("no matching block found in the requested range(s)")
            return 1
        return 0

    if not args.targets:
        print("no --in target given")
        return 2
    targets = {p: read_text(p) for p in args.targets}

    missing = 0
    subs = []
    PRESETS = {
        # renderer 的自扫源码测试搬进 renderer/ 子目录后，相对路径必须多一层
        "include-str-renderer": ('include_str!("renderer.rs")', 'include_str!("../renderer.rs")'),
        "include-str-build-rs": ('include_str!("../../build.rs")', 'include_str!("../../../build.rs")'),
    }
    for name in args.normalize_preset:
        if name not in PRESETS:
            print(f"unknown preset {name!r}; known: {', '.join(sorted(PRESETS))}")
            return 2
        subs.append(PRESETS[name])
        print(f"preset {name}: {PRESETS[name][0]} -> {PRESETS[name][1]}")
    for spec in args.normalize:
        if "=" not in spec:
            print(f"--normalize needs OLD=NEW, got {spec!r}")
            return 2
        old_s, new_s = spec.split("=", 1)
        subs.append((old_s, new_s))
    for lo, hi in args.range:
        lines = old_text.split("\n")
        if lo < 1 or hi > len(lines) or lo > hi:
            print(f"range {lo}..{hi} outside 1..{len(lines)}")
            return 2
        block = "\n".join(lines[lo - 1:hi])
        applied = []
        if getattr(args, "widen_all", False):
            head = block.lstrip().split("\n", 1)[0]
            if re.match(r"^impl\s+[\w:<>, '\[\]]+\s+for\s+", head):
                # trait impl 的方法不能带可见性限定符 ⇒ 搬运工具对它跳过加宽，判据同样跳过
                applied.append(f"skip widen (trait impl: {head.strip()[:40]})")
            else:
                block, n = re.subn(r"^(    |)(?:pub(?:\([^)]*\))? )?((?:unsafe )?(?:const )?fn \w+)",
                                   lambda m: f"{m.group(1)}pub(crate) {m.group(2)}", block, flags=re.M)
                if n:
                    applied.append(f"widen {n} method(s) to pub(crate)")
        for old_s, new_s in subs:
            n = block.count(old_s)
            if n:
                block = block.replace(old_s, new_s)
                applied.append(f"{old_s} -> {new_s} x{n}")
        if not block.strip():
            print(f"range {lo}..{hi} is blank -- refusing to call that a move")
            missing += 1
            continue
        hit = [p for p, text in targets.items() if block in text]
        note = ("  [normalized: " + "; ".join(applied) + "]") if applied else ""
        if hit:
            print(f"OK   {lo}..{hi} ({hi - lo + 1} lines) found verbatim in {', '.join(hit)}{note}")
        else:
            missing += 1
            heads = [l for l in block.split("\n") if l.strip()][:1]
            print(f"FAIL {lo}..{hi} ({hi - lo + 1} lines) NOT found; block starts: {heads}{note}")
            for p, text in targets.items():
                anchor = block.split("\n")[0]
                if anchor and anchor in text:
                    idx = text.index(anchor)
                    got = text[idx:idx + len(block)]
                    print(f"     {p}: anchor present but block differs -> {first_difference(block, got)}")

    if missing > args.allow_reextract:
        print(f"RESULT: {missing} block(s) not byte-identical -- the move edited code")
        return 1
    print(f"RESULT: {len(args.range) - missing}/{len(args.range)} block(s) byte-identical")
    return 0


if __name__ == "__main__":
    sys.exit(main())
