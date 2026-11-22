"""Zip a directory for Windows distribution with UTF-8 entry names.

Why not `tar -a` / `Compress-Archive`:
  * bsdtar on Windows encodes non-ASCII entry names in the local ANSI codepage and
    does not set the zip UTF-8 flag, so names like `联机主机.bat` come out garbled;
  * PowerShell 5.1's Compress-Archive has the same class of problem.
Python's zipfile sets bit 11 (UTF-8 flag) for non-ASCII names, which Explorer,
7-Zip and bsdtar all honour.

Usage: python -X utf8 tools/zip_release.py <source_dir> <out.zip>
"""
import os
import sys
import zipfile


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: zip_release.py <source_dir> <out.zip>")
        return 2
    src = os.path.abspath(sys.argv[1])
    out = os.path.abspath(sys.argv[2])
    if not os.path.isdir(src):
        print(f"source dir not found: {src}")
        return 2
    os.makedirs(os.path.dirname(out), exist_ok=True)
    entries = 0
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
        for root, dirs, files in os.walk(src):
            dirs.sort()
            rel_root = os.path.relpath(root, src)
            if rel_root != ".":
                zf.write(root, arcname=rel_root.replace(os.sep, "/") + "/")
            for name in sorted(files):
                full = os.path.join(root, name)
                rel = os.path.relpath(full, src).replace(os.sep, "/")
                zf.write(full, arcname=rel)
                entries += 1
    size_mb = os.path.getsize(out) / (1024 * 1024)
    print(f"{out} ({size_mb:.1f} MB, {entries} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
