"""Steel Front prop kit — headless Blender asset generator.

Conventions (do not change without updating the engine loader):
  * 1 Blender unit = 1 metre.
  * Object origin sits at the centre of the footprint on the ground plane (z = 0).
    Everything else has z >= 0, so a prop can be dropped at any terrain height as-is.
  * Authored +Z up; the glTF exporter converts to the Y-up convention on export.
  * Every mesh carries a POINT-domain byte colour attribute named "Col" (exported as
    COLOR_0) and a UVMap, because the engine's vertex layout is
    stride=32 pos@0 color@12 uv@24.

Run:  blender.exe --background --python gen_props.py -- --out <dir> [--only a,b,c]
"""

import sys
import os
import math
import random

import bpy
import bmesh
from mathutils import Matrix, Vector


# ----------------------------------------------------------------------------- palette
# Muted WWII-European theatre tones. These are linear-space-ish sRGB values; the engine
# multiplies them by its own lighting, so keep them mid-grey-ish and let light do the work.
C = {
    "brick":        (0.36, 0.21, 0.17),
    "brick_dark":   (0.27, 0.16, 0.14),
    "plaster":      (0.55, 0.50, 0.42),
    "plaster_warm": (0.60, 0.52, 0.41),
    "concrete":     (0.44, 0.43, 0.41),
    "concrete_dk":  (0.32, 0.31, 0.30),
    "stone":        (0.48, 0.46, 0.42),
    "roof_tile":    (0.30, 0.18, 0.15),
    "roof_slate":   (0.22, 0.22, 0.24),
    "wood":         (0.34, 0.24, 0.14),
    "wood_dark":    (0.22, 0.15, 0.09),
    "glass_dark":   (0.09, 0.11, 0.13),
    "metal":        (0.30, 0.31, 0.33),
    "metal_rust":   (0.32, 0.20, 0.13),
    "sandbag":      (0.46, 0.40, 0.27),
    "sandbag_old":  (0.38, 0.33, 0.23),
    "foliage_a":    (0.17, 0.27, 0.13),
    "foliage_b":    (0.22, 0.32, 0.15),
    "foliage_c":    (0.13, 0.21, 0.11),
    "bark":         (0.20, 0.15, 0.10),
    "dirt":         (0.30, 0.25, 0.18),
    "interior":     (0.05, 0.05, 0.06),
    # shipping-container livery: weathered paint, never saturated primary colours
    "oxide":        (0.32, 0.14, 0.10),
    "container_navy": (0.11, 0.15, 0.22),
    "container_green": (0.16, 0.21, 0.15),
    "asphalt":      (0.16, 0.16, 0.17),
    "road_mark":    (0.62, 0.60, 0.52),
}


# ----------------------------------------------------------------------------- scene
def reset_scene():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete()
    for block in (bpy.data.meshes, bpy.data.materials, bpy.data.lights, bpy.data.cameras):
        for item in list(block):
            if item.users == 0:
                block.remove(item)


# ----------------------------------------------------------------------------- builders
# Every builder appends (verts, tris, per-vertex-colour) into a Part list. Keeping this
# as plain data (rather than live Blender objects) means joining is trivial and the
# winding order stays under our control.
class Part:
    def __init__(self, name):
        self.name = name
        self.verts = []          # list[(x, y, z)]
        self.tris = []           # list[(i, j, k)] CCW when seen from outside
        self.colour = [None] * 0  # parallel to verts

    def add_quad(self, a, b, c, d, col):
        i = len(self.verts)
        self.verts += [a, b, c, d]
        self.colour += [col, col, col, col]
        self.tris += [(i, i + 1, i + 2), (i, i + 2, i + 3)]

    def add_tri(self, a, b, c, col):
        i = len(self.verts)
        self.verts += [a, b, c]
        self.colour += [col, col, col]
        self.tris.append((i, i + 1, i + 2))


def _xf(m, p):
    v = Vector(p)
    r = m @ v
    return (r.x, r.y, r.z)


def box(part, center, size, col, rot_z=0.0):
    """Axis-aligned box, optionally yawed. center is the box centre."""
    cx, cy, cz = center
    sx, sy, sz = size
    hx, hy, hz = sx * 0.5, sy * 0.5, sz * 0.5
    m = Matrix.Rotation(rot_z, 4, "Z") @ Matrix.Translation((cx, cy, cz))
    #        -y  -x  0 back-left ... enumerate the 8 corners consistently
    c = [
        _xf(m, (-hx, -hy, -hz)), _xf(m, (hx, -hy, -hz)),
        _xf(m, (hx, hy, -hz)),   _xf(m, (-hx, hy, -hz)),
        _xf(m, (-hx, -hy, hz)),  _xf(m, (hx, -hy, hz)),
        _xf(m, (hx, hy, hz)),    _xf(m, (-hx, hy, hz)),
    ]
    # outward-CCW winding per face, checked against the right-hand rule
    part.add_quad(c[1], c[0], c[3], c[2], col)   # -Z base
    part.add_quad(c[2], c[3], c[7], c[6], col)   # +Y
    part.add_quad(c[6], c[7], c[4], c[5], col)   # +Z top
    part.add_quad(c[5], c[4], c[0], c[1], col)   # -Y
    part.add_quad(c[4], c[7], c[3], c[0], col)   # -X
    part.add_quad(c[5], c[1], c[2], c[6], col)   # +X


def prism(part, center, size, col, slope_axis="x"):
    """Closed wedge with a single-pitch top (awnings, shed roofs, pentices).

    Winding is outward-CCW on every face, verified per-face against the right-hand rule:
    the engine derives its shading normal from screen-space derivatives, so an inverted
    face lights black rather than failing loudly.
    """
    cx, cy, cz = center
    sx, sy, sz = size
    hx, hy = sx * 0.5, sy * 0.5
    zb = cz - sz * 0.5
    if slope_axis == "x":
        t = (0.0, 1.0, 1.0, 0.0)
    else:
        t = (0.0, 0.0, 1.0, 1.0)
    corners = ((cx - hx, cy - hy), (cx + hx, cy - hy),
               (cx + hx, cy + hy), (cx - hx, cy + hy))
    bot = [(x, y, zb) for x, y in corners]
    top = [(corners[i][0], corners[i][1], zb + sz * t[i]) for i in range(4)]
    part.add_quad(top[0], top[1], top[2], top[3], col)
    part.add_quad(bot[0], bot[3], bot[2], bot[1], col)
    for i in range(4):
        j = (i + 1) % 4
        part.add_quad(bot[i], bot[j], top[j], top[i], col)


def cylinder(part, p0, p1, r0, r1, col, segs=10, caps=True):
    """Tapered cylinder between two points (tapered trunk, barrel staves, posts)."""
    p0 = Vector(p0); p1 = Vector(p1)
    axis = p1 - p0
    length = axis.length
    if length < 1e-6:
        return
    axis.normalize()
    # build a basis where local Z is the cylinder axis
    up = Vector((0, 0, 1)) if abs(axis.z) < 0.9 else Vector((1, 0, 0))
    x = axis.cross(up).normalized()
    y = axis.cross(x)
    base = len(part.verts)
    for i in range(segs):
        t = 2.0 * math.pi * i / segs
        off = x * math.cos(t) + y * math.sin(t)
        part.verts.append((p0 + off * r0).to_tuple()); part.colour.append(col)
        part.verts.append((p1 + off * r1).to_tuple()); part.colour.append(col)
    for i in range(segs):
        a = base + i * 2
        b = base + ((i + 1) % segs) * 2
        part.tris += [(a, b, b + 1), (a, b + 1, a + 1)]
    if caps:
        c0 = len(part.verts); part.verts.append(tuple(p0)); part.colour.append(col)
        c1 = len(part.verts); part.verts.append(tuple(p1)); part.colour.append(col)
        for i in range(segs):
            a = base + i * 2
            b = base + ((i + 1) % segs) * 2
            part.tris.append((c0, b, a))
            part.tris.append((c1, a + 1, b + 1))


