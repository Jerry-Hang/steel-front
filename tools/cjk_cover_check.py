"""Cheap pre-check for the CJK glyph guard (no cargo needed).

`source_cjk_codepoints_all_have_glyphs` (src/engine/font_cjk.rs) is the real gate, but
it only runs inside a full `cargo test --release`. When you add Chinese comments or log
strings, run this first: it reproduces the same membership test in a second, so a red
gate is caught before the 3-minute compile.

    python tools/extract_cjk_glyphs.py --scan   # regenerate tools/cjk_used_codepoints.txt
    python tools/cjk_cover_check.py
"""
import re
import sys

REPO = __file__.replace("\\", "/").rsplit("/", 2)[0]
USED = REPO + "/tools/cjk_used_codepoints.txt"
GLYPHS = REPO + "/src/engine/cjk_glyphs.rs"

used = set()
with open(USED, encoding="utf-8") as fh:
    for line in fh:
        line = line.strip()
        if line and not line.startswith("#"):
            used.add(chr(int(line, 16)))

with open(GLYPHS, encoding="utf-8") as fh:
    table = fh.read()
# entries look like: ('\u{4E2D}', [0x..., ...]),  -- one escaped char literal per entry
have = set(chr(int(h, 16)) for h in re.findall(r"\('\\u\{([0-9A-Fa-f]+)\}'", table))

missing = sorted(used - have)
print("used %d / table %d / missing %d" % (len(used), len(have), len(missing)))
if missing:
    print("missing chars: " + "".join(missing))
    print("missing hex  : " + " ".join("U+%04X" % ord(c) for c in missing[:40]))
    sys.exit(1)
print("CJK COVER: OK")
