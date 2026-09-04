"""盘点一批 GLB 的结构性特征，判断能否被本项目的加载器直接吃下。

用法:  python tools/glb_survey.py <目录或文件> [...]

对照 engine/assets.rs::parse_glb 的实际能力，逐项标出会被静默错读的开关：
  stride  有 bufferView 带 byteStride（交错缓冲）→ 加载器不读它，几何会错
  aoff    有 accessor 带非零 byteOffset         → 加载器已支持（2026-09-03 修）
  xform   有节点带 matrix/TRS                   → 加载器完全丢弃节点变换
  tex/img 有贴图/图像                            → 加载器不解析贴图，进来是灰模
以及面数与语义集合（缺 NORMAL / TEXCOORD_0 / COLOR_0 的后果）。
"""
import json
import os
import struct
import sys

MAX_JSON = 4 * 1024 * 1024


def load_json(path):
    with open(path, "rb") as fh:
        blob = fh.read(12 + 8 + MAX_JSON)
    magic, version, total = struct.unpack_from("<III", blob, 0)
    if magic != 0x46546C67:
        return None, "不是 GLB（magic=%08x）" % magic
    clen, ctype = struct.unpack_from("<II", blob, 12)
    if ctype != 0x4E4F534A:
        return None, "第一个 chunk 不是 JSON"
    if clen > len(blob) - 20:
        return None, "JSON chunk 超出已读取范围（%d 字节）" % clen
    txt = blob[20:20 + clen].decode("utf-8", "replace").rstrip("\x00 ")
    try:
        return json.loads(txt), None
    except Exception as e:  # noqa: BLE001 - 报告原文更有用
        return None, "JSON 解析失败: %s" % e


def survey(path):
    g, err = load_json(path)
    if g is None:
        return {"file": os.path.basename(path), "error": err}
    accs = g.get("accessors", [])
    bvs = g.get("bufferViews", [])
    nodes = g.get("nodes", [])
    sems = set()
    verts = tris = 0
    missing = set()
    for m in g.get("meshes", []):
        for pr in m.get("primitives", []):
            a = pr.get("attributes", {})
            sems |= set(a.keys())
            for need in ("NORMAL", "TEXCOORD_0", "COLOR_0"):
                if need not in a:
                    missing.add(need)
            if pr.get("indices") is not None:
                tris += accs[pr["indices"]].get("count", 0) // 3
            if "POSITION" in a:
                verts += accs[a["POSITION"]].get("count", 0)
    xform = any(("matrix" in n) or ("rotation" in n) or ("translation" in n)
                or ("scale" in n) for n in nodes)
    ext = set()
    for k in ("extensionsRequired", "extensionsUsed"):
        ext |= set(g.get(k) or [])
    return {
        "file": os.path.basename(path),
        "sizeMB": round(os.path.getsize(path) / 1048576.0, 1),
        "mesh": len(g.get("meshes", [])),
        "node": len(nodes),
        "verts": verts,
        "tris": tris,
        "stride": any(v.get("byteStride") for v in bvs),
        "aoff": any(a.get("byteOffset") for a in accs),
        "xform": xform,
        "tex": len(g.get("textures", [])),
        "img": len(g.get("images", [])),
        "mat": len(g.get("materials", [])),
        "missing": ",".join(sorted(missing)) or "-",
        "ext": ",".join(sorted(ext)) or "-",
    }


def main(argv):
    files = []
    for a in argv:
        if os.path.isdir(a):
            for root, _, names in os.walk(a):
                files += [os.path.join(root, n) for n in sorted(names)
                          if n.lower().endswith(".glb")]
        elif a.lower().endswith(".glb"):
            files.append(a)
    if not files:
        print("没有找到 .glb")
        return 1
    cols = [("file", 34), ("sizeMB", 7), ("mesh", 5), ("node", 5), ("verts", 8),
            ("tris", 8), ("stride", 6), ("aoff", 6), ("xform", 6),
            ("tex", 4), ("img", 4), ("mat", 4), ("missing", 22), ("ext", 18)]
    print(" ".join(h.ljust(w) for h, w in cols))
    bad = 0
    for f in files:
        r = survey(f)
        if "error" in r:
            print("%-34s 读取失败: %s" % (os.path.basename(f)[:34], r["error"]))
            bad += 1
            continue
        cells = []
        for k, w in cols:
            v = r.get(k, "")
            cells.append(str(v).ljust(w))
        print(" ".join(cells))
        # 汇总会被本加载器静默错读的项
        flags = []
        if r["stride"]:
            flags.append("byteStride(交错缓冲,会读错几何)")
        if r["xform"]:
            flags.append("节点变换(会被丢弃)")
        if r["img"]:
            flags.append("%d 张贴图(加载器不解析→灰模)" % r["img"])
        if r["mesh"] > 1 or r["node"] > 1:
            flags.append("多网格/多节点(合并后无逐件变换)")
        if "NORMAL" in r["missing"]:
            flags.append("缺 NORMAL(无法烘焙顶点光照)")
        if flags:
            bad += 1
            print("    ⚠ " + "; ".join(flags))
    print("\n需预处理或有风险: %d / %d" % (bad, len(files)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
