"""Cheap pre-check for the CJK glyph guard (no cargo needed).

`source_cjk_codepoints_all_have_glyphs` (src/engine/font_cjk.rs) is the real gate, but it
only runs inside a full `cargo test --release`, i.e. ~40 s of compile later. Run this first
after touching any Chinese comment or log string in src/: it reproduces the gate's two
checks in about a second.

    python tools/cjk_cover_check.py

WHY THIS SCANS src/ INSTEAD OF READING tools/cjk_used_codepoints.txt
-------------------------------------------------------------------
The first version of this tool compared the *stored list* against the glyph table. That
list is a generated artefact, so the check could only ever report "list and table agree" --
it printed `CJK COVER: OK` on 2026-09-26 while src/perf_log.rs had just gained two code
points (U+674E, U+6234) that had no glyph, and the real gate went red one compile later.
⇒ A pre-check that trusts a generated file cannot catch the thing that actually breaks.
This version scans the source tree live, exactly like the test does (lesson 36: a tool that
cannot fail is not a tool).

Checks performed (both mirror src/engine/font_cjk.rs):
  1. every CJK code point in src/**.rs (skipping the generated table and font_cjk.rs itself)
     must have a glyph entry;
  2. the glyph table must have exactly as many entries as the stored `--scan` list, because
     the gate asserts that too (a stale list is a red gate even when no character is missing).

Exit code 0 = the gate should pass; 1 = it will fail, and the offending characters and files
are printed here first.
"""
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SRC = REPO / "src"
GLYPHS = SRC / "engine" / "cjk_glyphs.rs"
STORED_LIST = REPO / "tools" / "cjk_used_codepoints.txt"

# Same ranges as tools/extract_cjk_glyphs.py::RANGES -- one predicate, both sides.
RANGES = ((0x3000, 0x303F), (0x3040, 0x30FF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF), (0xFF00, 0xFFEF))

# cjk_glyphs.rs is the generated table itself; font_cjk.rs deliberately mentions code points
# that are NOT in the table (its assertions use them as samples).
SKIP_FOR_GATE = {"cjk_glyphs.rs", "font_cjk.rs"}
SKIP_FOR_LIST = {"cjk_glyphs.rs"}  # what tools/extract_cjk_glyphs.py --scan excludes


def is_cjk(cp):
    return any(lo <= cp <= hi for lo, hi in RANGES)


def scan_source(skip):
    """code point -> sorted list of source files that mention it."""
    found = {}
    files = 0
    for path in sorted(SRC.rglob("*.rs")):
        if path.name in skip:
            continue
        text = path.read_text(encoding="utf-8")
        files += 1
        for ch in text:
            cp = ord(ch)
            if is_cjk(cp):
                found.setdefault(cp, set()).add(str(path.relative_to(REPO)))
    return found, files


def read_table():
    pat = re.compile(r"^\s*\('\\u\{([0-9A-Fa-f]+)\}'")
    have = {}
    for line in GLYPHS.read_text(encoding="utf-8").splitlines():
        m = pat.match(line)
        if m:
            have[int(m.group(1), 16)] = True
    return have


def read_stored_list():
    cps = []
    if not STORED_LIST.exists():
        return cps
    for line in STORED_LIST.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            cps.append(int(line, 16))
    return cps


def main():
    gate_used, files = scan_source(SKIP_FOR_GATE)
    if files < 10:
        print("CJK COVER: FAIL -- only %d .rs files scanned, the path is wrong" % files)
        return 1
    list_used, _ = scan_source(SKIP_FOR_LIST)
    table = read_table()
    stored = read_stored_list()

    missing = sorted(cp for cp in gate_used if cp not in table)
    print("scanned %d .rs files" % files)
    print("source uses  %5d CJK code points (gate filter)" % len(gate_used))
    print("table has    %5d entries" % len(table))
    print("stored list  %5d entries (must equal the table: the gate asserts it)" % len(stored))

    failed = False
    if missing:
        failed = True
        print("MISSING GLYPHS: %d -- the HUD would render these as blank" % len(missing))
        for cp in missing[:40]:
            where = ", ".join(sorted(gate_used[cp])[:3])
            print("  '%s' (U+%04X)  in %s" % (chr(cp), cp, where))
        print("  fix: rewrite the text to use characters that already have glyphs")
        print("       (the source font is not in the repo, so the table CANNOT be rebuilt)")
    if len(stored) != len(table):
        failed = True
        only_table = sorted(set(table) - set(stored))
        only_stored = sorted(set(stored) - set(table))
        print("STALE LIST: table %d entries vs stored list %d" % (len(table), len(stored)))
        print("  in table only   : %s" % " ".join("U+%04X" % c for c in only_table[:20]))
        print("  in stored only  : %s" % " ".join("U+%04X" % c for c in only_stored[:20]))
        print("  fix: python tools/extract_cjk_glyphs.py --scan  -- then regenerate the table")
    if failed:
        print("CJK COVER: FAIL")
        return 1
    print("CJK COVER: OK (the glyph gate should pass)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
