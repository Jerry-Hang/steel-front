"""Steel Front gun-mesh pre-processor — headless Blender (bpy): downloaded GLB -> engine-loadable GLB.

The engine's GLB reader (src/engine/assets.rs::parse_glb) understands only
POSITION / NORMAL / TEXCOORD_0 / COLOR_0 in dense (un-strided) bufferViews, one flat
mesh, and NOTHING else: no node transforms, no materials, no textures, no images.
Every downloaded gun violates all of that, so each renders as mis-read grey garbage at
the origin.  This script bakes everything the engine cannot parse into the one thing it
can read: vertices.

Run
    "D:/3D_Work/blender/blender-5.2.1-windows-x64/blender.exe" --background ^
        --python tools/blender/prep_guns.py -- [--in D:/Rust/3D] ^
        [--out D:/Rust/steel-front/assets/guns_ext] [--only sub1,sub2] ^
        [--verts 12000] [--tris 24000] [--scale longest|none] [--no-reorient] ^
        [--keep-junk] [--shots all|name-sub] [--report FILE]

Conventions (do not change without updating the engine loader)
  * SCALE: 1 Blender unit == 1 metre is NOT assumed - these files disagree wildly
    (as_val spans 9.9 units, saiga 345, osv-96 964).  Default normalises the longest
    bounding-box extent to 1.0 unit; `--scale none` keeps the source scale.  main.rs
    re-scales by 1.35/longest-axis at load, so this only changes the reported numbers.
  * COLOUR SPACE - COLOR_0 holds LINEAR albedo.  That is both what glTF requires of
    COLOR_0 and what the engine wants: main.rs GUN_REF_ALBEDO is documented as
    "line" and the flat=3 vertex-colour path is written to a B8G8R8A8_SRGB swapchain
    (one hardware linear->sRGB encode), and assets.rs read_acc divides the normalized
    integer by 65535 with no gamma step.
    bpy's Image.pixels returns the RAW sRGB-encoded value of an 8-bit sRGB image
    (verified here: a PNG byte of 128 reads back as 0.50196, not 0.21586), so the
    sRGB->linear transfer is applied explicitly by this script.  baseColorFactor is
    already linear per spec and is used as-is.
  * When a texture IS linked, the material's baseColorFactor is ignored: the glTF
    importer leaves the Principled socket at its 0.8 default in that case, so
    multiplying by it would silently darken every textured gun by 20%.
  * Blender stores BYTE_COLOR as sRGB-encoded bytes but exposes linear floats on both
    sides (verified: set 0.02 -> get 0.02029 -> COLOR_0 on disk 0.0203) and the 5.2
    exporter writes it as VEC4 / UNSIGNED_SHORT / normalized.  So we write linear
    floats and the loop closes.  Exactly ONE colour attribute may survive: with two,
    the exporter emits COLOR_0 *and* COLOR_1 and the engine reads the wrong one.
  * ORIENTATION - baked into the vertices, the node carries no transform.  Canonical
    target in the exported Y-up file: muzzle along +Z, up +Y, the gun's right +X, bbox
    centred on the origin.  That is the convention main.rs already assumes for its
    baked key light (GUN_KEY_DIR, "muzzle +Z, gun top +Y") and it makes load_gun_glb's
    longest-axis branch fall through to IDENTITY.  `--no-reorient` keeps the source
    orientation; the report states both either way.
  * BUDGET - the engine pins the gun vertex buffer at 32768 verts and the index buffer
    at 262144 indices; growing them destroys an in-flight buffer and previously cost an
    NVIDIA device-lost crash.  Defaults are <=12000 verts / <=24000 tris, 37% and 9% of
    the hard caps, leaving room for the exporter splitting verts for normals.
"""

import json
import os
import struct
import sys
import traceback

import bpy
import bmesh
import numpy as np
from mathutils import Matrix, Vector

MAX_TEX = 256                 # textures are box-filtered to this before sampling
TARGET_VERTS = 12000
TARGET_TRIS = 24000
HARD_CAP_VERTS = 32768
HARD_CAP_IDX = 262144
BARREL_HINTS = ("barrel", "silencer", "sound_suppressor", "muzzle", "compensator",
                "brake", "handguard", "hand_guard", "frontsight", "front_sight",
                "flight", "gas tube", "gastube")
GUN_GREY = (0.35, 0.35, 0.35)

ARGV = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []


def opt(flag, default=None):
    if flag in ARGV:
        i = ARGV.index(flag)
        if i + 1 < len(ARGV) and not ARGV[i + 1].startswith("--"):
            return ARGV[i + 1]
    return default


def has(flag):
    return flag in ARGV


def pprint(*args):
    """Console-safe print: some sources carry CP1251 object names (pkp.glb)."""
    line = " ".join(str(a) for a in args)
    try:
        print(line, flush=True)
    except UnicodeEncodeError:
        print(line.encode("ascii", "replace").decode("ascii"), flush=True)


# ---------------------------------------------------------------------- scene reset
def reset_scene():
    if bpy.context.object and bpy.context.object.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete()
    for block in (bpy.data.meshes, bpy.data.materials, bpy.data.images,
                  bpy.data.lights, bpy.data.cameras, bpy.data.textures,
                  bpy.data.curves, bpy.data.armatures, bpy.data.actions,
                  bpy.data.node_groups):
        for item in list(block):
            if item.users == 0:
                block.remove(item)