def icosphere(part, center, radius, col, subdiv=1, flatten=1.0, jitter=0.0, rng=None,
              stretch=None):
    """Flat-shaded blob for foliage canopies, rubble chunks and sandbags.

    `stretch` is an optional per-axis multiplier applied after the unit sphere, so a
    sandbag can be long and squat while a canopy stays round.
    """
    verts, faces = _ico_base()
    if jitter and rng is not None:
        verts = [(v[0] + rng.uniform(-jitter, jitter),
                  v[1] + rng.uniform(-jitter, jitter),
                  v[2] + rng.uniform(-jitter, jitter)) for v in verts]
    for _ in range(subdiv):
        verts, faces = _ico_subdiv(verts, faces, jitter, rng)
    sx, sy, sz = stretch if stretch else (1.0, 1.0, 1.0)
    base = len(part.verts)
    cx, cy, cz = center
    for v in verts:
        part.verts.append((cx + v[0] * radius * sx, cy + v[1] * radius * sy,
                           cz + v[2] * radius * sz * flatten))
        part.colour.append(col)
    for f in faces:
        part.tris.append((base + f[0], base + f[1], base + f[2]))


_ICO_CACHE = {}


def _ico_base():
    if "base" not in _ICO_CACHE:
        t = (1.0 + 5.0 ** 0.5) / 2.0
        raw = [(-1, t, 0), (1, t, 0), (-1, -t, 0), (1, -t, 0),
               (0, -1, t), (0, 1, t), (0, -1, -t), (0, 1, -t),
               (t, 0, -1), (t, 0, 1), (-t, 0, -1), (-t, 0, 1)]
        n = [Vector(v).normalized() for v in raw]
        f = [(0, 11, 5), (0, 5, 1), (0, 1, 7), (0, 7, 10), (0, 10, 11),
             (1, 5, 9), (5, 11, 4), (11, 10, 2), (10, 7, 6), (7, 1, 8),
             (3, 9, 4), (3, 4, 2), (3, 2, 6), (3, 6, 8), (3, 8, 9),
             (4, 9, 5), (2, 4, 11), (6, 2, 10), (8, 6, 7), (9, 8, 1)]
        _ICO_CACHE["base"] = ([tuple(v) for v in n], f)
    return _ICO_CACHE["base"]


def _ico_subdiv(verts, faces, jitter, rng):
    out_v = list(verts)
    cache = {}
    out_f = []

    def mid(a, b):
        key = (min(a, b), max(a, b))
        if key in cache:
            return cache[key]
        va = Vector(verts[a]); vb = Vector(verts[b])
        m = (va + vb) * 0.5
        if jitter and rng is not None:
            m += Vector((rng.uniform(-jitter, jitter), rng.uniform(-jitter, jitter),
                         rng.uniform(-jitter, jitter)))
        m.normalize()
        idx = len(out_v)
        out_v.append(tuple(m))
        cache[key] = idx
        return idx

    for a, b, c in faces:
        ab = mid(a, b); bc = mid(b, c); ca = mid(c, a)
        out_f += [(a, ab, ca), (b, bc, ab), (c, ca, bc), (ab, bc, ca)]
    return out_v, out_f


# ----------------------------------------------------------------------------- mesh out
def box_project_uv(mesh):
    """Deterministic planar-per-face UVs. Avoids bpy.ops context overrides (fragile in
    --background) and gives the engine a valid TEXCOORD_0 for its detail modulation."""
    uv = mesh.uv_layers.new(name="UVMap")
    for poly in mesh.polygons:
        n = Vector(poly.normal)
        ax = max(range(3), key=lambda i: abs(n[i]))
        for li in poly.loop_indices:
            co = mesh.vertices[mesh.loops[li].vertex_index].co
            if ax == 0:      u, v = co.y, co.z
            elif ax == 1:    u, v = co.x, co.z
            else:            u, v = co.x, co.y
            uv.data[li].uv = (u * 0.5, v * 0.5)


def finish_object(part, name):
    """Turn a Part into a real mesh object with normals, colour attribute and UVs."""
    me = bpy.data.meshes.new(name)
    me.from_pydata(part.verts, [], part.tris)
    me.validate()
    me.update()
    for p in me.polygons:
        p.use_smooth = False
    # Colour attribute must exist before box_project_uv so export picks it up per-vertex.
    ca = me.color_attributes.new(name="Col", type="BYTE_COLOR", domain="POINT")
    for i, col in enumerate(part.colour):
        ca.data[i].color = (col[0], col[1], col[2], 1.0)
    box_project_uv(me)
    ob = bpy.data.objects.new(name, me)
    bpy.context.collection.objects.link(ob)
    return ob


def export_glb(ob, out_path, export_yup=True):
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    bpy.ops.export_scene.gltf(
        filepath=out_path,
        export_format="GLB",
        use_selection=True,
        export_yup=export_yup,
        export_apply=True,
        export_normals=True,
        export_texcoords=True,
        export_vertex_color="NAME",
        export_vertex_color_name="Col",
        export_materials="NONE",
        export_extras=False,
        export_animations=False,
        export_morph=False,
        export_skins=False,
    )


# ============================================================== asset definitions
def asset_crate():
    """Supply crate: planked box with a frame, lid lip and corner braces."""
    p = Part("crate_wood")
    col, frame = C["wood"], C["wood_dark"]
    box(p, (0, 0, 0.30), (0.60, 0.60, 0.60), col)
    # lid lip
    box(p, (0, 0, 0.615), (0.64, 0.64, 0.03), frame)
    # edge battens on all four faces (reads as a constructed object at any distance)
    for sx in (-1, 1):
        for sy in (-1, 1):
            box(p, (sx * 0.31, sy * 0.31, 0.30), (0.045, 0.045, 0.62), frame)
    for z in (0.10, 0.50):
        box(p, (0, 0.31, z), (0.62, 0.04, 0.07), frame)
        box(p, (0, -0.31, z), (0.62, 0.04, 0.07), frame)
        box(p, (0.31, 0, z), (0.04, 0.62, 0.07), frame)
        box(p, (-0.31, 0, z), (0.04, 0.62, 0.07), frame)
    return p, (0.64, 0.64, 0.65)


def asset_barrel():
    """Metal drum with chimes (the two rolled ribs) and a lid ring."""
    p = Part("barrel_metal")
    col = C["metal_rust"]
    cylinder(p, (0, 0, 0.02), (0, 0, 0.86), 0.26, 0.26, col, segs=14)
    for z in (0.28, 0.58):
        cylinder(p, (0, 0, z - 0.045), (0, 0, z + 0.045), 0.275, 0.275,
                 C["metal"], segs=14, caps=False)
    cylinder(p, (0, 0, 0.87), (0, 0, 0.90), 0.245, 0.245, C["metal"], segs=14)
    return p, (0.55, 0.55, 0.90)


def asset_tree():
    """Deciduous tree: tapered trunk, branches that carry leaf clusters low down,
    and a wide multi-blob canopy.

    The old procedural trees read as a canopy on stilts: no trunk volume, and one flat
    disc of foliage on top. Two things actually make a tree read at 100 m — a trunk that
    visibly tapers, and a canopy whose silhouette is irregular, which needs overlapping
    blobs at different greens rather than one sphere.
    """
    rng = random.Random(7)
    p = Part("tree_oak")
    h = 3.5
    cylinder(p, (0, 0, 0.0), (0.12, -0.07, h + 1.4), 0.34, 0.10, C["bark"], segs=9)
    # root flare so the base does not look like a drilled post
    cylinder(p, (0, 0, 0.0), (0, 0, 0.7), 0.50, 0.32, C["bark"], segs=9, caps=False)
    # limbs branching from mid-trunk: these are what break the lollipop silhouette
    limbs = [(0.5, 1.30, 0.55, 1.9), (2.5, 1.45, -0.5, 2.0),
             (4.2, 1.15, 0.2, 1.6), (1.6, 0.95, -0.9, 1.4),
             (3.4, 1.40, 0.9, 1.8), (5.4, 1.10, -0.3, 1.5)]
    for ang, ln, yb, zb in limbs:
        bx, by = math.cos(ang) * ln, math.sin(ang) * ln
        cylinder(p, (0.04, 0.0, h - 0.35 + zb * 0.25), (bx, by, h + zb * 0.6),
                 0.13, 0.05, C["bark"], segs=6)
    # canopy: one dominant mass plus skirt blobs that hang down over the limbs
    blobs = [(0.0, 0.0, h + 1.90, 2.30, C["foliage_b"]),
             (1.55, 0.55, h + 1.15, 1.55, C["foliage_a"]),
             (-1.45, -0.65, h + 1.25, 1.60, C["foliage_c"]),
             (0.25, -1.60, h + 1.35, 1.45, C["foliage_a"]),
             (-0.45, 1.55, h + 1.45, 1.50, C["foliage_c"]),
             (1.05, -0.95, h + 2.55, 1.35, C["foliage_b"]),
             (-1.15, 0.95, h + 2.45, 1.30, C["foliage_a"]),
             (0.15, 0.25, h + 3.20, 1.45, C["foliage_c"])]
    for bx, by, bz, br, bc in blobs:
        icosphere(p, (bx, by, bz), br, bc, subdiv=1, flatten=0.78,
                  jitter=0.13, rng=rng, stretch=(1.0, 1.0, 1.0))
    return p, (6.6, 6.6, h + 4.7)


