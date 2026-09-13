#!/usr/bin/env python3
"""Extract the CJK bitmap glyph table used by Steel Front's HUD.

WHY THIS TOOL EXISTS
--------------------
`src/engine/cjk_glyphs.rs` previously carried ~21,500 baked glyph bitmaps with
**no generator in the repository** -- the file's own header recorded that it was
"extracted once at build time" from a Windows system font. That left two
problems:

1. **Provenance was unauditable.** Nobody could reproduce, or re-source, the
   data, because the code that produced it was never committed.
2. **92.6% of it was dead weight.** A scan of `src/**/*.rs` (excluding the
   glyph file itself) finds **1,580** distinct CJK code points in use, against
   21,486 entries in the table -- so 19,906 glyphs were shipped that nothing
   referenced.

This script fixes (1) by making the extraction reproducible, and gives you the
lever for (2) via `--trim`, which keeps only the code points the source actually
uses.

WHAT IT DOES NOT DO
-------------------
**It does not, by itself, resolve the font licensing problem.** If you point it
at `simsun.ttc` you get exactly the same legally-encumbered bitmaps, just
smaller. See `THIRD-PARTY-NOTICES.md` section 3. The point of this script is to
make it *possible* to point it at a redistributable font instead -- and to make
that swap a one-command operation.

USAGE
-----
    # Report what the source actually uses, and what the table currently holds.
    # Writes the used-code-point list to tools/cjk_used_codepoints.txt
    python tools/extract_cjk_glyphs.py --scan

    # Re-extract every used code point from a specific font and rewrite the table.
    python tools/extract_cjk_glyphs.py --font path/to/NotoSansCJKsc-Regular.otf

    # Same, but keep the full CJK range instead of trimming to what is used.
    python tools/extract_cjk_glyphs.py --font FONT --all

FONT REQUIREMENTS
-----------------
* Must cover every code point in `tools/cjk_used_codepoints.txt`. The script
  reports any that are missing and refuses to write a table that would make the
  HUD render blanks.
* Should be licensed for redistribution *and* for embedding a derived bitmap.
  Good candidates: **Noto Sans CJK SC** or **Source Han Sans** (SIL OFL 1.1),
  **WenQuanYi Zen Hei** (GPL-2.0 + font exception). OFL-1.1 is the cleanest fit
  for an AGPL project, because it imposes no copyleft obligation on the
  software that embeds the glyphs.

DEPENDENCIES
------------
`fontTools` (pip install fonttools) and `Pillow` (pip install pillow). Neither
is a build dependency of the game -- this is a maintainer tool, run on demand.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
GLYPH_RS = REPO / "src" / "engine" / "cjk_glyphs.rs"
USED_TXT = REPO / "tools" / "cjk_used_codepoints.txt"

# The HUD rasterises at 12x12 and indexes rows as u16 bitmasks (bit 11 = leftmost
# pixel). These two constants are a hard contract with `ui.rs` -- changing either
# without changing the renderer produces garbled or blank text, not an error.
CELL = 12
ROW_MASK_BITS = 12

# Code-point ranges considered "CJK" for the purposes of this tool: CJK symbols
# and punctuation, Hiragana/Katakana, CJK Unified Ideographs (+ Ext A), the
# compatibility block, and the fullwidth/halfwidth forms.
RANGES = (
    (0x3000, 0x303F),
    (0x3040, 0x30FF),
    (0x4E00, 0x9FFF),
    (0xF900, 0xFAFF),
    (0xFF00, 0xFFEF),
)


def is_cjk(cp: int) -> bool:
    return any(lo <= cp <= hi for lo, hi in RANGES)


def scan_source() -> set[int]:
    """Every CJK code point appearing in the game's own source, excluding the
    generated glyph table (which would otherwise trivially "use" everything)."""
    used: set[int] = set()
    for path in (REPO / "src").rglob("*.rs"):
        if path.name == "cjk_glyphs.rs":
            continue
        for ch in path.read_text(encoding="utf-8"):
            cp = ord(ch)
            if is_cjk(cp):
                used.add(cp)
    return used


def read_table() -> set[int]:
    """Code points currently present in the generated table."""
    if not GLYPH_RS.exists():
        return set()
    pat = re.compile(r"^\s*\('\\u\{([0-9A-Fa-f]+)\}'")
    found: set[int] = set()
    for line in GLYPH_RS.read_text(encoding="utf-8").splitlines():
        m = pat.match(line)
        if m:
            found.add(int(m.group(1), 16))
    return found


def cmd_scan() -> int:
    used = scan_source()
    table = read_table()
    missing = sorted(used - table)
    dead = len(table) - (len(used) - len(missing))

    print(f"source uses        {len(used):>7,} CJK code points")
    print(f"table contains     {len(table):>7,} entries")
    print(f"used but missing   {len(missing):>7,}"
          + ("   <-- the HUD would render blanks" if missing else "   (coverage complete)"))
    if table:
        print(f"table but unused   {dead:>7,}   ({100.0 * dead / len(table):.1f}% dead weight)")

    USED_TXT.write_text(
        "# CJK code points that Steel Front's source actually references.\n"
        "# Generated by tools/extract_cjk_glyphs.py --scan -- do not edit by hand.\n"
        "# Any replacement font MUST cover every code point listed here.\n"
        + "".join(f"{cp:04X}\n" for cp in sorted(used)),
        encoding="utf-8",
    )
    print(f"\nwrote {USED_TXT.relative_to(REPO)} ({len(used)} code points)")
    if missing:
        sample = "".join(chr(cp) for cp in missing[:40])
        print(f"missing sample: {sample}")
    return 1 if missing else 0


def cmd_extract(font_path: Path, keep_all: bool, size: int = CELL, dy: int = 0) -> int:
    try:
        from fontTools.ttLib import TTFont, TTCollection
    except ImportError:
        print("error: fontTools is required (pip install fonttools)", file=sys.stderr)
        return 2
    from PIL import Image, ImageDraw, ImageFont

    codepoints = sorted(read_table() if keep_all else scan_source())
    if not codepoints:
        print("error: no code points to extract", file=sys.stderr)
        return 2

    # A .ttc is a collection; take face 0 unless told otherwise.
    if font_path.suffix.lower() == ".ttc":
        coll = TTCollection(str(font_path))
        cmap = coll.fonts[0].getBestCmap()
    else:
        cmap = TTFont(str(font_path)).getBestCmap()

    missing = [cp for cp in codepoints if cp not in cmap]
    if missing:
        print(f"error: {font_path.name} does not cover {len(missing)} required code "
              f"point(s); refusing to write a table that would blank the HUD.",
              file=sys.stderr)
        print("missing: " + "".join(chr(cp) for cp in missing[:60]), file=sys.stderr)
        return 1

    font = ImageFont.truetype(str(font_path), size)
    rows_out: list[str] = []
    for cp in codepoints:
        img = Image.new("L", (CELL, CELL), 0)
        # `anchor="lt"` pins the glyph's ascender to the cell top. That is NOT the
        # same as the CJK em box, so most faces end up sitting low in a 12px cell
        # with their bottom row clipped -- measured on Noto Sans SC, the ink
        # started 3-4 rows down. `--dy` corrects that; `--calibrate` finds the
        # value empirically instead of by guessing.
        ImageDraw.Draw(img).text((0, dy), chr(cp), fill=255, font=font, anchor="lt")
        px = img.load()
        rows = []
        for y in range(CELL):
            bits = 0
            for x in range(CELL):
                if px[x, y] > 127:
                    bits |= 1 << (ROW_MASK_BITS - 1 - x)
            rows.append(bits)
        rows_out.append(
            "    ('\\u{{{:X}}}', [{}]),".format(cp, ", ".join(f"0x{b:03X}" for b in rows))
        )

    header = f'''//! Pre-baked {CELL}x{CELL} CJK pixel bitmaps for the HUD.
//!
//! **Generated file -- do not edit by hand.** Regenerate with:
//!
//!     python tools/extract_cjk_glyphs.py --font <path-to-font>
//!
//! Source font: `{font_path.name}` at {size}px, dy={dy}
//! Coverage: {len(codepoints)} code points, taken from the set the source actually
//! references (see `tools/cjk_used_codepoints.txt`).
//!
//! ## Licensing
//!
//! The bitmaps below are a **derived work of the source font**, so the font's
//! licence governs whether this file may be redistributed. The pre-2026-09-14
//! revision of this file was extracted from Windows' **SimSun**, a proprietary
//! font whose licence prohibits redistribution -- see `THIRD-PARTY-NOTICES.md`
//! section 3. When regenerating, use a font that is licensed for redistribution
//! and for embedding a derived bitmap (SIL OFL 1.1 fonts such as Noto Sans CJK
//! or Source Han Sans are the recommended choice).
//!
//! ## Format contract
//!
//! `[u16; {CELL}]` is one row per element, bit {ROW_MASK_BITS - 1} = leftmost pixel of that row.
//! `ui.rs` indexes this table directly; changing the cell size or the bit order
//! makes the HUD render garbage rather than fail loudly.

/// {CELL}x{CELL} bitmaps, sorted by code point.
pub static CJK_GLYPHS: &[(char, [u16; {CELL}])] = &[
'''
    GLYPH_RS.write_text(header + "\n".join(rows_out) + "\n];\n", encoding="utf-8")
    print(f"wrote {GLYPH_RS.relative_to(REPO)}: {len(codepoints)} glyphs "
          f"from {font_path.name}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scan", action="store_true",
                    help="report usage and write the used-code-point list; changes nothing else")
    ap.add_argument("--font", type=Path,
                    help="font file to extract from (.ttf/.otf/.ttc)")
    ap.add_argument("--size", type=int, default=CELL, help=f"render size in px (default {CELL})")
    ap.add_argument("--dy", type=int, default=0, help="vertical nudge in px; use negative to raise the ink")
    ap.add_argument("--all", action="store_true",
                    help="with --font: keep the table's current coverage instead of trimming "
                         "to the code points the source uses")
    args = ap.parse_args()

    if args.scan:
        return cmd_scan()
    if args.font:
        if not args.font.exists():
            print(f"error: no such font: {args.font}", file=sys.stderr)
            return 2
        return cmd_extract(args.font, args.all, args.size, args.dy)
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