# ------------------------------------------------------------------ colour helpers
def srgb_to_linear(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


_TEX_CACHE = {}
_FACTORS = {}


def read_pixels(img):
    """Force-materialise an image buffer and return float32 (h, w, 4), else None.

    Packed GLB textures report has_data=False until something touches them (measured:
    every material of all 14 guns came out grey when the read was gated on that flag),
    so read unconditionally and judge the buffer by its contents.
    """
    for attempt in (0, 1):
        w, h = int(img.size[0]), int(img.size[1])
        if w < 1 or h < 1:
            return None
        buf = np.empty(w * h * 4, dtype=np.float32)
        ok = True
        try:
            img.pixels.foreach_get(buf)
        except Exception as e:
            pprint("   WARN %s: pixels unreadable (%s)" % (img.name, e))
            ok = False
        if ok and float(np.abs(buf).sum()) > 1e-3:
            return buf.reshape(h, w, 4)
        if attempt == 0:                       # lazily loaded: force it, then try once more
            try:
                img.update()
                img.reload()
            except Exception as e:
                pprint("   WARN %s: reload failed (%s)" % (img.name, e))
    return None


def image_lut(img):
    """-> (lut | None, is_linear, note).  lut is float32 (h, w, ch) in stored space."""
    if img is None:
        return None, False, "no image on the material"
    key = img.name
    if key in _TEX_CACHE:
        return _TEX_CACHE[key]
    if max(int(img.size[0]), int(img.size[1])) > MAX_TEX:
        f = MAX_TEX / float(max(img.size[0], img.size[1]))
        try:
            img.scale(max(1, int(img.size[0] * f)), max(1, int(img.size[1] * f)))
        except Exception as e:
            pprint("   WARN %s: scale() failed (%s)" % (img.name, e))
    lut = read_pixels(img)
    if lut is None:
        note = ("%s: no readable pixels (%dx%d src=%s packed=%dB cs=%s)"
                % (img.name, img.size[0], img.size[1], img.source,
                   len(img.packed_file.data) if img.packed_file else 0,
                   img.colorspace_settings.name))
        out = (None, False, note)
    else:
        out = (lut, img.colorspace_settings.name != "sRGB",
               "%s %dx%d cs=%s" % (img.name, lut.shape[1], lut.shape[0],
                                   img.colorspace_settings.name))
    _TEX_CACHE[key] = out
    return out


def sample_lut(lut, u, v):
    """Nearest texel with GL_REPEAT wrapping; Blender row 0 is v=0, so no flip."""
    h, w = lut.shape[0], lut.shape[1]
    x = np.clip((np.mod(u, 1.0) * w).astype(np.int32), 0, w - 1)
    y = np.clip((np.mod(v, 1.0) * h).astype(np.int32), 0, h - 1)
    return lut[y, x]


def glb_factors(path):
    """baseColorFactor per material, read straight out of the source GLB.

    Authoritative where the node tree is not: the glTF importer leaves Principled
    "Base Color" at its 0.8 default whenever a texture is linked, so once a texture
    turns out to be unusable there is no factor left to fall back to inside Blender.
    glTF factors are linear already (assets.rs reads the same numbers).
    """
    out = {}
    try:
        with open(path, "rb") as fh:
            d = fh.read(12 + 8 + min(8 * 1024 * 1024, os.path.getsize(path)))
        clen = struct.unpack_from("<I", d, 12)[0]
        j = json.loads(d[20:20 + clen].decode("utf-8", "replace").rstrip("\x00 "))
    except Exception as e:
        pprint("   WARN cannot pre-read %s for baseColorFactor (%s)" % (path, e))
        return out
    for m in j.get("materials", []):
        f = (m.get("pbrMetallicRoughness") or {}).get("baseColorFactor") or [1.0, 1.0, 1.0, 1.0]
        nm = (m.get("name") or "").strip().lower()
        if nm:
            out[nm] = [float(c) for c in f[:3]]
    return out


def factor_for(mat_name):
    key = (mat_name or "").strip().lower()
    if key in _FACTORS:
        return _FACTORS[key]
    base = key.rsplit(".", 1)[0] if key[-4:].isdigit() else key
    hits = [v for k, v in _FACTORS.items() if k == base or k.startswith(base)]
    return hits[0] if len(hits) == 1 else None


class Source:
    def __init__(self, kind, note, img=None, linear=False, const=None, attr=None,
                 fallback=None):
        self.kind = kind          # image | vcol | const | avg
        self.note = note
        self.img = img
        self.linear = linear
        self.const = const
        self.attr = attr
        self.fallback = fallback  # linear baseColorFactor from the source GLB


def upstream_nodes(node, depth=0, acc=None, seen=None):
    acc = [] if acc is None else acc
    seen = set() if seen is None else seen
    if node is None or depth > 6 or len(acc) > 16 or id(node) in seen:
        return acc
    seen.add(id(node))
    acc.append(node)
    for sock in node.inputs:
        for link in sock.links:
            upstream_nodes(link.from_socket.node, depth + 1, acc, seen)
    return acc


def resolve_source(mat):
    src = _resolve_source(mat)
    src.fallback = factor_for(mat.name if mat else None)
    if src.kind == "const" and src.fallback is not None and "RGB node" not in src.note:
        src.const = src.fallback          # the file's own factor beats a socket default
        src.note += " (baseColorFactor from file)"
    return src


def _resolve_source(mat):
    """Reduce Principled Base Color to something samplable, logging approximations."""
    nt = mat.node_tree
    if nt is None:
        return Source("const", "no node tree", const=GUN_GREY)
    bsdf = next((n for n in nt.nodes if n.type == "BSDF_PRINCIPLED"), None)
    if bsdf is None:
        return Source("const", "no principled", const=GUN_GREY)
    sock = next((s for s in bsdf.inputs if s.name.lower().startswith("base color")), None)
    if sock is None:
        return Source("const", "no base color socket", const=GUN_GREY)
    const = tuple(float(c) for c in sock.default_value[:3])
    if not sock.links:
        return Source("const", "baseColorFactor", const=const)
    node = sock.links[0].from_socket.node
    if node.type == "TEX_IMAGE" and node.image is not None:
        return Source("image", "texture", img=node.image,
                      linear=node.image.colorspace_settings.name != "sRGB")
    if node.type == "VERTEX_COLOR":
        return Source("vcol", "vertex colour node", attr=node.get("attribute_name", ""))
    chain = upstream_nodes(node)
    imgs = [n for n in chain if n.type == "TEX_IMAGE" and n.image is not None]
    vcs = [n for n in chain if n.type == "VERTEX_COLOR"]
    tag = ",".join(sorted({n.type for n in chain}))
    if imgs:
        return Source("image", "approx: texture behind %s" % tag, img=imgs[0].image,
                      linear=imgs[0].image.colorspace_settings.name != "sRGB")
    if vcs:
        return Source("vcol", "approx: vertex colour", attr=vcs[0].get("attribute_name", ""))
    rgb = next((n for n in chain if n.type == "RGB"), None)
    if rgb is not None:
        return Source("const", "approx: RGB node",
                      const=tuple(float(c) for c in rgb.outputs[0].default_value[:3]))
    return Source("const", "UNRESOLVED %s" % tag, const=const)


def read_colour_attr(me, name):
    """Linear floats from an existing colour attribute (already linear on read)."""
    ca = None
    for c in me.color_attributes:
        if c.name == name:
            ca = c
            break
    if ca is None:
        ca = next((c for c in me.color_attributes if c.domain == "POINT"), None)
    if ca is None or not len(ca.data):
        return None
    n = len(ca.data)
    buf = np.empty(n * 4, dtype=np.float32)
    ca.data.foreach_get("color", buf)
    out = buf.reshape(n, 4)[:, :3].astype(np.float64)
    if not ca.data[0].color:
        pass
    return out if ca.data_type == "FLOAT_COLOR" else out


# ------------------------------------------------------------------------ the bake
def bake_vertex_colour(ob):
    """Per-loop material sample -> per-vertex average -> POINT BYTE_COLOR 'Col'.

    Faces welded onto one vertex average their colours, which is what we want: with no
    textures in the engine a material seam is just a vertex between two materials.
    """
    me = ob.data
    nloop = len(me.loops)
    loop_vert = np.empty(nloop, dtype=np.int32)
    me.loops.foreach_get("vertex_index", loop_vert)
    npoly = len(me.polygons)
    loop_start = np.empty(npoly, dtype=np.int32)
    loop_total = np.empty(npoly, dtype=np.int32)
    mat_idx = np.empty(npoly, dtype=np.int32)
    me.polygons.foreach_get("loop_start", loop_start)
    me.polygons.foreach_get("loop_total", loop_total)
    me.polygons.foreach_get("material_index", mat_idx)
    ordered = np.array_equal(loop_start, np.arange(npoly, dtype=np.int32) * 3)
    if not ordered or np.any(loop_total != 3):
        pprint("   WARN face/loop order is not 3-per-face; colour falls back to order")
    ulayer = None
    if len(me.uv_layers):
        ulayer = me.uv_layers.active
        if ulayer is None or not getattr(ulayer, "name", None):
            ulayer = me.uv_layers[0]
    uvs = None
    if ulayer is not None:
        uvs = np.empty(nloop * 2, dtype=np.float32)
        ulayer.data.foreach_get("uv", uvs)
        uvs = uvs.reshape(nloop, 2)
    nvert = len(me.vertices)
    acc = np.zeros((nvert, 3), dtype=np.float64)
    cnt = np.zeros(nvert, dtype=np.float64)
    stats, notes = {}, []

    def scatter(li, cols):
        np.add.at(acc, loop_vert[li], cols)
        np.add.at(cnt, loop_vert[li], 1.0)

    sources = []
    for slot, mat in enumerate(me.materials):
        sources.append(resolve_source(mat) if mat
                       else Source("const", "empty slot", const=GUN_GREY))

    def fb(src):
        v = src.fallback if src.fallback else GUN_GREY
        return np.clip(np.asarray(v, dtype=np.float64), 0.0, 1.0)

    for mi, src in enumerate(sources):
        sel = np.where(mat_idx == mi)[0]
        if sel.size == 0:
            continue
        stats[src.kind] = stats.get(src.kind, 0) + 1
        li = sel * 3 if ordered else loop_start[sel]
        if src.kind == "const":
            scatter(li, np.tile(fb(src), (li.size, 1)))
        elif src.kind == "vcol":
            per = read_colour_attr(me, src.attr)
            if per is None or len(per) != nvert:
                notes.append("material %d: vertex colour %r missing/mis-sized -> "
                             "baseColorFactor" % (mi, src.attr))
                scatter(li, np.tile(fb(src), (li.size, 1)))
            else:
                scatter(li, np.clip(per[loop_vert[li]], 0.0, 1.0))
        else:                              # image (or its average)
            lut, lin, note = image_lut(src.img)
            if lut is None:
                src.kind = "const"
                stats["image"] -= 1
                stats["const"] = stats.get("const", 0) + 1
                notes.append("material %d: %s -> baseColorFactor fallback" % (mi, note))
                src.const = tuple(fb(src))
                scatter(li, np.tile(fb(src), (li.size, 1)))
                notes.append("slot %d const: %s" % (mi, src.note))
                continue
            if uvs is None:
                src.kind = "avg"
                stats["avg"] = stats.get("avg", 0) + 1
                c = np.clip(lut[:, :, :3].reshape(-1, 3).mean(axis=0), 0.0, 1.0)
                if not lin:
                    c = np.clip(srgb_to_linear(c), 0.0, 1.0)
                scatter(li, np.tile(c.astype(np.float64), (li.size, 1)))
                continue
            got = sample_lut(lut, uvs[li, 0], uvs[li, 1])[:, :3].astype(np.float64)
            if not lin:
                got = srgb_to_linear(got)
            scatter(li, got)
        notes.append("slot %d %s: %s" % (mi, sources[mi].kind, sources[mi].note))
    hit = cnt > 0
    col = np.zeros((nvert, 3), dtype=np.float64)
    col[hit] = acc[hit] / cnt[hit, None]
    col[~hit] = GUN_GREY                   # faces with no material slot at all
    col = np.clip(col, 0.0, 1.0)
    for ca in list(me.color_attributes):
        me.color_attributes.remove(ca)
    ca = me.color_attributes.new(name="Col", type="BYTE_COLOR", domain="POINT")
    rgba = np.ones((nvert, 4), dtype=np.float32)
    rgba[:, :3] = col.astype(np.float32)
    ca.data.foreach_set("color", rgba.reshape(-1))
    me.update()
    lum = 0.2126 * col[:, 0] + 0.7152 * col[:, 1] + 0.0722 * col[:, 2]
    uniq = len({tuple(int(b) for b in (col[i] * 255)) for i in range(0, nvert,
                                                                    max(1, nvert // 400))})
    return {"kinds": stats, "luma_min": round(float(lum.min()), 4),
            "luma_max": round(float(lum.max()), 4), "luma_mean": round(float(lum.mean()), 4),
            "distinct_colours_sampled": uniq, "unhit_verts": int((~hit).sum()),
            "notes": notes, "uv_layer": ulayer.name if ulayer else "NONE"}


# -------------------------------------------------------------------------- geometry
def triangulate(me):
    """Triangulate, recompute normals (drops stale custom split data) and validate."""
    bm = bmesh.new()
    bm.from_mesh(me)
    bad = [f for f in bm.faces if len(f.verts) != 3]
    if bad:
        bmesh.ops.triangulate(bm, faces=bad, quad_method="FIXED", ngon_method="BEAUTY")
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.normal_update()
    bm.to_mesh(me)
    bm.free()
    me.validate(verbose=False)
    me.update()


def mesh_stats(me):
    """(verts referenced by at least one face, tris, loose verts).

    The engine's vertex buffer is sized by the POSITION accessor, and the glTF exporter
    only writes face-referenced verts - so loose verts are what made pkp look like it
    needed a 0.0085 decimation ratio (13532 Blender verts, 2762 on disk).  Budget on the
    number that is actually uploaded.
    """
    n = len(me.vertices)
    if n == 0:
        return 0, 0, 0
    lv = np.empty(len(me.loops), dtype=np.int32)
    me.loops.foreach_get("vertex_index", lv)
    used = int(np.unique(lv).size)
    return used, len(me.polygons), n - used


def clean_mesh(me, weld=True):
    """Drop loose verts / degenerate faces, weld exact coincidences.  -> log string"""
    bm = bmesh.new()
    bm.from_mesh(me)
    v0, f0 = len(bm.verts), len(bm.faces)
    loose = [v for v in bm.verts if not v.link_faces]
    if loose:
        bmesh.ops.delete(bm, geom=loose, context="VERTS")
    deg = [f for f in bm.faces if len(f.verts) < 3]
    if deg:
        bmesh.ops.delete(bm, geom=deg, context="FACES")
    nw = 0
    if weld:
        co = np.array([list(v.co) for v in bm.verts], dtype=np.float64)
        span = float((co.max(axis=0) - co.min(axis=0)).max()) if len(co) else 0.0
        before = len(bm.verts)
        bmesh.ops.remove_doubles(bm, verts=bm.verts[:], dist=max(span * 1e-5, 1e-7))
        nw = before - len(bm.verts)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.normal_update()
    bm.to_mesh(me)
    bm.free()
    me.validate(verbose=False)
    me.update()
    return "loose=%d welded=%d (%dv/%df -> %dv/%df)" % (len(loose), nw, v0, f0,
                                                        len(me.vertices), len(me.polygons))


def decimate_to(ob, verts_cap, tris_cap):
    """Collapse-decimate until both budgets hold, measured as the engine will see them."""
    used, tris, loose = mesh_stats(ob.data)
    if used <= verts_cap and tris <= tris_cap:
        return "under budget already (%dv/%dt)" % (used, tris), 1.0
    total = 1.0
    log = []
    prev = None
    for step in range(6):
        used, tris, loose = mesh_stats(ob.data)
        if used <= verts_cap * 0.97 and tris <= tris_cap * 0.97:
            break
        want = min(verts_cap * 0.88 / max(used, 1), tris_cap * 0.88 / max(tris, 1))
        want = min(max(want, 1e-4), 0.98)
        mod = ob.modifiers.new("dec", "DECIMATE")
        mod.decimate_type = "COLLAPSE"
        mod.ratio = want
        mod.use_collapse_triangulate = True
        try:
            with bpy.context.temp_override(active_object=ob, selected_objects=[ob],
                                           selected_editable_objects=[ob], object=ob):
                bpy.ops.object.modifier_apply(modifier=mod.name)
        except Exception as e:
            ob.select_set(True)
            bpy.context.view_layer.objects.active = ob
            try:
                bpy.ops.object.modifier_apply(modifier=mod.name)
            except Exception as e2:
                log.append("apply failed (%s/%s)" % (e, e2))
                break
        total *= want
        u2, t2, l2 = mesh_stats(ob.data)
        log.append("%.4f->%dv/%dt" % (want, u2, t2))
        if prev is not None and (u2 >= prev[0] * 0.98 or t2 >= prev[1] * 0.98):
            log.append("stalled, stop")
            break
        prev = (u2, t2)
    used, tris, loose = mesh_stats(ob.data)
    return "passes: " + " ".join(log) + ("; loose left=%d" % loose if loose else ""), \
        round(total, 5)


def world_bbox(ob):
    cs = np.array([list(ob.matrix_world @ Vector(c)) for c in ob.bound_box])
    return cs.min(axis=0), cs.max(axis=0)


def positions(me):
    co = np.empty(len(me.vertices) * 3, dtype=np.float32)
    me.vertices.foreach_get("co", co)
    return co.reshape(-1, 3)


def detect_axes(me, hints=()):
    """Which axis is the barrel, which is up - from geometry, not from node matrices.

    barrel  : longest bbox axis; the end whose outer slab sits closer to the axis line
              wins (a barrel/suppressor is thin, a stock is not).  A part named barrel/
              silencer/... overrides when the geometric margin is weak.
    up      : the larger of the two remaining axes (guns are taller than they are wide);
              signed so the model's centroid falls on the -up side (grip, magazine and
              stock hang below the bore line).
    """
    co = positions(me)
    mn, mx = co.min(axis=0), co.max(axis=0)
    ext = mx - mn
    L = int(np.argmax(ext))
    rest = [i for i in range(3) if i != L]
    U = rest[0] if ext[rest[0]] >= ext[rest[1]] else rest[1]
    W = rest[1] if U == rest[0] else rest[0]
    mid = (mn + mx) / 2.0
    perp = [i for i in range(3) if i != L]
    span = max(float(ext[L]), 1e-9)
    ends, radii = {}, {}
    for sign in (1, -1):
        d_along = (co[:, L] - mn[L]) if sign > 0 else (mx[L] - co[:, L])
        keep = d_along <= span * 0.07
        if int(keep.sum()) < 8:
            keep = d_along <= span * 0.2
        sub = co[keep]
        if len(sub) == 0:
            ends[sign] = radii[sign] = 1e9
            continue
        ends[sign] = max(float(sub[:, perp[0]].max() - sub[:, perp[0]].min()),
                         float(sub[:, perp[1]].max() - sub[:, perp[1]].min()))
        radii[sign] = float(np.hypot(sub[:, perp[0]] - mid[perp[0]],
                                     sub[:, perp[1]] - mid[perp[1]]).mean())
    msign = 1 if ends[1] <= ends[-1] else -1
    conf = round(max(ends.values()) / max(min(ends.values()), 1e-9), 2)
    method = "tip-thinness"
    # A part actually named barrel/silencer/handguard is authored metadata and beats a
    # thickness guess: take the end those parts *reach*, not their centroid (as_val's
    # suppressor spans a third of the gun, so its centroid sat near mid-length and the
    # first-match centroid rule pointed the wrong way).
    hl = [lo[L] for name, lo, hi in hints
          if any(h in name.lower() for h in BARREL_HINTS)]
    hh = [hi[L] for name, lo, hi in hints
          if any(h in name.lower() for h in BARREL_HINTS)]
    if hl and hh:
        touch_hi = max(hh) >= mx[L] - 0.05 * span
        touch_lo = min(hl) <= mn[L] + 0.05 * span
        if touch_hi != touch_lo:
            hint = 1 if touch_hi else -1
            if hint != msign:
                msign, method = hint, "part-name reach (tip-thinness said otherwise)"
        elif not (touch_hi and touch_lo):
            method += "; parts named barrel/etc touch neither end"
    # up sign: the vertex-density peak along U is the receiver/bore band (the spine);
    # the longer tail off the spine is the grip+magazine hanging BELOW it.
    hist, edges = np.histogram(co[:, U], bins=24,
                               range=(float(mn[U]), float(mx[U])))
    peak = int(hist.argmax())
    spine = float((edges[peak] + edges[peak + 1]) * 0.5)
    tail_hi = float(mx[U]) - spine
    tail_lo = spine - float(mn[U])
    usign = 1 if tail_lo >= tail_hi else -1
    uconf = round((max(tail_hi, tail_lo) + 1e-9) / (min(tail_hi, tail_lo) + 1e-9), 2)
    return {"L": L, "msign": msign, "U": U, "usign": usign, "right": W,
            "conf": conf, "uconf": uconf, "method": method, "ends": ends,
            "radii": radii, "spine": round(spine, 5),
            "ext": [round(float(x), 5) for x in ext], "mn": mn, "mx": mx,
            "cent": [round(float(x), 5) for x in co.mean(axis=0)]}


AXN = {0: "X", 1: "Y", 2: "Z"}


def gltf_of(b_axis, b_sign):
    """Blender axis+sign -> glTF (Y-up) axis+sign, because export_yup maps b->(x,z,-y)."""
    return {0: ("X", b_sign), 1: ("Z", -b_sign), 2: ("Y", b_sign)}[b_axis]


def align_matrix(ax):
    """Rotation onto Blender (+X right, -Y muzzle, +Z up) == glTF (+X, +Z muzzle, +Y up)."""
    y_s = Vector((0.0, 0.0, 0.0))
    z_s = Vector((0.0, 0.0, 0.0))
    y_s[ax["L"]] = -ax["msign"]
    z_s[ax["U"]] = ax["usign"]
    x_s = y_s.cross(z_s).normalized()
    src = Matrix((tuple(x_s), tuple(y_s), tuple(z_s))).transposed()
    if abs(src.determinant() - 1.0) > 1e-4:
        pprint("   WARN align determinant %.3f - skipping reorient" % src.determinant())
        return Matrix.Identity(4)
    return src.inverted().to_4x4()


def bake_transform(ob, mat):
    """Fold `mat` into the mesh data (positions *and* split normals) and stay at identity.

    Deliberately done through transform_apply rather than Mesh.transform(): the raw data
    call is not guaranteed to rotate custom split normals, and the engine bakes its
    lighting out of NORMAL, so a stale normal is a visible defect, not a rounding error.
    """
    ob.matrix_basis = mat.copy()
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    bpy.context.view_layer.objects.active = ob
    return ob


def extents_gltf(ext_blender):
    e = list(ext_blender)
    return [e[0], e[2], e[1]]


def centre_scale(ob, mode):
    """Bake bbox-centred-on-origin (+ optional longest-extent=1.0) into the vertices."""
    me = ob.data
    co = positions(me)
    mn, mx = co.min(axis=0), co.max(axis=0)
    s = 1.0 / float((mx - mn).max()) if mode == "longest" else 1.0
    centre = (mn + mx) * 0.5
    # p' = s*(p - centre): scale first, then translate by the *scaled* offset.
    bake_transform(ob, Matrix.Translation((-centre[0] * s, -centre[1] * s,
                                           -centre[2] * s))
                   @ Matrix.Diagonal((s, s, s, 1.0)))
    return round(s, 6)


def export_one(ob, out, tag):
    """One dense, single-primitive, untransformed GLB - then read it back."""
    me = ob.data
    ob.name = me.name = "gun_" + tag
    ob.parent = None
    ob.matrix_world = Matrix.Identity(4)
    me.update()
    bpy.context.view_layer.update()
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    ex = bpy.ops.export_scene.gltf
    kw = dict(filepath=out, export_format="GLB", use_selection=True, export_yup=True,
              export_apply=True, export_normals=True, export_texcoords=True,
              export_vertex_color="NAME", export_vertex_color_name="Col",
              export_materials="NONE", export_extras=False, export_animations=False,
              export_morph=False, export_skins=False)
    extra = {"export_all_vertex_colors": False,
             "export_active_vertex_color_when_no_material": True,
             "export_image_format": "NONE", "export_unused_images": False}
    names = [p.identifier for p in ex.get_rna_type().properties.values()]
    kw.update({k: v for k, v in extra.items() if k in names})
    ex(**kw)
    return glb_read(out)


# ------------------------------------------------------------------------ one file
def process(path, out_dir, opts):
    stem = os.path.splitext(os.path.basename(path))[0]
    tag = ascii_stem(stem)
    rep = {"src": os.path.basename(path), "out_name": tag + ".glb", "ok": False,
           "notes": []}
    _FACTORS.clear()
    _FACTORS.update(glb_factors(path))
    _TEX_CACHE.clear()          # image names repeat across files (Image_0, ...) - never
    reset_scene()               # let a stale LUT from the previous gun tint this one
    try:
        bpy.ops.import_scene.gltf(filepath=path, merge_vertices=False)
    except Exception:
        bpy.ops.import_scene.gltf(filepath=path)
    objs = [o for o in bpy.data.objects if o.type == "MESH"]
    if not objs:
        rep["notes"].append("NO MESH OBJECTS")
        return rep
    for o in objs:                             # hidden/excluded objects cannot be joined
        try:
            o.hide_set(False)
        except Exception:
            pass
        o.hide_viewport = False
        o.hide_render = False
    bpy.context.view_layer.update()
    rep["src_verts"] = sum(len(o.data.vertices) for o in objs)
    rep["src_tris"] = sum(sum(len(p.vertices) - 2 for p in o.data.polygons) for o in objs)
    rep["src_objs"] = len(objs)
    # stray geometry: these Sketchfab cuts ship material-less background spheres that
    # are BIGGER than the gun itself (Icosphere 2.0 units in rpk-16, ak104, sv98, pkm),
    # and at least one ships a flat "Floor" card (svd_63: 1.702 x 1.702 x 0.000, 4 verts)
    # which out-measures the rifle and steals the length axis from under the bbox test.
    box = {o.name: world_bbox(o) for o in objs}
    big = max((float((box[o.name][1] - box[o.name][0]).max()) for o in objs), default=0.0)
    junk = [o for o in objs if not [m for m in o.data.materials if m]]
    for o in objs:
        if o in junk:
            continue
        e = np.sort(box[o.name][1] - box[o.name][0])
        if len(o.data.vertices) <= 16 and e[0] <= 1e-4 * max(e[2], 1e-9) \
                and e[1] >= 0.5 * max(big, 1e-9):
            junk.append(o)
            rep.setdefault("backdrops", []).append(
                "%s ext=%s v=%d" % (o.name, np.round(e, 3).tolist(),
                                    len(o.data.vertices)))
    # Duplicated-assembly detector: svd_63 is a product shot holding TWO copies of the
    # rifle 90 degrees apart plus a detached scope.  No bbox rule can orient that, and no
    # automatic pick is safe (these guns ship as 60-88 separate parts, so connected-
    # component filtering would delete the gun), so flag it for a human.
    heavy = [o for o in objs if len(o.data.vertices) > 2000]
    dups = []
    for i, o in enumerate(heavy):
        for p in heavy[i + 1:]:
            a, b = len(o.data.vertices), len(p.data.vertices)
            if abs(a - b) > 0.02 * max(a, b):
                continue
            ca = (box[o.name][0] + box[o.name][1]) / 2.0
            cp = (box[p.name][0] + box[p.name][1]) / 2.0
            if float(np.linalg.norm(cp - ca)) <= 0.25 * max(big, 1e-9):
                dups.append("%s/%s (%d/%d verts)" % (o.name, p.name, a, b))
    if dups:
        rep["notes"].append("POSSIBLE DUPLICATED ASSEMBLY: %s - the merged mesh's length "
                            "and up axes are then meaningless; check the screenshot and "
                            "delete the extra copy" % "; ".join(dups[:3]))
        rep["dup_warn"] = True
    if junk and not opts["keep_junk"] and len(junk) < len(objs):
        rep["notes"].append("dropped %d stray object(s) [%s]: %s"
                            % (len(junk),
                               "material-less" if not rep.get("backdrops") else
                               "material-less + flat backdrop",
                               ", ".join(sorted(o.name for o in junk)[:5])))
        bpy.ops.object.select_all(action="DESELECT")
        for o in junk:
            o.select_set(True)
        bpy.context.view_layer.objects.active = junk[0]
        bpy.ops.object.delete()
        objs = [o for o in bpy.data.objects if o.type == "MESH"]
    for o in objs:                             # keep the world pose, drop the node graph
        mw = o.matrix_world.copy()
        o.parent = None
        o.matrix_parent_inverse = Matrix.Identity(4)
        o.matrix_world = mw
    bpy.context.view_layer.update()
    hints = []
    for o in objs:
        lo, hi = world_bbox(o)
        hints.append((o.name, lo, hi))
    # 1. modifiers -> real geometry  2. node transforms -> vertices  3. join -> 1 mesh
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    objs = [o for o in bpy.context.selected_objects]
    bpy.context.view_layer.objects.active = objs[0]
    if any(len(o.modifiers) for o in objs):
        try:
            bpy.ops.object.convert(target="MESH")
        except Exception as e:
            rep["notes"].append("convert(modifiers) failed: %s" % e)
        objs = [o for o in bpy.context.selected_objects if o.type == "MESH"]
        if not objs:
            rep["notes"].append("CONVERT LOST EVERYTHING")
            return rep
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    if len(objs) > 1:
        bpy.context.view_layer.objects.active = objs[0]
        bpy.ops.object.join()
    ob = bpy.context.view_layer.objects.active
    if ob is None or ob.type != "MESH":
        rep["notes"].append("JOIN FAILED")
        return rep
    me = ob.data
    rep["joined"] = "%d obj -> %dv/%dpoly, %d slots, %d uv layers" % (
        len(objs), len(me.vertices), len(me.polygons), len(me.materials),
        len(me.uv_layers))
    for o in [x for x in bpy.data.objects if x.type == "MESH" and x != ob]:
        n = len(o.data.vertices)
        has_mat = bool([m for m in o.data.materials if m])
        if not has_mat:                       # stray background sphere, not gun parts
            rep["notes"].append("deleted un-joined material-less object %s (%d verts)"
                                % (o.name, n))
            try:
                bpy.data.objects.remove(o)
            except Exception as e:
                rep["notes"].append("  could not delete: %s" % e)
            continue
        # real geometry that somehow escaped the join: pull it in, never silently drop it
        rep["notes"].append("WARNING un-joined mesh object with materials: %s (%d verts) "
                            "- joining" % (o.name, n))
        try:
            if not o.users_collection:
                bpy.context.scene.collection.objects.link(o)
            o.hide_set(False)
            o.hide_viewport = False
            bpy.context.view_layer.update()
            bpy.ops.object.select_all(action="DESELECT")
            ob.select_set(True)
            o.select_set(True)
            bpy.context.view_layer.objects.active = ob
            bpy.ops.object.join()
            ob = bpy.context.view_layer.objects.active
            me = ob.data
        except Exception as e:
            rep["notes"].append("  join of leftover failed: %s" % e)
    # 2. clean, then colour bake (before decimation destroys the UV correspondence)
    uv_name = None
    if len(me.uv_layers):
        uv_name = me.uv_layers.active.name if me.uv_layers.active else me.uv_layers[0].name
    triangulate(me)
    rep["clean"] = clean_mesh(me)
    rep["budget_before"] = mesh_stats(me)
    rep["colour"] = bake_vertex_colour(ob)
    for n in rep["colour"]["notes"]:
        if "slot" not in n:
            rep["notes"].append(n)
    # 3. single UV layer, no material slots (slots would split the primitive)
    if uv_name:
        for ul in list(me.uv_layers):
            if ul.name != uv_name:
                me.uv_layers.remove(ul)
        me.uv_layers[uv_name].active = True
        me.uv_layers[uv_name].active_render = True
    else:
        rep["notes"].append("source has NO UVs: TEXCOORD_0 written as a planar fallback")
        for ul in list(me.uv_layers):
            me.uv_layers.remove(ul)
        me.uv_layers.new(name="UVMap")
        project_planar_uv(me)
    if len(me.materials):
        nslot = len(me.materials)
        try:
            me.materials.clear()
        except Exception:
            while len(me.materials):
                me.materials.remove(len(me.materials) - 1)
        rep["notes"].append("cleared %d material slot(s): the exporter splits primitives "
                            "by slot even with export_materials=NONE" % nslot)
    # 4. orientation of the SOURCE (what the engine would have had to cope with)
    ax = detect_axes(me, hints)
    bl, bu = gltf_of(ax["L"], ax["msign"]), gltf_of(ax["U"], ax["usign"])
    rep["src_extents_blender"] = ax["ext"]
    rep["src_barrel"] = "%s%s (blender %s%s, thinness ends %+.2f/%+.2f, %s)" % (
        bl[0], "+" if bl[1] > 0 else "-", AXN[ax["L"]], "+" if ax["msign"] > 0 else "-",
        ax["ends"][1], ax["ends"][-1], ax["method"])
    rep["src_up"] = "%s%s (blender %s%s, spine tails %.4f below / %.4f above, conf %s)" % (
        bu[0], "+" if bu[1] > 0 else "-", AXN[ax["U"]], "+" if ax["usign"] > 0 else "-",
        abs(ax["spine"] - ax["mn"][ax["U"]]), abs(ax["mx"][ax["U"]] - ax["spine"]),
        ax["uconf"])
    if opts["reorient"]:
        bake_transform(ob, align_matrix(ax))
        me = ob.data
        rep["notes"].append("baked canonical rotation: muzzle -> glTF +Z, up -> glTF +Y")
    # 5..8. decimate -> normalise -> export -> measure the FILE, and repeat while the
    # exported vertex count is over budget.  Decimating on the Blender-side vertex count
    # is not enough: glTF has one vertex per (position, normal, uv) tuple, so a seamed
    # mesh grows on export - measured on this batch, pkp went 10610 -> 30574 and vss
    # 9343 -> 17725.  Only the on-disk accessor count is what the engine buffers.
    out = os.path.join(out_dir, tag + ".glb")
    rep["out"] = out
    vtarget, ttarget = opts["verts"], opts["tris"]
    g = None
    attempt = 0
    for attempt in range(4):
        rep["decimate"], rep["decimate_ratio"] = decimate_to(ob, vtarget, ttarget)
        me = ob.data
        rep["scale_applied"] = centre_scale(ob, opts["scale"])
        me = ob.data
        triangulate(me)
        rep["budget_after"] = mesh_stats(me)
        ax2 = detect_axes(me, [])          # independent check, not an echo of step 4
        al, au = gltf_of(ax2["L"], ax2["msign"]), gltf_of(ax2["U"], ax2["usign"])
        rep["barrel"] = "%s%s" % (al[0], "+" if al[1] > 0 else "-")
        rep["up"] = "%s%s" % (au[0], "+" if au[1] > 0 else "-")
        rep["barrel_check"] = "blender %s%s conf=%.2f (%s)" % (
            AXN[ax2["L"]], "+" if ax2["msign"] > 0 else "-", ax2["conf"], ax2["method"])
        rep["up_check"] = "blender %s%s uconf=%.2f" % (
            AXN[ax2["U"]], "+" if ax2["usign"] > 0 else "-", ax2["uconf"])
        rep["final_verts_blender"] = len(me.vertices)
        rep["final_tris_blender"] = len(me.polygons)
        rep["extents_gltf_XYZ"] = extents_gltf(ax2["ext"])
        rep["not_canonical"] = bool(opts["reorient"] and (
            ax2["L"] != 1 or ax2["msign"] != -1 or ax2["U"] != 2 or ax2["usign"] != 1))
        if rep["not_canonical"]:
            rep["notes"].append("ORIENTATION NOT CANONICAL after re-bake: muzzle along "
                                "blender %s%s, up blender %s%s"
                                % (AXN[ax2["L"]], "+" if ax2["msign"] > 0 else "-",
                                   AXN[ax2["U"]], "+" if ax2["usign"] > 0 else "-"))
        g = export_one(ob, out, tag)
        if g is None:
            rep["notes"].append("OUTPUT UNREADABLE")
            return rep
        if g["verts"] <= opts["verts"] and g["tris"] <= opts["tris"]:
            break
        shrink = min(opts["verts"] * 0.9 / max(g["verts"], 1),
                     opts["tris"] * 0.9 / max(g["tris"], 1))
        vtarget = max(1500, int(vtarget * shrink))
        ttarget = max(800, int(ttarget * shrink))
        rep["notes"].append("attempt %d exported %dv/%dt over the %dv/%dt cap "
                            "(export splits seamed verts), re-targeting %dv/%dt"
                            % (attempt + 1, g["verts"], g["tris"], opts["verts"],
                               opts["tris"], vtarget, ttarget))
    rep["glb"] = g
    rep["export_attempts"] = attempt + 1
    rep["verify"] = verify_output(out)
    v = rep["verify"]
    if v.get("error"):
        rep["notes"].append("VERIFY FAILED: %s" % v["error"])
    rep["ok"] = (not g["stride"] and not g["xform"] and g["mesh"] == 1
                 and g["prims"] == 1 and g["node"] <= 1
                 and set(g["attrs"]) >= {"POSITION", "NORMAL", "TEXCOORD_0", "COLOR_0"}
                 and g["verts"] <= opts["verts"] and g["tris"] <= opts["tris"]
                 and g["idx"] <= HARD_CAP_IDX and not g["img"] and not g["tex"]
                 and g["verts"] <= HARD_CAP_VERTS and not rep.get("not_canonical")
                 and not v.get("error") and v.get("pos_nan", 1) == 0
                 and v.get("color_over_1", 1) == 0 and v.get("distinct", 0) >= 2
                 and 0.9 <= v.get("normal_len_max", 0) <= 1.1
                 and v.get("normal_distinct", 0) >= 2
                 and max(v.get("off_centre", [9, 9, 9])) <= 0.02
                 and (opts["scale"] != "longest" or max(g["ext"]) <= 1.0001))
    if opts["shots"]:
        try:
            rep["shot"] = shoot(ob, opts["shot_dir"], tag)
        except Exception as e:
            rep["shot"] = "FAILED %s: %s" % (type(e).__name__, e)
            rep["notes"].append("screenshot failed: %s" % e)
    return rep


def project_planar_uv(me):
    """Deterministic triplanar-ish UVs so TEXCOORD_0 is never garbage."""
    uv = me.uv_layers[0]
    co = positions(me)
    for poly in me.polygons:
        n = np.abs(poly.normal)
        a, b = (1, 2) if n[0] >= max(n[1], n[2]) else ((0, 2) if n[1] >= n[2] else (0, 1))
        for li in poly.loop_indices:
            v = me.loops[li].vertex_index
            uv.data[li].uv = (float(co[v, a]) * 0.5, float(co[v, b]) * 0.5)
    try:
        uv.active = True
        uv.active_render = True
    except Exception:
        pass


def ascii_stem(stem):
    keep = "".join(c if c.isalnum() or c in "-_." else "_" for c in stem)
    keep = keep.strip("_.-") or "gun"
    return keep[:40]


# ------------------------------------------------------------------- output checking
def glb_read(path):
    """Parse exactly as much of the GLB as the engine's capability list needs."""
    with open(path, "rb") as fh:
        d = fh.read()
    if len(d) < 28 or struct.unpack_from("<I", d, 0)[0] != 0x46546C67:
        return None
    clen = struct.unpack_from("<I", d, 12)[0]
    j = json.loads(d[20:20 + clen].decode("utf-8", "replace").rstrip("\x00 "))
    accs = j.get("accessors", [])
    prims = [p for m in j.get("meshes", []) for p in m.get("primitives", [])]
    if not prims or not accs:
        return None
    a0 = prims[0].get("attributes", {})
    if "POSITION" not in a0:
        return None
    pacc = accs[a0["POSITION"]]
    ci = a0.get("COLOR_0")
    cacc = accs[ci] if ci is not None else {}
    return {"sizeMB": round(len(d) / 1048576.0, 2), "mesh": len(j.get("meshes", [])),
            "prims": len(prims), "node": len(j.get("nodes", [])),
            "verts": pacc.get("count", 0),
            "tris": sum(accs[p["indices"]].get("count", 0) // 3 for p in prims
                        if p.get("indices") is not None),
            "idx": sum(accs[p["indices"]].get("count", 0) for p in prims
                       if p.get("indices") is not None),
            "attrs": sorted(a0.keys()),
            "stride": bool([b for b in j.get("bufferViews", []) if b.get("byteStride")]),
            "xform": bool([n for n in j.get("nodes", [])
                           if any(k in n for k in ("matrix", "translation", "rotation",
                                                   "scale"))]),
            "tex": len(j.get("textures", [])), "img": len(j.get("images", [])),
            "mat": len(j.get("materials", [])),
            "ext": [round(pacc.get("max", [0] * 3)[i] - pacc.get("min", [0] * 3)[i], 4)
                    for i in range(3)],
            "color_type": cacc.get("type"), "color_ctype": cacc.get("componentType"),
            "color_norm": cacc.get("normalized", False),
            "ext_used": sorted(set(j.get("extensionsUsed", [])))}


# ------------------------------------------------------------------- output checking
def chunks(path):
    """(json dict, bin bytes) of a GLB."""
    with open(path, "rb") as fh:
        d = fh.read()
    if len(d) < 28 or struct.unpack_from("<I", d, 0)[0] != 0x46546C67:
        return None, b""
    clen = struct.unpack_from("<I", d, 12)[0]
    j = json.loads(d[20:20 + clen].decode("utf-8", "replace").rstrip("\x00 "))
    at = 20 + clen
    bin_data = b""
    while at + 8 <= len(d):
        ln, tag = struct.unpack_from("<II", d, at)
        if tag == 0x004E4942:
            bin_data = d[at + 8: at + 8 + ln]
            break
        at += 8 + ln + (4 - (ln % 4)) % 4
    return j, bin_data


def acc_read(j, bin_data, idx, comps):
    """Re-implement assets.rs::read_acc (componentType + normalized + both offsets)."""
    acc = j["accessors"][idx]
    bv = j["bufferViews"][acc.get("bufferView", 0)]
    off = bv.get("byteOffset", 0) + acc.get("byteOffset", 0)
    ct = acc.get("componentType", 5126)
    norm = acc.get("normalized", False)
    div = {5120: 127.0 if norm else 1.0, 5121: 255.0 if norm else 1.0,
           5122: 32767.0 if norm else 1.0, 5123: 65535.0 if norm else 1.0,
           5125: 1.0}.get(ct, 1.0)
    n = acc.get("count", 0) * comps
    out = np.empty(n, dtype=np.float64)
    ty = {5120: np.int8, 5121: np.uint8, 5122: np.int16, 5123: np.uint16,
          5125: np.uint32}.get(ct, np.float32)
    raw = np.frombuffer(bin_data, dtype=ty, count=n, offset=off).astype(np.float64)
    out[:len(raw)] = raw / div
    return out.reshape(-1, comps), acc


def verify_output(path):
    """What the engine will actually decode out of the finished file."""
    j, bin_data = chunks(path)
    if not j or not bin_data:
        return {"error": "unreadable"}
    prim = j["meshes"][0]["primitives"][0]
    a = prim["attributes"]
    out = {}
    pos, pacc = acc_read(j, bin_data, a["POSITION"], 3)
    out["pos_nan"] = int(np.count_nonzero(~np.isfinite(pos)))
    out["pos_range"] = [round(float(pos[:, i].min()), 3) for i in range(3)] + \
                       [round(float(pos[:, i].max()), 3) for i in range(3)]
    out["off_centre"] = [round(abs(out["pos_range"][i] + out["pos_range"][i + 3]), 3)
                         for i in range(3)]
    nrm, _ = acc_read(j, bin_data, a["NORMAL"], 3)
    ln = np.linalg.norm(nrm, axis=1)
    out["normal_len_min"] = round(float(ln.min()), 4)
    out["normal_len_max"] = round(float(ln.max()), 4)
    out["normal_distinct"] = int(len({tuple(np.round(v, 3)) for v in nrm[::max(1, len(nrm) // 300)]}))
    col, cacc = acc_read(j, bin_data, a["COLOR_0"],
                         4 if j["accessors"][a["COLOR_0"]].get("type") == "VEC4" else 3)
    lum = 0.2126 * col[:, 0] + 0.7152 * col[:, 1] + 0.0722 * col[:, 2]
    out["color_type"] = cacc.get("type")
    out["luma_min"] = round(float(lum.min()), 4)
    out["luma_max"] = round(float(lum.max()), 4)
    out["luma_mean"] = round(float(lum.mean()), 4)
    out["color_over_1"] = int((col > 1.001).sum())
    out["distinct"] = int(len({tuple(int(b) for b in (c[:3] * 63))
                              for c in col[::max(1, len(col) // 400)]}))
    uv, uacc = acc_read(j, bin_data, a["TEXCOORD_0"], 2)
    out["uv_range"] = [round(float(uv[:, i].min()), 3) for i in range(2)] + \
                      [round(float(uv[:, i].max()), 3) for i in range(2)]
    return out


# ------------------------------------------------------------------------ screenshots
def shoot(ob, shot_dir, tag):
    os.makedirs(shot_dir, exist_ok=True)
    sc = bpy.context.scene
    sc.render.engine = "BLENDER_WORKBENCH"
    sh = sc.display.shading
    for prop, val in (("color_type", "VERTEX"), ("light", "FLAT"),
                      ("show_shadows", False), ("show_cavity", False),
                      ("show_object_outline", False)):
        try:
            setattr(sh, prop, val)
        except Exception:
            pass
    sc.render.resolution_x = 720
    sc.render.resolution_y = 480
    co = positions(ob.data)
    mn, mx = co.min(axis=0), co.max(axis=0)
    c = Vector(((mx + mn) / 2.0).tolist())
    r = float((mx - mn).max()) * 0.5 + 1e-6
    made = []
    # Canonical Blender space after the re-bake is muzzle -> -Y, up -> +Z, so a camera on
    # -X sees the gun's right side with the muzzle to the right of frame and the sky above
    # (camera at +Y would give a useless end-on stub).
    for name, off in (("side", (-3.3 * r, 0.0, 0.12 * r)),
                      ("threequarter", (-2.2 * r, -2.4 * r, 1.4 * r))):
        cd = bpy.data.cameras.new("shotcam")
        cd.lens = 50.0
        cam = bpy.data.objects.new("shotcam", cd)
        bpy.context.collection.objects.link(cam)
        cam.location = c + Vector(off)
        dire = (c - cam.location).normalized()
        cam.rotation_euler = dire.to_track_quat("-Z", "Y").to_euler()
        sc.camera = cam
        out = os.path.join(shot_dir, "gun_ext_%s_%s.png" % (tag, name))
        sc.render.filepath = out
        try:
            bpy.ops.render.render(write_still=True)
            made.append(os.path.basename(out))
        except Exception as e:
            pprint("   WARN render %s failed: %s" % (out, e))
        bpy.data.objects.remove(cam)
        bpy.data.cameras.remove(cd)
    return ",".join(made)


# ------------------------------------------------------------------------------ main
def main():
    opts = {"in": opt("--in", r"D:\Rust\3D"),
            "out": opt("--out", r"D:\Rust\steel-front\assets\guns_ext"),
            "verts": int(opt("--verts", TARGET_VERTS)),
            "tris": int(opt("--tris", TARGET_TRIS)),
            "scale": opt("--scale", "longest"),
            "reorient": not has("--no-reorient"),
            "keep_junk": has("--keep-junk"),
            "shots": opt("--shots"),
            "shot_dir": opt("--shot-dir", r"D:\Rust\steel-front\screenshots"),
            "report": opt("--report", os.path.join(
                os.path.dirname(os.path.abspath(__file__)), "prep_guns_report.json"))}
    os.makedirs(opts["out"], exist_ok=True)
    only = opt("--only")
    only = [s.strip().lower() for s in only.split(",") if s.strip()] if only else None
    files = sorted(os.path.join(opts["in"], f) for f in os.listdir(opts["in"])
                   if f.lower().endswith(".glb"))
    if only:
        files = [f for f in files
                 if any(o in os.path.basename(f).lower() for o in only)]
    if not files:
        pprint("### prep_guns: no input files matched")
        return 1
    pprint("### prep_guns: %d file(s) from %s -> %s (cap %dv/%dt, scale=%s, reorient=%s)"
           % (len(files), opts["in"], opts["out"], opts["verts"], opts["tris"],
              opts["scale"], opts["reorient"]))
    reps = []
    for f in files:
        pprint("\n--- %s" % os.path.basename(f))
        try:
            r = process(f, opts["out"], dict(opts, shots=opts["shots"] == "all" or (
                bool(opts["shots"]) and opts["shots"].lower() in os.path.basename(f).lower())))
        except Exception as e:
            pprint("    !! FAILED %s: %s" % (type(e).__name__, e))
            traceback.print_exc()
            r = {"src": os.path.basename(f), "ok": False, "notes": ["%s: %s" % (
                type(e).__name__, e)]}
        reps.append(r)
        g = r.get("glb") or {}
        pprint("    -> %s ok=%s %sv/%st stride=%s xform=%s COLOR_0=%s/%s%s barrel=%s up=%s"
               % (r.get("out", "-"), r.get("ok"), g.get("verts"), g.get("tris"),
                  g.get("stride"), g.get("xform"), g.get("color_type"),
                  g.get("color_ctype"), "n" if g.get("color_norm") else "",
                  r.get("barrel"), r.get("up")))
        pprint("       colour=%s luma %s..%s distinct~%s | src %sv/%st objs %s | %s | clean %s"
               % (r.get("colour", {}).get("kinds"), r.get("colour", {}).get("luma_min"),
                  r.get("colour", {}).get("luma_max"),
                  r.get("colour", {}).get("distinct_colours_sampled"),
                  r.get("src_verts"), r.get("src_tris"), r.get("src_objs"),
                  r.get("joined"), r.get("clean")))
        pprint("       src barrel=%s | up=%s | decim ratio=%s %s | used=%s tries=%s | %s | shot=%s"
               % (r.get("src_barrel"), r.get("src_up"), r.get("decimate_ratio"),
                  r.get("decimate"), r.get("budget_after"), r.get("export_attempts"),
                  r.get("barrel_check"), r.get("shot", "")))
        for n in r.get("notes", []):
            if "slot" not in n:
                pprint("       . %s" % n)
        v = r.get("verify") or {}
        if v:
            pprint("       on-disk: colour %s luma %s..%s mean %s distinct~%s over1=%s | "
                   "normal len %s..%s distinct~%s | off-centre %s | uv %s | pos_nan=%s"
                   % (v.get("color_type"), v.get("luma_min"), v.get("luma_max"),
                      v.get("luma_mean"), v.get("distinct"), v.get("color_over_1"),
                      v.get("normal_len_min"), v.get("normal_len_max"),
                      v.get("normal_distinct"), v.get("off_centre"), v.get("uv_range"),
                      v.get("pos_nan")))
    pprint("\n=== SUMMARY (out / verts / tris / idx / MB / extXYZ / barrel / up / ok)")
    for r in reps:
        g = r.get("glb") or {}
        pprint("%-34s %7s %7s %8s %6s  %-22s %-8s %-6s %s"
               % (r["src"][:34], g.get("verts"), g.get("tris"), g.get("idx"),
                  g.get("sizeMB"), g.get("ext"), r.get("barrel"), r.get("up"),
                  "OK" if r.get("ok") else "NOT CLEAN"))
    try:
        with open(opts["report"], "w", encoding="utf-8") as fh:
            json.dump(reps, fh, indent=1, ensure_ascii=False, default=str)
        pprint("report -> %s" % opts["report"])
    except Exception as e:
        pprint("WARN report write failed: %s" % e)
    bad = [r["src"] for r in reps if not r.get("ok")]
    pprint("NOT CLEAN: %s" % (bad or "none"))
    return 0


sys.exit(main())