def asset_building(w=12.0, d=9.0, floors=2, bays=4, wall="plaster", roof="roof_tile",
                   seed=11, name="building_block"):
    """Two-storey urban block module with genuine window openings.

    This is the direct answer to defect D11. The previous attempt drew window bands as
    separate thin boxes in front of a flat facade, so from the street they looked like
    cantilevered fins, and at grazing angles their front and back faces fought in the
    depth buffer and produced V-shaped comb artifacts. Here the facade is *assembled*
    from piers and spandrels, so the openings are real holes with real reveals; the
    glass sits recessed behind the wall plane and nothing floats.

    Parametrised so one generator yields the whole street: identical footprints side by
    side read as a clone army, which is as bad as the current mess.
    """
    rng = random.Random(seed)
    p = Part(name)
    W, D = w, d
    FLOOR_H = 3.4
    FLOORS = floors
    H = FLOOR_H * FLOORS                  # wall height
    T = 0.30                              # facade thickness
    BAYS = bays                           # window bays per long side
    col = C[wall]

    def facade(off_sign, axis, bays, ground_shop):
        """Lay piers + spandrels along one side, leaving real openings.

        `axis` is the direction the facade *runs* along; it stands at a fixed distance
        from the centre measured on the *other* horizontal axis. Those two lengths come
        from different footprint dimensions, so they are kept separate deliberately.
        """
        run, dist = (W, D * 0.5) if axis == "x" else (D, W * 0.5)
        half = run * 0.5
        pier_w, sill_h, head_h = 0.75, 1.05, 0.75
        bay = run / bays
        wall_at = dist - T * 0.5
        glass_at = dist - 0.12
        lip_at = dist + 0.06
        for i in range(bays):
            c0 = -half + i * bay
            win_w = bay - pier_w
            cx = c0 + bay * 0.5
            for fl in range(FLOORS):
                z0 = fl * FLOOR_H
                z1 = z0 + FLOOR_H
                if ground_shop and fl == 0:
                    _place(cx, win_w + 0.3, z0 + 0.15, z1 - 0.45, C["glass_dark"],
                           axis, off_sign, dist - 0.10)
                    _wall(c0, c0 + pier_w * 0.5, z0, z1, col, axis, off_sign, wall_at)
                    _wall(c0 + bay - pier_w * 0.5, c0 + bay, z0, z1, col,
                          axis, off_sign, wall_at)
                    _wall(cx - win_w * 0.5, cx + win_w * 0.5, z1 - 0.45, z1, col,
                          axis, off_sign, wall_at)
                    continue
                _wall(cx - win_w * 0.5, cx + win_w * 0.5, z0, z0 + sill_h, col,
                      axis, off_sign, wall_at)
                _wall(cx - win_w * 0.5, cx + win_w * 0.5, z1 - head_h, z1, col,
                      axis, off_sign, wall_at)
                _place(cx, win_w, z0 + sill_h, z1 - head_h, C["glass_dark"],
                       axis, off_sign, glass_at)
                _lip(cx, win_w + 0.16, z0 + sill_h, 0.10, axis, off_sign, lip_at)
            _wall(c0, c0 + pier_w * 0.5, 0, H, col, axis, off_sign, wall_at)
        _wall(half - pier_w * 0.5, half, 0, H, col, axis, off_sign, wall_at)

    def _wall(a0, a1, z0, z1, cc, axis, off_sign, at):
        if a1 - a0 <= 1e-4 or z1 - z0 <= 1e-4:
            return
        mid_a = (a0 + a1) * 0.5
        w = a1 - a0
        if axis == "x":
            box(p, (mid_a, off_sign * at, (z0 + z1) * 0.5), (w, T, z1 - z0), cc)
        else:
            box(p, (off_sign * at, mid_a, (z0 + z1) * 0.5), (T, w, z1 - z0), cc)

    def _place(a, w, z0, z1, cc, axis, off_sign, at):
        if axis == "x":
            box(p, (a, off_sign * at, (z0 + z1) * 0.5), (w, 0.04, z1 - z0), cc)
        else:
            box(p, (off_sign * at, a, (z0 + z1) * 0.5), (0.04, w, z1 - z0), cc)

    def _lip(a, w, z, th, axis, off_sign, at):
        if axis == "x":
            box(p, (a, off_sign * at, z + th * 0.5), (w, 0.18, th), C["stone"])
        else:
            box(p, (off_sign * at, a, z + th * 0.5), (0.18, w, th), C["stone"])

    facade(1, "x", BAYS, True)         # +Y front
    facade(-1, "x", BAYS, False)       # -Y rear: no shopfront
    facade(1, "y", 3, False)
    facade(-1, "y", 3, False)

    # Street-level read: a shopfront row needs doors, an awning and a kerb step, or the
    # ground floor is just a band of black holes and the building looks abandoned.
    fy = D * 0.5
    bay = W / BAYS
    for i in range(BAYS):
        cx = -W * 0.5 + bay * (i + 0.5)
        if i % 2 == 1:
            # recessed entrance with a reveal frame
            box(p, (cx, fy - 0.22, 1.15), (1.05, 0.06, 2.30), C["wood_dark"])
            box(p, (cx - 0.60, fy - 0.06, 1.18), (0.16, 0.16, 2.46), C["stone"])
            box(p, (cx + 0.60, fy - 0.06, 1.18), (0.16, 0.16, 2.46), C["stone"])
            box(p, (cx, fy - 0.06, 2.42), (1.36, 0.16, 0.16), C["stone"])
            box(p, (cx, fy + 0.42, 0.06), (1.5, 0.60, 0.12), C["concrete"])   # step
        else:
            # stall riser under the display glass so the window is not floor-to-ceiling
            box(p, (cx, fy - 0.10, 0.42), (2.30, 0.10, 0.84), C["concrete_dk"])
    # canvas awning across the whole front, sloped out and down
    prism(p, (0, fy + 0.55, 2.86), (W - 1.0, 1.15, 0.16), C["metal_rust"], "y")
    box(p, (0, fy + 1.10, 2.62), (W - 1.0, 0.06, 0.34), C["metal_rust"])       # valance
    for i in range(BAYS + 1):
        x = -W * 0.5 + bay * i
        box(p, (x, fy + 1.02, 2.86), (0.07, 1.05, 0.07), C["metal"])           # stays
    # kerb running the full frontage ties the building down to the street
    box(p, (0, fy + 1.45, 0.07), (W + 2.0, 0.45, 0.14), C["concrete"])

    # corner quoins + a plinth band: silhouette detail that survives distance
    for sx in (-1, 1):
        for sy in (-1, 1):
            box(p, (sx * (W * 0.5 - 0.15), sy * (D * 0.5 - 0.15), H * 0.5),
                (0.42, 0.42, H), C["stone"])
    box(p, (0, 0, 0.22), (W + 0.10, D + 0.10, 0.44), C["concrete_dk"])

    # interior shell so the openings never show sky through the far wall
    box(p, (0, 0, H * 0.5 + 0.05), (W - 2 * T - 0.1, D - 2 * T - 0.1, H), C["interior"])
    # floor slabs (visible through the windows, gives the openings depth)
    for fl in range(FLOORS + 1):
        box(p, (0, 0, fl * FLOOR_H + 0.06), (W - 0.6, D - 0.6, 0.14), C["concrete"])

    # roof: hipped tile mass + eave overhang, so the top is not a flat lid
    roof_z = H + 0.30
    box(p, (0, 0, H + 0.10), (W + 0.5, D + 0.5, 0.22), C["concrete"])   # cornice
    _hip_roof(p, W + 0.4, D + 0.4, roof_z, 1.9, C[roof])
    # chimney
    box(p, (-W * 0.28, D * 0.22, roof_z + 1.9), (0.8, 0.8, 2.4), C["brick"])
    box(p, (-W * 0.28, D * 0.22, roof_z + 3.15), (1.0, 1.0, 0.18), C["brick_dark"])
    return p, (W + 0.5, D + 0.5, roof_z + 3.4)


