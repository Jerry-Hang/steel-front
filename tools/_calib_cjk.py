"""Calibrate the CJK bitmap extraction: find the (size, anchor, offset) that makes
a 12x12 cell actually get filled by Noto Sans SC's glyphs.

Run:  python tools/_calib_cjk.py
"""
from PIL import Image, ImageDraw, ImageFont
from pathlib import Path

FONT = Path("build/_font/noto-sc-subset.otf")
# 灭 and 人 are the two the test flagged; the rest are the test's own sample.
CHARS = "灭人中风暴雨设歼敌连发国永"


def measure(size, anchor, dy):
    f = ImageFont.truetype(str(FONT), size)
    out = []
    for ch in CHARS:
        img = Image.new("L", (12, 12), 0)
        ImageDraw.Draw(img).text((0, dy), ch, fill=255, font=f, anchor=anchor)
        px = img.load()
        rows = sum(1 for y in range(12) if any(px[x, y] > 127 for x in range(12)))
        cols = sum(1 for x in range(12) if any(px[x, y] > 127 for y in range(12)))
        out.append((ch, rows, cols))
    return out


print("Legend: each cell is <filled_rows><filled_cols>; worst = min over both, over all chars.")
print()
print(f"{'size':>4} {'anchor':>6} {'dy':>3}   " + " ".join(f"{c:>3}" for c in CHARS) + "   worst")
best = None
for size in range(11, 19):
    for anchor in ("lt", "la", "ls", "mm"):
        for dy in (0, -1, 1, -2, 2):
            d = measure(size, anchor, dy)
            worst = min(min(r, c) for _, r, c in d)
            if best is None or worst > best[0]:
                best = (worst, size, anchor, dy)
            if worst >= 8:
                print(f"{size:>4} {anchor:>6} {dy:>3}   "
                      + " ".join(f"{r:>1}{c:>2}" for _, r, c in d) + f"   {worst}")

print()
print(f"BEST: worst={best[0]}  size={best[1]}  anchor={best[2]!r}  dy={best[3]}")
d = measure(best[1], best[2], best[3])
for ch, r, c in d:
    flag = "OK " if (r >= 8 and c >= 8) else "SPARSE"
    print(f"  {ch}  rows={r:>2} cols={c:>2}  {flag}")
