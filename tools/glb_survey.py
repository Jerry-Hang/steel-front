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
    # 🔴 底面高度（2026-11-21）：资产契约要求"原点在底面中心"（AGENTS 铁律 D）。
    # 一旦某件资产的 minY 明显大于 0，它就会整件**浮在地面上** —— 用户报的
    # "房子底部缺一块"最直接的一种成因。取所有 POSITION accessor 的 min/max 之并。
    base = None
    for m in g.get("meshes", []):
        for pr in m.get("primitives", []):
            ai = pr.get("attributes", {}).get("POSITION")
            if ai is None:
                continue
            a = accs[ai]
            if "min" not in a or "max" not in a:
                continue
            y0, y1 = a["min"][1], a["max"][1]
            if base is None:
                base = [y0, y1]
            else:
                base[0] = min(base[0], y0)
                base[1] = max(base[1], y1)
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
        "baseY": round(base[0], 3) if base else None,
        "topY": round(base[1], 3) if base else None,
    }


# 允许"设计性悬空"的白名单：这些资产的 minY > 0 是有意的，不是缺陷。
#   capture_flag = 据点旗杆**顶端**的旗面（杆子由 marker 画，旗面挂在 5m 处）。
FLOATING_ALLOW = {"capture_flag"}

# 底面契约容差（米）。AGENTS 铁律 D：原点在底面中心，Blender z=0 即模型最底。
# LOD 代理实测 0.010~0.012，所以容差取 0.02 才能放它们过去。
BASE_TOL = 0.02


def check_base(files):
    """底面契约判据。三态退出码（教训 46）：0=扫过无命中 / 1=有违规 / 2=根本没扫成。"""
    if not files:
        print("没扫到任何 .glb（退出码 2：这不等于通过）")
        return 2
    bad = []
    for f in files:
        r = survey(f)
        if "error" in r:
            print("%-30s 读取失败: %s" % (os.path.basename(f), r["error"]))
            return 2
        y = r.get("baseY")
        if y is None:
            print("%-30s 没有 POSITION min/max，无法判底面（退出码 2）" % os.path.basename(f))
            return 2
        name = os.path.basename(f)[:-4].replace("_lod1", "").replace("_lod2", "")
        if name in FLOATING_ALLOW:
            continue
        # 只判**悬空**（minY > 0）。负数 = 模型略埋进地面，不会产生缝，
        # 而且埋一点反而是好事（掠射角不露底边），所以不判。
        if y > BASE_TOL:
            bad.append((os.path.basename(f), y))
    if bad:
        print("底面契约违规 %d 件（minY > %.2f m ⇒ 会浮在地面上、底下露缝）:" % (len(bad), BASE_TOL))
        for n, y in bad[:20]:
            print("  %-30s minY=%+.3f" % (n, y))
        print("允许的设计性悬空（白名单）: %s" % ", ".join(sorted(FLOATING_ALLOW)))
        return 1
    print("底面契约 OK：%d 件全部贴地（白名单 %d 件设计性悬空）"
          % (len(files), len(FLOATING_ALLOW)))
    return 0


def main(argv):
    files = []
    for a in argv:
        if os.path.isdir(a):
            for root, _, names in os.walk(a):
                files += [os.path.join(root, n) for n in sorted(names)
                          if n.lower().endswith(".glb")]
        elif a.lower().endswith(".glb"):
            files.append(a)
    # `--check-base`：只做底面契约判据（给资产管线当闸门用），不打印大表。
    if argv and argv[0] == "--check-base":
        return check_base(files)
    if not files:
        print("没有找到 .glb")
        return 2
    cols = [("file", 34), ("sizeMB", 7), ("mesh", 5), ("node", 5), ("verts", 8),
            ("tris", 8), ("baseY", 8), ("topY", 8), ("stride", 6), ("aoff", 6),
            ("xform", 6), ("tex", 4), ("img", 4), ("mat", 4), ("missing", 22), ("ext", 18)]
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
