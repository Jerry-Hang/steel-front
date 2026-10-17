"""Report unwrap()/expect()/panic!/unreachable! that live in PRODUCTION code.

Test bodies are excluded by cutting the file at the `#[cfg(test)]` module (this repo puts tests
at the bottom of each file). Without that cut the counts are meaningless -- most matches are
assertions in tests, which is exactly why an "N unwraps" number by itself has never found
anything here.

Usage: python tools/prod_panic_sweep.py [file ...]      (default: every .rs under src/)

🔴 2026-09-26：默认列表以前是**写死的 16 个文件**，而 `src/` 下有 33 个 ——
`net.rs`（**解析外部 UDP 报文的那一个**）、`renderer.rs`、`weapons.rs`、`geom.rs`、
`simd.rs`、`lighting.rs`、`ray_tracer.rs`、`ai_command.rs` 等 17 个文件**从来没被扫过**，
而本轮记录里写的是"全仓普查"。⇒ 改成**默认扫 `src/` 下全部 .rs**：
判据必须来自"扫了什么"，不能来自"我以为扫了什么"（教训 27 的同形）。
"""
import io
import pathlib
import re
import sys


def is_test_only(path):
    """整个文件就是测试代码的那些 —— 靠**路径**判断，不靠文件内的 `#[cfg(test)]`。

    🔴 2026-10-17 修：原实现只在文件内部找 `#[cfg(test)]` 来切割，
    但 `src/engine/game/tests/*.rs`（父模块 `game.rs:1095` 的 `#[cfg(test)] mod tests;` 引入）
    与 `src/engine/renderer/tests_*.rs`（`renderer.rs:1265-1273` 的 `#[cfg(test)] mod tests_*;`）
    **文件内部根本没有那个标记** ⇒ 整个测试文件被当成生产代码，44 条命中里 34 条是断言。
    ⇒ 一个常报 34 条假警报的扫描器等于没人看（教训 26）。
    """
    parts = pathlib.Path(path).parts
    if "tests" in parts[:-1]:
        return True
    return pathlib.Path(path).name.startswith("tests_")


def default_paths():
    """src/ 下全部 .rs（含 src/engine/ 与任何新加的子目录）。"""
    root = pathlib.Path("src")
    if not root.is_dir():
        raise SystemExit("必须在仓库根目录下运行（找不到 src/）")
    return sorted(str(p).replace("\\", "/") for p in root.rglob("*.rs"))

PAT = re.compile(r"\.unwrap\(\)|\.expect\(|panic!\(|unreachable!\(")


def main(paths):
    total = 0
    skipped = 0
    for path in paths:
        if is_test_only(path):
            skipped += 1
            continue
        try:
            text = io.open(path, encoding="utf-8").read()
        except OSError:
            continue
        cut = text.find("#[cfg(test)]")
        prod = text if cut < 0 else text[:cut]
        hits = []
        for i, line in enumerate(prod.split("\n"), 1):
            stripped = line.strip()
            if stripped.startswith("//") or stripped.startswith("///"):
                continue
            if PAT.search(line):
                hits.append((i, stripped))
        if hits:
            print("%s: %d production site(s)" % (path, len(hits)))
            for i, line in hits:
                print("  L%-5d %s" % (i, line[:150]))
            total += len(hits)
    print("total production sites: %d" % total)
    # 教训 46：审计工具必须说清「扫了什么」—— 排除多少也要报出来
    print("excluded %d test-only file(s) by path" % skipped)


if __name__ == "__main__":
    paths = sys.argv[1:] or default_paths()
    print("scanned %d .rs files under src/" % len(paths))
    main(paths)