def _hip_roof(p, w, d, z0, rise, col):
    hx, hy = w * 0.5, d * 0.5
    rx, ry = w * 0.18, d * 0.18
    a = [(-hx, -hy, z0), (hx, -hy, z0), (hx, hy, z0), (-hx, hy, z0)]
    b = [(-rx, -ry, z0 + rise), (rx, -ry, z0 + rise),
         (rx, ry, z0 + rise), (-rx, ry, z0 + rise)]
    for i in range(4):
        j = (i + 1) % 4
        p.add_quad(a[i], a[j], b[j], b[i], col)
    p.add_quad(b[3], b[2], b[1], b[0], col)          # ridge cap
    p.add_quad(a[3], a[2], a[1], a[0], col)          # underside


def asset_wall_low():
    """Brick garden wall section with coping stones and a broken end."""
    rng = random.Random(23)
    p = Part("wall_brick")
    L, Hh, T = 4.0, 1.30, 0.30
    box(p, (0, 0, Hh * 0.5), (L, T, Hh), C["brick"])
    box(p, (0, 0, Hh + 0.06), (L + 0.12, T + 0.10, 0.12), C["concrete"])   # coping
    # broken step at one end so a straight run of these does not look machined
    box(p, (L * 0.5 - 0.30, 0, Hh + 0.30), (0.60, T, 0.42), C["brick"])
    for i in range(5):
        x = -L * 0.5 + 0.45 + i * 0.75 + rng.uniform(-0.06, 0.06)
        box(p, (x, 0, 0.10), (0.5, T + 0.06, 0.20), C["concrete_dk"])      # plinth
    return p, (L + 0.12, T + 0.10, Hh + 0.55)


def asset_sandbags():
    """Three-course sandbag wall — the standard cover prop.

    Courses alternate a half-bag offset (real stacking) and each bag is an elongated,
    subdivided blob: a 20-face icosahedron flattened to a disc reads as a pile of
    pyramids rather than as filled sacks.
    """
    rng = random.Random(31)
    p = Part("sandbag_wall")
    rows = 3
    bag = (1.45, 0.62, 0.55)          # long, narrow, squat
    step = 0.52
    for r in range(rows):
        z = 0.15 + r * 0.245
        n = 6 if r % 2 == 0 else 5
        x0 = -1.30 if r % 2 == 0 else -1.30 + step * 0.5
        for c in range(n):
            x = x0 + c * step + rng.uniform(-0.025, 0.025)
            y = rng.uniform(-0.04, 0.04) + (0.06 if r == 1 else 0.0)
            col = C["sandbag"] if (r + c) % 3 else C["sandbag_old"]
            icosphere(p, (x, y, z), 0.24, col, subdiv=1, flatten=1.0,
                      jitter=0.015, rng=rng, stretch=bag)
    return p, (3.3, 0.75, 0.72)


def asset_rubble():
    """Collapsed masonry: irregular chunks settling into a mound, plus splintered timber.

    Even-sized bright cubes read as a pile of sugar; real debris is darker than the wall
    it came from (dust and moisture), varies 3:1 in size, and is denser at the base.
    """
    rng = random.Random(41)
    p = Part("rubble_pile")
    palette = [C["concrete_dk"], C["brick"], C["brick_dark"], C["stone"], C["dirt"]]
    for i in range(18):
        a = rng.uniform(0, 6.28)
        rad = rng.uniform(0.0, 1.55)
        x, y = math.cos(a) * rad, math.sin(a) * rad * 0.72
        # size falls off with distance from the centre so the pile has a mass, not a rim
        s = rng.uniform(0.16, 0.58) * (1.15 - rad / 2.6)
        col = palette[rng.randrange(len(palette))]
        if i % 3 == 0:
            # spalled brick-sized fragment
            icosphere(p, (x, y, s * 0.55), s * 0.9, col, subdiv=0, flatten=0.7,
                      jitter=0.10, rng=rng,
                      stretch=(rng.uniform(0.8, 1.5), rng.uniform(0.7, 1.2), 1.0))
        else:
            box(p, (x, y, s * 0.42), (s, s * rng.uniform(0.55, 1.25), s * 0.8),
                col, rot_z=rng.uniform(0, 3.14))
    for i in range(3):
        cylinder(p, (rng.uniform(-1.1, 1.1), rng.uniform(-0.8, 0.8), 0.05),
                 (rng.uniform(-1.1, 1.1), rng.uniform(-0.8, 0.8), 0.55),
                 0.10, 0.05, C["wood_dark"], segs=5)
    # a broken length of wall still standing, so it reads as "this was a building"
    box(p, (-1.35, 0.15, 0.42), (0.35, 1.5, 0.84), C["brick"], rot_z=0.12)
    box(p, (-1.35, 0.15, 0.87), (0.40, 1.55, 0.06), C["concrete_dk"], rot_z=0.12)
    return p, (3.6, 2.6, 1.0)


def asset_capture_base():
    """Capture-point plinth (defect D10).

    Why this shape: the old plinth was a 0.15 m disc at y=0.08, so its top face sat about
    10 cm above the ground instances. Seen from a 1.7 m eye it is nearly edge-on and
    projects to under a pixel — the point read as two bare poles. What survives distance
    is *height plus a silhouette*: a 0.4 m drum with a chamfer, four corner bollards, and
    a tall pole. Kept neutral-grey because ownership colour is applied by the flag mesh.
    """
    p = Part("capture_point")
    R, HH = 3.2, 0.40
    _octagon_prism(p, R, HH, C["concrete"])
    _octagon_prism(p, R * 0.78, HH + 0.06, C["concrete_dk"])       # inset ring
    _octagon_prism(p, R * 0.52, HH + 0.12, C["stone"])             # raised centre
    # chamfer skirt at the base so it does not look like a extruded cookie cutter
    for i in range(8):
        a = math.pi / 8 + i * math.pi / 4
        box(p, (math.cos(a) * (R - 0.18), math.sin(a) * (R - 0.18), 0.11),
            (0.9, 0.34, 0.22), C["concrete_dk"], rot_z=a + math.pi / 2)
    for sx in (-1, 1):
        for sz in (-1, 1):
            cylinder(p, (sx * 2.35, sz * 2.35, HH), (sx * 2.35, sz * 2.35, HH + 0.75),
                     0.11, 0.09, C["metal"], segs=6)
            box(p, (sx * 2.35, sz * 2.35, HH + 0.80), (0.20, 0.20, 0.12), C["metal_rust"])
    cylinder(p, (0, 0, HH), (0, 0, 6.4), 0.11, 0.075, C["wood_dark"], segs=8)
    box(p, (0, 0, 6.52), (0.26, 0.26, 0.24), C["metal_rust"])
    return p, (R * 2 + 0.4, R * 2 + 0.4, 6.8)


