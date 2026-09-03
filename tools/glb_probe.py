"""Dump the accessor/attribute layout of a GLB so the engine loader's job is explicit.

Usage: python glb_probe.py <file.glb> [...]
No third-party deps: parses the 12-byte GLB header + JSON chunk by hand.
"""
import json
import struct
import sys

COMPONENT = {5120: "i8", 5121: "u8", 5122: "i16", 5123: "u16", 5125: "u32",
             5126: "f32"}
TYPE_N = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}


def probe(path):
    with open(path, "rb") as fh:
        blob = fh.read()
    magic, version, length = struct.unpack_from("<III", blob, 0)
    assert magic == 0x46546C67, "not a GLB"
    clen, ctype = struct.unpack_from("<II", blob, 12)
    assert ctype == 0x4E4F534A, "first chunk is not JSON"
    gltf = json.loads(blob[20:20 + clen].decode("utf-8").rstrip("\x00"))

    print("=" * 78)
    print(path)
    print("  glb version=%d declared_len=%d actual_len=%d json_len=%d" %
          (version, length, len(blob), clen))
    print("  asset generator=%r min_version=%r" %
          (gltf.get("asset", {}).get("generator"), gltf.get("asset", {}).get("minVersion")))
    print("  required_extensions=%s" % (gltf.get("extensionsRequired"),))

    accs = gltf.get("accessors", [])
    views = gltf.get("bufferViews", [])

    def acc(i):
        a = accs[i]
        v = views[a["bufferView"]]
        n = TYPE_N[a["type"]]
        comp = COMPONENT[a["componentType"]]
        return a, v, n, comp

    print("  meshes=%d nodes=%d materials=%d images=%d buffers=%d" %
          (len(gltf.get("meshes", [])), len(gltf.get("nodes", [])),
           len(gltf.get("materials", [])), len(gltf.get("images", [])),
           len(gltf.get("buffers", []))))

    for mi, m in enumerate(gltf.get("meshes", [])):
        print("  mesh[%d] %r primitives=%d" % (mi, m.get("name"), len(m["primitives"])))
        for pi, prim in enumerate(m["primitives"]):
            print("    prim[%d] mode=%s material=%s indices=%s" %
                  (pi, prim.get("mode", 4), prim.get("material"),
                   prim.get("indices")))
            for sem, ai in sorted(prim["attributes"].items()):
                a, v, n, comp = acc(ai)
                stride = v.get("byteStride")
                print("      %-14s acc#%-3d type=%-6s comp=%-4s count=%-6d "
                      "view#%s off=%s stride=%s target=%s" %
                      (sem, ai, a["type"], comp, a["count"],
                       a.get("bufferView"), v.get("byteOffset"), stride,
                       v.get("target")))
                if a["type"] == "VEC3" and comp == "f32" and "min" in a:
                    print("                     min=%s max=%s" %
                          ([round(x, 3) for x in a["min"]],
                           [round(x, 3) for x in a["max"]]))
                if a["type"] == "VEC4" and "min" in a:
                    print("                     min=%s max=%s" %
                          ([round(x, 3) for x in a["min"]],
                           [round(x, 3) for x in a["max"]]))
            ext = prim.get("extensions", {})
            if ext:
                print("      extensions=%s" % list(ext.keys()))

    for ni, nd in enumerate(gltf.get("nodes", [])):
        keys = [k for k in nd if k not in ("name",)]
        print("  node[%d] %r keys=%s" % (ni, nd.get("name"), keys))
        if "translation" in nd:
            print("      translation=%s" % [round(x, 3) for x in nd["translation"]])
        if "matrix" in nd:
            print("      matrix=%s" % [round(x, 3) for x in nd["matrix"]])
        if "mesh" in nd:
            print("      mesh=%d" % nd["mesh"])
    for si, s in enumerate(gltf.get("scenes", [])):
        print("  scene[%d] %r nodes=%s" % (si, s.get("name"), s.get("nodes")))
    print("  default_scene=%s" % gltf.get("scene"))


for arg in sys.argv[1:]:
    probe(arg)
