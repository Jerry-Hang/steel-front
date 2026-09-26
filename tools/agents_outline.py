"""Report AGENTS.md section sizes, biggest first.

Why: AGENTS.md is injected into every session and is silently truncated past the 65,536 B
hard cap (2026-09-26: it was 65,387 B, so the tail -- the last lessons -- was being cut).
Trimming needs to target the fat sections, not the load-bearing rules.

Usage:
    python tools/agents_outline.py            # writes %TEMP%\\agents_outline.txt (UTF-8)
    python tools/agents_outline.py out.txt    # or an explicit path

Prints only the path and the total, because this shell's console is GBK and would mangle
the section titles (see AGENTS lesson 30).
"""
import io
import os
import sys

repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
path = os.path.join(repo, "AGENTS.md")
out_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.environ.get("TEMP", "."), "agents_outline.txt"
)

with io.open(path, encoding="utf-8") as fh:
    text = fh.read()
lines = text.split("\n")

rows = []
cur = None
for i, line in enumerate(lines):
    if line.startswith("## ") or line.startswith("### "):
        if cur:
            rows.append((cur[0], cur[1], i))
        cur = (line[:72], i)
if cur:
    rows.append((cur[0], cur[1], len(lines)))

sized = [(name, s, e, sum(len(l.encode("utf-8")) + 1 for l in lines[s:e]))
         for name, s, e in rows]
sized.sort(key=lambda r: -r[3])
total = len(text.encode("utf-8"))

with io.open(out_path, "w", encoding="utf-8") as fh:
    fh.write("AGENTS.md total %d bytes / %d lines (hard cap 65536, target < 48KB)\n"
             % (total, len(lines)))
    for name, s, e, b in sized:
        fh.write("%7d  L%-5d-%-5d %s\n" % (b, s + 1, e, name))

print("AGENTS.md %d bytes; outline -> %s" % (total, out_path))