def _octagon_prism(p, r, h, col, z0=0.0):
    """Flat-topped octagonal drum (cheaper and harder-edged in silhouette than a disc)."""
    ring = [(math.cos(math.pi / 8 + i * math.pi / 4) * r,
             math.sin(math.pi / 8 + i * math.pi / 4) * r) for i in range(8)]
    bot = [(x, y, z0) for x, y in ring]
    top = [(x, y, z0 + h) for x, y in ring]
    p.add_quad(top[0], top[1], top[2], top[3], col)
    p.add_quad(top[0], top[3], top[4], top[5], col)
    p.add_quad(bot[0], bot[7], bot[6], bot[5], col)
    p.add_quad(bot[0], bot[5], bot[4], bot[3], col)
    p.add_quad(bot[0], bot[3], bot[2], bot[1], col)
    for i in range(8):
        j = (i + 1) % 8
        p.add_quad(bot[i], bot[j], top[j], top[i], col)


def asset_capture_flag():
    """Flag exported separately from the plinth so the engine can tint or swap it per
    owning side without rebuilding the static base mesh."""
    rng = random.Random(5)
    p = Part("capture_flag")
    z0, z1 = 5.05, 6.35
    w = 1.55
    # a hanging cloth: three vertical panels with a slight billow and a notched fly end
    prev = (-w * 0.5, 0.0)
    for i in range(4):
        x0 = -w * 0.5 + i * (w / 4)
        x1 = x0 + w / 4
        b0 = 0.10 * math.sin(i * 1.3) + rng.uniform(-0.03, 0.03)
        b1 = 0.10 * math.sin((i + 1) * 1.3) + rng.uniform(-0.03, 0.03)
        p.add_quad((x0, b0, z1), (x1, b1, z1), (x1, b1, z0), (x0, b0, z0),
                   (0.86, 0.85, 0.82))
    # hoist sleeve wrapping the pole
    cylinder(p, (0, 0, z0 - 0.05), (0, 0, z1 + 0.05), 0.13, 0.13, C["wood_dark"], segs=6)
    return p, (w + 0.3, 0.4, z1 - z0 + 0.2)


def asset_fence_wire():
    """4 m section of barbed wire fence on wooden posts — the cheap area-denial silhouette."""
    rng = random.Random(13)
    p = Part("fence_wire")
    L, PH = 4.0, 1.75
    for x in (-L * 0.5, 0.0, L * 0.5):
        cylinder(p, (x, 0, 0.0), (x + rng.uniform(-0.04, 0.04), 0.04, PH),
                 0.075, 0.055, C["wood_dark"], segs=6)
    for z in (1.62, 1.30, 0.95, 0.60):
        sag = -0.09 if z < 1.6 else 0.0
        for i in range(8):
            x0 = -L * 0.5 + i * (L / 8)
            x1 = x0 + L / 8
            t0 = i / 8.0
            t1 = (i + 1) / 8.0
            s0 = sag * 4 * t0 * (1 - t0)
            s1 = sag * 4 * t1 * (1 - t1)
            cylinder(p, (x0, 0, z + s0), (x1, 0, z + s1), 0.016, 0.016,
                     C["metal"], segs=4, caps=False)
    # barbs
    for i in range(10):
        x = -L * 0.5 + 0.2 + i * 0.38
        z = 1.62 + rng.uniform(-0.02, 0.02)
        box(p, (x, 0, z), (0.09, 0.09, 0.02), C["metal"], rot_z=0.7)
        box(p, (x, 0, z), (0.02, 0.09, 0.09), C["metal"], rot_z=0.7)
    # angled struts
    for sx in (-1, 1):
        cylinder(p, (sx * L * 0.5, 0, 0.15), (sx * (L * 0.5 - 0.75), 0, 1.45),
                 0.055, 0.045, C["wood_dark"], segs=5)
    return p, (L, 0.5, PH + 0.1)


def asset_street_lamp():
    """Pole lamp — vertical rhythm along a street, and it gives the eye a known 5 m scale."""
    p = Part("street_lamp")
    H = 4.6
    cylinder(p, (0, 0, 0.0), (0, 0, H), 0.13, 0.075, C["metal"], segs=8)
    cylinder(p, (0, 0, 0.0), (0, 0, 0.35), 0.22, 0.15, C["metal"], segs=8)
    # curved arm approximated by three short tapered segments
    pts = [(0.0, 0.0, H), (0.30, 0.0, H + 0.26), (0.72, 0.0, H + 0.33)]
    for i in range(len(pts) - 1):
        cylinder(p, pts[i], pts[i + 1], 0.062, 0.055, C["metal"], segs=6, caps=False)
    # lantern housing
    box(p, (0.80, 0, H + 0.24), (0.44, 0.26, 0.10), C["metal"])
    box(p, (0.80, 0, H + 0.10), (0.30, 0.18, 0.20), (0.72, 0.66, 0.45))
    box(p, (0.80, 0, H - 0.02), (0.20, 0.13, 0.06), C["metal_rust"])
    return p, (1.1, 0.3, H + 0.4)


def extrude_x(part, profile, x0, x1, col):
    """Sweep a closed (y, z) polyline along X.

    Jersey barriers, kerbs, rails and any other constant-section run are all one of
    these, so they cost one helper instead of a hand-placed pile of boxes.
    """
    n = len(profile)
    for i in range(n):
        j = (i + 1) % n
        y0, z0 = profile[i]
        y1, z1 = profile[j]
        part.add_quad((x0, y0, z0), (x1, y0, z0), (x1, y1, z1), (x0, y1, z1), col)
    for i in range(1, n - 1):
        a, b, c = profile[0], profile[i], profile[i + 1]
        part.add_tri((x1, a[0], a[1]), (x1, c[0], c[1]), (x1, b[0], b[1]), col)
        part.add_tri((x0, a[0], a[1]), (x0, b[0], b[1]), (x0, c[0], c[1]), col)


def extrude_y(part, profile, y0, y1, col):
    """Sweep a closed (x, z) polyline along Y.

    A vehicle's side elevation swept across its width is the one construction that makes
    a car read as a car; stacking boxes to imitate it fails as soon as the proportions are
    off by a little, which is exactly what the first attempt did.
    """
    n = len(profile)
    for i in range(n):
        j = (i + 1) % n
        x0, z0 = profile[i]
        x1, z1 = profile[j]
        part.add_quad((x0, y0, z0), (x0, y1, z0), (x1, y1, z1), (x1, y0, z1), col)
    for i in range(1, n - 1):
        a, b, c = profile[0], profile[i], profile[i + 1]
        part.add_tri((a[0], y1, a[1]), (b[0], y1, b[1]), (c[0], y1, c[1]), col)
        part.add_tri((a[0], y0, a[1]), (c[0], y0, c[1]), (b[0], y0, b[1]), col)


def asset_container(color="oxide"):
    """20 ft ISO shipping container — the single most useful cover prop in a modern fight.

    Corrugation is modelled rather than faked with shading: at 100 m a flat-sided box
    reads as a building, and the ridge rhythm is what tells the eye "container".
    """
    L, W, H = 6.058, 2.438, 2.591
    col = C[color]
    p = Part("container_20ft")
    box(p, (0, 0, H * 0.5), (L, W, H), col)
    # corrugation ribs along both long faces
    for i in range(19):
        x = -L * 0.5 + 0.28 + i * 0.30
        box(p, (x, W * 0.5, H * 0.5), (0.09, 0.05, H - 0.24), col)
        box(p, (x, -W * 0.5, H * 0.5), (0.09, 0.05, H - 0.24), col)
    # roof ribs
    for i in range(9):
        box(p, (-L * 0.5 + 0.4 + i * 0.66, 0, H + 0.02), (0.07, W - 0.1, 0.045), col)
    # corner castings (the fitting points are what make it unmistakably an ISO box)
    for sx in (-1, 1):
        for sy in (-1, 1):
            for sz in (0.0, 1.0):
                box(p, (sx * (L * 0.5 - 0.09), sy * (W * 0.5 - 0.09),
                        0.09 + sz * (H - 0.18)), (0.18, 0.18, 0.18), C["metal"])
    # door end: two leaves, locking rods, cam keepers
    dx = L * 0.5
    box(p, (dx, 0, H * 0.5), (0.05, W - 0.05, H - 0.05), C[("metal")])
    for sy in (-1, 1):
        for i in range(2):
            x = dx + 0.05
            z0, z1 = 0.18, H - 0.18
            cylinder(p, (x, sy * (W * 0.25 + i * W * 0.25), z0),
                     (x, sy * (W * 0.25 + i * W * 0.25), z1), 0.028, 0.028,
                     C["metal_rust"], segs=5, caps=False)
            box(p, (x, sy * (W * 0.25 + i * W * 0.25), z1 + 0.05),
                (0.08, 0.10, 0.14), C["metal_rust"])
    # base rails
    for sy in (-1, 1):
        box(p, (0, sy * (W * 0.5 - 0.09), 0.055), (L - 0.2, 0.18, 0.11), C["metal"])
    return p, (L + 0.15, W + 0.1, H + 0.1)


