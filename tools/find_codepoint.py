"""Find a code point (given as hex) in a file and print line numbers + ASCII-safe context."""
import io
import sys

hexcp, path = sys.argv[1], sys.argv[2]
ch = chr(int(hexcp, 16))
out = []
for i, line in enumerate(io.open(path, encoding="utf-8").read().split("\n"), 1):
    if ch in line:
        out.append("%d: %s" % (i, line.strip().encode("ascii", "replace").decode("ascii")[:160]))
print("U+%s (%s) in %s: %d line(s)" % (hexcp.upper(), repr(ch), path, len(out)))
for line in out:
    print(line)
