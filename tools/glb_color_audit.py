#!/usr/bin/env python3
"""Audit vertex colors in every GLB under a directory.

Why this exists
---------------
The engine has no normal slot and (for authored props) no material: an asset's
entire appearance comes from its **vertex colors**. If an exporter forgets to
write COLOR_0, the mesh renders as flat white -- which is exactly the "pure
white slab dead-centre of the street" seen in screenshots on 2026-09-12.

Rather than guess which asset is at fault (or re-run the game twice to A/B
`RV3D_NO_PROPS=1` and eyeball two frames), read the files directly. A GLB is a
12-byte header + a JSON chunk + a BIN chunk, so this needs no bpy and no
Blender launch.

Usage (system python 3.11 is fine):
    python tools/glb_color_audit.py assets/props
    python tools/glb_color_audit.py assets --recursive

Exit code is 0 always; this is an inspection tool, not a gate.
"""

import json
import os
import struct
import sys

# glTF component types we may meet in COLOR_0
_COMP = {
    5120: ("b", 1), 5121: ("B", 1), 5122: ("h", 2),
    5123: ("H", 2), 5125: ("I", 4), 5126: ("f", 4),
}
_NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}


def read_glb(path):
    """Return (json_dict, bin_bytes) or (None, None) if not a GLB."""
    with open(path, "rb") as fh:
        data = fh.read()
    if len(data) < 12 or data[:4] != b"glTF":
        return None, None
    _magic, _ver, _length = struct.unpack_from("<III", data, 0)
    off = 12
    js, bin_chunk = None, b""
    while off + 8 <= len(data):
        clen, ctype = struct.unpack_from("<II", data, off)
        chunk = data[off + 8: off + 8 + clen]
        if ctype == 0x4E4F534A:      # 'JSON'
            js = json.loads(chunk.decode("utf-8"))
        elif ctype == 0x004E4942:    # 'BIN'
            bin_chunk = chunk
        off += 8 + clen + ((4 - clen % 4) % 4 if clen % 4 else 0)
    return js, bin_chunk


def accessor_values(gltf, blob, idx):
    """Decode a (non-sparse, non-interleaved) accessor into a list of tuples."""
    acc = gltf["accessors"][idx]
    n = _NCOMP[acc["type"]]
    fmt, size = _COMP[acc["componentType"]]
    bv = gltf["bufferViews"][acc["bufferView"]]
    base = bv.get("byteOffset", 0) + acc.get("byteOffset", 0)
    stride = bv.get("byteStride") or (n * size)
    out = []
    for i in range(acc["count"]):
        o = base + i * stride
        out.append(struct.unpack_from("<" + fmt * n, blob, o))
    return out, acc["componentType"]


def audit(path):
    gltf, blob = read_glb(path)
    if gltf is None:
        return None
    lines = []
    for mi, mesh in enumerate(gltf.get("meshes", [])):
        for pi, prim in enumerate(mesh.get("primitives", [])):
            attrs = prim.get("attributes", {})
            tag = "mesh%d.prim%d" % (mi, pi)
            if "COLOR_0" not in attrs:
                lines.append((tag, "MISSING COLOR_0", None, None))
                continue
            vals, ctype = accessor_values(gltf, blob, attrs["COLOR_0"])
            # 8/16-bit integer colors are normalized; float is already 0..1
            div = {5121: 255.0, 5123: 65535.0}.get(ctype, 1.0)
            rgb = [(v[0] / div, v[1] / div, v[2] / div) for v in vals]
            lo = tuple(min(c[i] for c in rgb) for i in range(3))
            hi = tuple(max(c[i] for c in rgb) for i in range(3))
            lines.append((tag, "ok", lo, hi))
    return lines


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    rec = "--recursive" in sys.argv
    root = args[0] if args else "assets/props"
    files = []
    if rec:
        for d, _sub, names in os.walk(root):
            files += [os.path.join(d, n) for n in names if n.lower().endswith(".glb")]
    else:
        files = [os.path.join(root, n) for n in sorted(os.listdir(root))
                 if n.lower().endswith(".glb")]
    files.sort()

    n_missing = 0
    n_white = 0
    for p in files:
        try:
            lines = audit(p)
        except Exception as exc:                       # noqa: BLE001
            print("%-34s  !! parse error: %s" % (os.path.basename(p), exc))
            continue
        if lines is None:
            print("%-34s  !! not a GLB" % os.path.basename(p))
            continue
        name = os.path.basename(p)
        for tag, status, lo, hi in lines:
            if status == "MISSING COLOR_0":
                n_missing += 1
                print("%-34s %-12s ** MISSING COLOR_0 -> renders WHITE **" % (name, tag))
            else:
                flat = (hi[0] - lo[0] < 0.01 and hi[1] - lo[1] < 0.01
                        and hi[2] - lo[2] < 0.01)
                if flat and lo[0] > 0.95 and lo[1] > 0.95 and lo[2] > 0.95:
                    n_white += 1
                    print("%-34s %-12s ** uniform WHITE %.2f,%.2f,%.2f **"
                          % (name, tag, lo[0], lo[1], lo[2]))
                elif flat:
                    print("%-34s %-12s uniform %.2f,%.2f,%.2f"
                          % (name, tag, lo[0], lo[1], lo[2]))
                # varied colors are the normal case -> stay quiet

    print("")
    print("scanned %d glb, %d primitive(s) missing COLOR_0, %d uniform-white"
          % (len(files), n_missing, n_white))
    return 0


if __name__ == "__main__":
    sys.exit(main())