def asset_barrier_jersey():
    """F-shape concrete safety barrier — the default modern roadblock element."""
    prof = [(-0.30, 0.0), (0.30, 0.0), (0.30, 0.10), (0.20, 0.26),
            (0.10, 0.54), (0.075, 0.81), (-0.075, 0.81), (-0.10, 0.54),
            (-0.20, 0.26), (-0.30, 0.10)]
    p = Part("barrier_jersey")
    extrude_x(p, prof, -1.5, 1.5, C["concrete"])
    # top cap slightly lighter (sun-bleached) + a pour joint at each end
    box(p, (0, 0, 0.825), (3.0, 0.15, 0.03), C["concrete_dk"])
    for sx in (-1, 1):
        box(p, (sx * 1.5, 0, 0.42), (0.02, 0.5, 0.8), C["concrete_dk"])
    # reflective marker so it reads at night and at range
    box(p, (0.9, 0.09, 0.70), (0.16, 0.02, 0.10), (0.75, 0.72, 0.60))
    return p, (3.0, 0.6, 0.86)


def asset_barrier_hesco():
    """HESCO bastion: welded wire basket, geotextile liner, earth fill cresting over.

    This is what a modern fighting position actually looks like, and it is the answer to
    "sandbags everywhere" — one of these replaces thirty bags on the silhouette.
    """
    rng = random.Random(53)
    p = Part("barrier_hesco")
    L, W, H = 2.0, 1.1, 1.05
    box(p, (0, 0, H * 0.46), (L, W, H * 0.92), C["metal_rust"])
    # liner board on the long faces (darker, slightly proud of the mesh)
    for sy in (-1, 1):
        box(p, (0, sy * (W * 0.5 + 0.015), H * 0.42), (L - 0.12, 0.03, H * 0.8),
            C["dirt"])
    # visible wire grid on all four faces (the basket is what identifies it)
    for i in range(5):
        y = -W * 0.5 + 0.12 + i * (W - 0.24) / 4
        for sx in (-1, 1):
            box(p, (sx * (L * 0.5 + 0.015), y, H * 0.45), (0.03, 0.05, H * 0.86),
                C["metal"])
    for i in range(4):
        z = 0.15 + i * 0.26
        for sx in (-1, 1):
            box(p, (sx * (L * 0.5 + 0.015), 0, z), (0.03, W - 0.1, 0.05), C["metal"])
    for i in range(9):
        x = -L * 0.5 + 0.10 + i * (L - 0.20) / 8
        for sy in (-1, 1):
            box(p, (x, sy * (W * 0.5 + 0.012), H * 0.45), (0.04, 0.03, H * 0.86),
                C["metal"])
    for i in range(4):
        z = 0.15 + i * 0.26
        for sy in (-1, 1):
            box(p, (0, sy * (W * 0.5 + 0.012), z), (L - 0.1, 0.03, 0.04), C["metal"])
    # earth fill: an irregular crest, not a row of flat discs — the first attempt read as
    # pancakes on a crate because every blob had the same radius and the same squash
    fill = [(-0.72, 0.16, 0.32, 0.40), (-0.30, -0.20, 0.46, 0.48),
            (0.14, 0.14, 0.52, 0.52), (0.58, -0.14, 0.38, 0.44),
            (0.92, 0.10, 0.28, 0.32), (-0.98, -0.10, 0.26, 0.28)]
    # a continuous crest ties the lumps together, otherwise the fill reads as boulders
    # dumped in a basket rather as soil heaped along the top of one
    box(p, (0, 0, H + 0.06), (L - 0.10, W - 0.16, 0.16), (0.21, 0.17, 0.13))
    for i, (fx, fy, fr, fh) in enumerate(fill):
        icosphere(p, (fx, fy, H - 0.04 + fh * 0.40), fr,
                  (0.23, 0.185, 0.135) if i % 2 else (0.185, 0.15, 0.115), subdiv=1,
                  flatten=fh / fr, jitter=0.09, rng=rng,
                  stretch=(rng.uniform(1.1, 1.5), rng.uniform(0.7, 0.9), 1.0))
    # a few stones and a spilled sandbag at the toe so it does not sit like a die-cut
    for i in range(4):
        icosphere(p, (rng.uniform(-1.2, 1.2), rng.uniform(-0.9, 0.9), 0.09), 0.14,
                  C["concrete_dk"], subdiv=0, flatten=0.7, jitter=0.05, rng=rng)
    return p, (L + 0.3, W + 0.4, H + 0.6)


def asset_fence_chainlink():
    """3 m chain-link panel on steel line posts, with a barbed ribbon head."""
    p = Part("fence_chainlink")
    L, H = 3.0, 2.1
    for x in (-L * 0.5, L * 0.5):
        cylinder(p, (x, 0, 0.0), (x, 0, H + 0.12), 0.045, 0.04, C["metal"], segs=6)
        box(p, (x, 0, H + 0.16), (0.09, 0.09, 0.10), C["metal"])
    cylinder(p, (-L * 0.5, 0, H), (L * 0.5, 0, H), 0.032, 0.032, C["metal"],
             segs=6, caps=False)
    cylinder(p, (-L * 0.5, 0, 0.10), (L * 0.5, 0, 0.10), 0.028, 0.028, C["metal"],
             segs=6, caps=False)
    # mesh: vertical strands are what read as chain-link at distance; the diagonals only
    # matter up close, so they are thinned out rather than modelled as a real weave
    n = 20
    for i in range(n):
        x = -L * 0.5 + 0.06 + i * (L - 0.12) / (n - 1)
        box(p, (x, 0, (H + 0.10) * 0.5), (0.018, 0.018, H - 0.08), C["metal"])
    for i in range(7):
        x = -L * 0.5 + 0.1 + i * (L - 0.2) / 6
        box(p, (x, 0, H * 0.55), (0.014, 0.014, H * 0.9), C["metal"], rot_z=0.62)
        box(p, (x, 0, H * 0.55), (0.014, 0.014, H * 0.9), C["metal"], rot_z=-0.62)
    # barbed ribbon angled outboard on brackets
    for i in range(6):
        x = -L * 0.5 + 0.25 + i * 0.5
        box(p, (x, 0.16, H + 0.30), (0.02, 0.34, 0.02), C["metal_rust"], rot_z=0.0)
        cylinder(p, (x - 0.2, 0.05, H + 0.16), (x + 0.2, 0.28, H + 0.44),
                 0.014, 0.014, C["metal"], segs=4, caps=False)
    return p, (L, 0.5, H + 0.5)


