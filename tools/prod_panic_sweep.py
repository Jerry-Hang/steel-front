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


def default_paths():
    """src/ 下全部 .rs（含 src/engine/ 与任何新加的子目录）。"""
    root = pathlib.Path("src")
    if not root.is_dir():
        raise SystemExit("必须在仓库根目录下运行（找不到 src/）")
    return sorted(str(p).replace("\\", "/") for p in root.rglob("*.rs"))

PAT = re.compile(r"\.unwrap\(\)|\.expect\(|panic!\(|unreachable!\(")


def main(paths):
    total = 0
    for path in paths:
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


if __name__ == "__main__":
    paths = sys.argv[1:] or default_paths()
    print("scanned %d .rs files under src/" % len(paths))
    main(paths)