def asset_utility_pole():
    """Concrete power pole with cross-arm, insulators and a pole transformer."""
    p = Part("utility_pole")
    H = 8.0
    extrude_x(p, [(-0.13, 0.0), (0.13, 0.0), (0.09, H), (-0.09, H)], -0.001, 0.001,
              C["concrete"])
    for i, (r0, r1, z0, z1) in enumerate(((0.15, 0.115, 0.0, 2.6),
                                          (0.115, 0.095, 2.6, 5.4),
                                          (0.095, 0.075, 5.4, H))):
        cylinder(p, (0, 0, z0), (0, 0, z1), r0, r1, C["concrete"], segs=8, caps=False)
    box(p, (0, 0, H - 0.55), (3.4, 0.16, 0.18), C["concrete"])          # cross-arm
    box(p, (0, 0, H - 1.35), (2.2, 0.14, 0.16), C["concrete"])          # lower arm
    for x in (-1.5, -0.75, 0.75, 1.5):
        cylinder(p, (x, 0, H - 0.46), (x, 0, H - 0.20), 0.055, 0.075,
                 C["stone"], segs=6)
    # transformer can with a hood
    cylinder(p, (0.42, 0.16, 4.55), (0.42, 0.16, 5.45), 0.26, 0.26, C["metal"], segs=10)
    cylinder(p, (0.42, 0.16, 5.46), (0.42, 0.16, 5.56), 0.30, 0.24, C["metal"], segs=10)
    box(p, (0.20, 0.0, 5.0), (0.24, 0.16, 0.10), C["metal_rust"])       # bracket
    # service drop stubs
    for x in (-1.5, 1.5):
        cylinder(p, (x, 0, H - 0.22), (x * 0.35, 0, H - 1.0), 0.014, 0.014,
                 C["metal"], segs=4, caps=False)
    return p, (3.4, 0.6, H + 0.2)


def asset_car_wreck():
    """Burned-out sedan swept from a real side elevation.

    Nothing says a 21st-century battlefield faster than wrecked civil traffic, and it is
    hard cover that is not a sandbag. The silhouette carries it: nose, hood, raked
    windshield, roof, boot. Tyres are burnt off so only the hubs and arch lips remain.
    """
    p = Part("car_wreck")
    body = (0.085, 0.080, 0.078)
    soot = (0.035, 0.033, 0.032)
    glass = (0.03, 0.03, 0.035)
    HW = 0.86                                   # half width
    # side elevation, x+ is the front, listed CCW as seen from +Y
    prof = [(-2.30, 0.34), (-2.30, 0.80), (-1.62, 0.98), (-1.02, 1.04),
            (-0.62, 1.50), (0.52, 1.52), (1.10, 1.06), (1.72, 0.95),
            (2.20, 0.82), (2.30, 0.52), (2.30, 0.34), (1.62, 0.30),
            (1.30, 0.30), (0.90, 0.34), (-0.90, 0.34), (-1.20, 0.30),
            (-1.62, 0.30), (-2.05, 0.34)]
    extrude_y(p, prof, -HW, HW, body)
    # greenhouse: darker glass band inset on both flanks, following the pillar line
    for sy in (-1, 1):
        y = sy * (HW + 0.012)
        gp = [(-1.00, 1.06), (-0.66, 1.44), (0.50, 1.46), (1.02, 1.08)]
        for i in range(len(gp) - 1):
            x0, z0 = gp[i]
            x1, z1 = gp[i + 1]
            p.add_quad((x0, y, z0), (x1, y, z1), (x1, y, z1 - 0.30),
                       (x0, y, z0 - 0.30), glass)
        # B pillar splitting the two side windows
        box(p, (0.02, y, 1.24), (0.10, 0.03, 0.42), body)
    # wheel arch lips + bare hubs
    for fx in (1.46, -1.46):
        for sy in (-1, 1):
            box(p, (fx, sy * HW, 0.44), (0.86, 0.06, 0.10), soot)
            cylinder(p, (fx, sy * (HW - 0.02), 0.36), (fx, sy * (HW + 0.05), 0.36),
                     0.24, 0.24, soot, segs=9)
            cylinder(p, (fx, sy * (HW + 0.05), 0.36), (fx, sy * (HW + 0.10), 0.36),
                     0.11, 0.11, C["metal_rust"], segs=8)
    # a door left hanging open — reads instantly as "abandoned in a hurry"
    box(p, (-0.45, HW + 0.42, 0.72), (1.15, 0.055, 0.66), body, rot_z=0.62)
    box(p, (-0.45, HW + 0.80, 1.06), (1.05, 0.05, 0.30), glass, rot_z=0.62)
    # nose and tail detail so it is not a monolith
    box(p, (2.28, 0, 0.62), (0.06, HW * 2 - 0.1, 0.20), soot)             # grille
    box(p, (-2.29, 0, 0.66), (0.05, HW * 2 - 0.1, 0.16), (0.28, 0.06, 0.05))  # tail lights
    for sy in (-1, 1):
        box(p, (2.24, sy * 0.58, 0.74), (0.14, 0.22, 0.10), soot)         # headlight shells
    # underbody shadow mass + soot up the flanks
    box(p, (0, 0, 0.24), (4.1, HW * 2 - 0.2, 0.12), soot)
    for sy in (-1, 1):
        box(p, (0, sy * (HW + 0.008), 0.46), (4.2, 0.02, 0.30), soot)
    return p, (4.7, HW * 2 + 1.0, 1.6)


def asset_panel_block():
    """Five-storey precast panel block — the mass that makes a modern city *look* modern.

    Old-town blocks alone read as a heritage centre; the plate-concrete apartment slab with
    its regular window grid and stacked balconies is what actually fills a 21st-century
    European skyline. Flat roof + parapet, no chimneys.
    """
    p = Part("panel_block")
    W, D, FL = 18.0, 10.0, 5
    FH = 2.9
    H = FL * FH
    T = 0.28
    col = (0.52, 0.49, 0.44)

    def wall_run(axis, off_sign, bays, with_balcony):
        run, dist = (W, D * 0.5) if axis == "x" else (D, W * 0.5)
        half = run * 0.5
        bay = run / bays
        win_w = bay * 0.52
        for i in range(bays):
            cx = -half + bay * (i + 0.5)
            for fl in range(FL):
                z0 = fl * FH
                # full-width spandrel panel between floors (the precast joint line)
                if axis == "x":
                    box(p, (cx, off_sign * (dist - T * 0.5), z0 + 0.05),
                        (bay - 0.02, T, 0.55), col)
                    box(p, (cx, off_sign * (dist - T * 0.5), z0 + FH - 0.50),
                        (bay - 0.02, T, 0.50), col)
                    box(p, (cx, off_sign * (dist - 0.10), z0 + 1.05),
                        (win_w, 0.05, FH - 1.55), (0.07, 0.09, 0.11))
                else:
                    box(p, (off_sign * (dist - T * 0.5), cx, z0 + 0.05),
                        (T, bay - 0.02, 0.55), col)
                    box(p, (off_sign * (dist - T * 0.5), cx, z0 + FH - 0.50),
                        (T, bay - 0.02, 0.50), col)
                    box(p, (off_sign * (dist - 0.10), cx, z0 + 1.05),
                        (0.05, win_w, FH - 1.55), (0.07, 0.09, 0.11))
                if with_balcony and fl >= 1 and i % 3 == 1:
                    bb = 1.25
                    if axis == "x":
                        y = off_sign * (dist + bb * 0.5)
                        box(p, (cx, y, z0 + 0.06), (bay * 0.86, bb, 0.12), C["concrete"])
                        box(p, (cx, off_sign * (dist + bb - 0.05), z0 + 0.52),
                            (bay * 0.86, 0.08, 0.80), C["concrete_dk"])
                        for s in (-1, 1):
                            box(p, (cx + s * bay * 0.42, y, z0 + 0.50),
                                (0.08, bb, 0.76), C["concrete_dk"])
                    else:
                        x = off_sign * (dist + bb * 0.5)
                        box(p, (x, cx, z0 + 0.06), (bb, bay * 0.86, 0.12), C["concrete"])
                        box(p, (off_sign * (dist + bb - 0.05), cx, z0 + 0.52),
                            (0.08, bay * 0.86, 0.80), C["concrete_dk"])
                        for s in (-1, 1):
                            box(p, (x, cx + s * bay * 0.42, z0 + 0.50),
                                (bb, 0.08, 0.76), C["concrete_dk"])
        # vertical joint strips emphasise the panel grid
        for i in range(bays + 1):
            e = -half + bay * i
            if axis == "x":
                box(p, (e, off_sign * (dist - T * 0.5 + 0.01), H * 0.5),
                    (0.10, T + 0.02, H), C["concrete_dk"])
            else:
                box(p, (off_sign * (dist - T * 0.5 + 0.01), e, H * 0.5),
                    (T + 0.02, 0.10, H), C["concrete_dk"])

    wall_run("x", 1, 6, True)
    wall_run("x", -1, 6, True)
    wall_run("y", 1, 3, False)
    wall_run("y", -1, 3, False)
    # end stair core, blank except a small window band
    box(p, (W * 0.5 + 0.9, 0, H * 0.5), (2.6, D - 1.0, H), C["concrete"])
    box(p, (W * 0.5 + 0.9, 0, H + 0.16), (2.9, D - 0.7, 0.32), C["concrete_dk"])
    # interior shell so openings never show sky
    box(p, (0, 0, H * 0.5), (W - 2 * T, D - 2 * T, H), C["interior"])
    # flat roof + parapet + rooftop plant
    box(p, (0, 0, H + 0.08), (W + 0.2, D + 0.2, 0.18), C["concrete"])
    for sx in (-1, 1):
        box(p, (0, sx * (D * 0.5 + 0.06), H + 0.55), (W + 0.3, 0.12, 0.76), col)
    for sy in (-1, 1):
        box(p, (sy * (W * 0.5 + 0.06), 0, H + 0.55), (0.12, D + 0.3, 0.76), col)
    box(p, (-W * 0.25, 0, H + 0.90), (2.2, 1.6, 1.0), C["concrete_dk"])  # lift house
    for i in range(3):
        box(p, (W * 0.15 + i * 0.7, -1.2, H + 0.45), (0.5, 0.5, 0.42), C["metal"])
    cylinder(p, (W * 0.32, 1.6, H + 0.30), (W * 0.32, 1.6, H + 2.6), 0.05, 0.03,
             C["metal"], segs=5)
    return p, (W + 3.5, D + 0.6, H + 3.0)


ASSETS = {
    "crate_wood": asset_crate,
    "barrel_metal": asset_barrel,
    "tree_oak": asset_tree,
    "wall_brick": asset_wall_low,
    "sandbag_wall": asset_sandbags,
    "rubble_pile": asset_rubble,
    "capture_point": asset_capture_base,
    "capture_flag": asset_capture_flag,
    "fence_wire": asset_fence_wire,
    "street_lamp": asset_street_lamp,
    # 21st-century theatre: the props a modern large battlefield is actually made of.
    "container_20ft": asset_container,
    "container_navy": lambda: asset_container("container_navy"),
    "container_green": lambda: asset_container("container_green"),
    "barrier_jersey": asset_barrier_jersey,
    "barrier_hesco": asset_barrier_hesco,
    "fence_chainlink": asset_fence_chainlink,
    "utility_pole": asset_utility_pole,
    "car_wreck": asset_car_wreck,
    "panel_block": asset_panel_block,
    # Street furniture variants: same generator, different footprint/palette so a row of
    # buildings does not read as one cloned object repeated.
    "building_block": lambda: asset_building(12.0, 9.0, 2, 4, "plaster", "roof_tile", 11),
    "building_tall": lambda: asset_building(10.0, 8.0, 3, 3, "brick", "roof_slate", 17),
    "building_wide": lambda: asset_building(16.0, 8.0, 2, 5, "plaster_warm",
                                            "roof_tile", 23),
    "building_corner": lambda: asset_building(9.0, 9.0, 2, 3, "stone",
                                              "roof_slate", 29),
    "building_shed": lambda: asset_building(11.0, 7.0, 1, 3, "concrete",
                                            "roof_slate", 37),
}


# ----------------------------------------------------------------------------- preview
def _look_at(cam_ob, target, dist, elev_deg, yaw_deg):
    e = math.radians(elev_deg)
    y = math.radians(yaw_deg)
    off = Vector((math.cos(e) * math.sin(y), math.cos(e) * math.cos(y), math.sin(e)))
    tgt = Vector(target)
    cam_ob.location = tgt + off * dist
    cam_ob.rotation_mode = "QUATERNION"
    cam_ob.rotation_quaternion = (tgt - cam_ob.location).to_track_quat("-Z", "Y")


def render_preview(names, png_path):
    """Vertex-coloured workbench render of the whole kit, so modelling mistakes are
    caught here rather than after they are wired into the engine."""
    reset_scene()
    objs = []
    for n in names:
        fn = ASSETS.get(n)
        if fn is None:
            continue
        part, size = fn()
        ob = finish_object(part, n)
        objs.append((n, ob, size))

    # lay the kit out in a row along +X, spaced by each prop's own footprint
    x = 0.0
    small_x = []
    for n, ob, size in objs:
        w = max(size[0], 1.0) + 1.2
        ob.location.x = x + w * 0.5
        if size[0] < 4.0:
            small_x.append(ob.location.x)
        x += w
    span = x

    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    sh = scene.display.shading
    sh.light = "STUDIO"
    sh.color_type = "VERTEX"
    sh.show_cavity = True
    sh.show_shadows = True
    scene.render.film_transparent = False
    world = bpy.data.worlds.get("World") or bpy.data.worlds.new("World")
    scene.world = world
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.42, 0.48, 0.56, 1.0)

    scene.camera = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    scene.collection.objects.link(scene.camera)
    cam = scene.camera
    cam.data.lens = 35
    centre = (span * 0.5, 0, 2.4)
    dist = max(span * 1.30, 14.0)

    scene.render.resolution_x = 1920
    scene.render.resolution_y = 1080
    scene.render.image_settings.file_format = "PNG"

    # Two three-quarter views (opposite sides) plus an orthographic plan, so both the
    # front and the rear of every asset get eyeballed before they reach the engine.
    views = [("a", 25.0, 12.0), ("b", 205.0, 14.0)]
    for tag, yaw, elev in views:
        cam.data.type = "PERSP"
        _look_at(cam, centre, dist, elev, yaw)
        path = png_path.replace(".png", "_" + tag + ".png")
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        print("PREVIEW", path)

    cam.data.type = "ORTHO"
    cam.data.ortho_scale = max(span, 10.0) * 1.1
    cam.rotation_mode = "XYZ"
    cam.location = (span * 0.5, 0.0, 40.0)
    cam.rotation_euler = (0.0, 0.0, 0.0)
    top = png_path.replace(".png", "_top.png")
    scene.render.filepath = top
    bpy.ops.render.render(write_still=True)
    print("PREVIEW", top)

    # close plates for the props too small to judge in the wide shot
    cam.data.type = "PERSP"
    cam.data.lens = 60
    for i, px in enumerate(small_x):
        _look_at(cam, (px, 0, 0.55), 6.0, 16.0, 32.0)
        path = png_path.replace(".png", "_z%02d.png" % i)
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        print("PREVIEW", path)


# ----------------------------------------------------------------------------- CLI
def _args():
    out = "D:/Rust/steel-front/assets/props"
    only = None
    preview = None
    argv = sys.argv
    if "--" in argv:
        argv = argv[argv.index("--") + 1:]
    else:
        argv = []
    i = 0
    while i < len(argv):
        if argv[i] == "--out":
            out = argv[i + 1]; i += 2
        elif argv[i] == "--only":
            only = argv[i + 1].split(","); i += 2
        elif argv[i] == "--preview":
            preview = argv[i + 1]; i += 2
        else:
            i += 1
    return out, only, preview


def main():
    out, only, preview = _args()
    os.makedirs(out, exist_ok=True)
    names = only if only else list(ASSETS.keys())
    if preview:
        os.makedirs(os.path.dirname(preview), exist_ok=True)
        render_preview(names, preview)
        return
    reset_scene()
    report = []
    for n in names:
        fn = ASSETS.get(n)
        if fn is None:
            print("SKIP unknown asset", n)
            continue
        part, size = fn()
        ob = finish_object(part, n)
        path = os.path.join(out, n + ".glb")
        export_glb(ob, path)
        me = ob.data
        report.append((n, len(me.vertices), len(me.polygons),
                       tuple(round(s, 2) for s in size), os.path.getsize(path)))
        bpy.data.objects.remove(ob, do_unlink=True)
        bpy.data.meshes.remove(me)
    print("=== BUILT ===")
    for r in report:
        print("%-16s verts=%-6d tris=%-6d size=%-22s bytes=%d" %
              (r[0], r[1], r[2], "x".join(str(v) for v in r[3]), r[4]))


main()
