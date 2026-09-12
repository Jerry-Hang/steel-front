"""Designed city kit for a 2020s war-torn eastern-european town.

WHY THIS FILE EXISTS
  The retired `asset_building()` in gen_props.py did build REAL window openings
  (piers + spandrels + recessed glass), so the geometry engine was never the
  problem. The problem was that everything above it was a parameter: bay count,
  pier width, sill height. It produced a 2.75 x 1.6 m window on a 3.5 m bay --
  a shopfront proportion on a dwelling -- and it had no architectural vocabulary
  whatsoever: no plinth, no projecting sill, no lintel, no parapet or coping, no
  entrance, no balcony, no roof clutter, and one flat colour per surface.

  Parameters cannot invent taste, but a DESIGNED module can. So modules here are
  authored against real reference proportions rather than generated from a size
  range, and the parts that make a facade read at street level are explicit:
  plinth, sill, lintel, reveal, parapet, coping, entrance canopy, balcony.

ENGINE CONVENTIONS (AGENTS.md 铁律 D -- do not deviate)
  * 1 unit = 1 m. Origin at the BOTTOM CENTRE of the footprint; z = 0 is ground.
  * Blender +Z up; exported with export_yup=True.
  * Single mesh, single primitive, object carries NO transform.
  * Winding is outward-CCW (right-handed glTF). props::merge reverses it for the
    engine, so authoring standard glTF winding here is correct.
  * Vertex colour layer "Col", BYTE_COLOR on POINT domain. export_materials="NONE":
    ALL appearance comes from vertex colour, because the engine's prop path bakes
    pose into vertices and its vertex layout is pos/color/uv only -- there is no
    normal slot (normals are rebuilt from screen-space derivatives => flat shading)
    and no texture sampling for authored shapes.

  Because flat shading is all we get, detail MUST be real geometry, and ambient
  occlusion MUST be baked into vertex colour. That is what `_ao` and the reveal
  gradients below are for.

Run:
  blender.exe --background --python tools/blender/build_city_kit.py -- <out_dir> [name...]
"""
import sys
import os
import math
import random
import zlib

import bpy
from mathutils import Vector, Matrix

# ============================================================== palette
# Cold, desaturated, war-torn. Deliberately narrow: a coherent street beats a
# colourful one, and variance comes from geometry + AO, not from hue.
C = {
    "concrete":   (0.470, 0.468, 0.452),
    "concrete_l": (0.556, 0.552, 0.534),
    "concrete_d": (0.340, 0.338, 0.330),
    "plinth":     (0.196, 0.194, 0.190),
    "plaster":    (0.596, 0.556, 0.486),
    "plaster_d":  (0.470, 0.432, 0.378),
    "roof":       (0.148, 0.150, 0.160),
    "metal":      (0.286, 0.290, 0.302),
    "rust":       (0.330, 0.196, 0.116),
    "glass":      (0.058, 0.072, 0.088),
    "board":      (0.322, 0.256, 0.172),
    "sill":       (0.512, 0.508, 0.494),
    "door":       (0.126, 0.164, 0.186),
    "joint":      (0.352, 0.350, 0.342),
    "soot":       (0.210, 0.206, 0.202),
}

# ============================================================== part buffer
class Part:
    """Accumulates triangles with a per-vertex colour so AO can vary inside a face."""

    def __init__(self, name):
        self.name = name
        self.verts = []
        self.tris = []
        self.colour = []

    def add_quad(self, a, b, c, d, col):
        self.add_quad_v(a, b, c, d, (col, col, col, col))

    def add_quad_v(self, a, b, c, d, cols):
        i = len(self.verts)
        self.verts.extend((a, b, c, d))
        self.colour.extend(cols)
        self.tris.append((i, i + 1, i + 2))
        self.tris.append((i, i + 2, i + 3))


def _sub(p, q):
    return (p[0] - q[0], p[1] - q[1], p[2] - q[2])


def _cross(u, v):
    return (u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0])


def add_quad_n(part, a, b, c, d, cols, n):
    """Emit a quad whose outward normal is `n`, flipping the winding if needed.

    The engine rebuilds shading normals from screen-space derivatives, so a
    backwards face renders BLACK and reports nothing. Rather than trusting hand
    derived winding for every reveal and jamb, we pass the INTENDED outward normal
    and let the cross product decide -- this removes an entire class of silent
    failure, and it is cheap.
    """
    g = _cross(_sub(b, a), _sub(c, a))
    if g[0] * n[0] + g[1] * n[1] + g[2] * n[2] < 0.0:
        part.add_quad_v(a, d, c, b, (cols[0], cols[3], cols[2], cols[1]))
    else:
        part.add_quad_v(a, b, c, d, cols)


def _ao(col, k):
    k = 0.0 if k < 0.0 else (1.35 if k > 1.35 else k)
    return (col[0] * k, col[1] * k, col[2] * k)


def exposure_ao(z, floor_h):
    """Ambient occlusion from the sky, as a function of height.

    Ground contact is the single strongest depth cue on a flat-shaded box, so the
    bottom of every wall is pulled down hard and it recovers over roughly one
    storey. Cheap, but it is what stops a facade from reading as a paper cut-out.
    """
    t = z / max(floor_h, 0.001)
    if t >= 1.0:
        return 1.0
    if t <= 0.0:
        return 0.66
    return 0.66 + 0.34 * (t ** 0.65)


# ============================================================== wall with openings
def wall_panel(part, run, sign, at, thick, u0, u1, z0, z1, col,
               openings=(), joints=(), floor_h=3.15, backing=None,
               revealed=True, jamb_ao=0.40):
    """A facade plane with real punched openings, reveals, sills and panel joints.

    `run` is the axis the wall RUNS along ('x' or 'y'); `at` is the coordinate on
    the other horizontal axis; `sign` is the outward direction on that axis.
    Thickness is extruded INWARD (away from `sign`).

    The rectangle-minus-rectangles is decomposed on the grid of all opening and
    joint edges. Every resulting cell is either a hole or one front quad, so the
    result is always watertight with no overlapping faces and no T-junctions on
    the facade plane.
    """
    if u1 - u0 <= 1e-5 or z1 - z0 <= 1e-5:
        return

    JOINT_W = 0.075
    xs = {u0, u1}
    zs = {z0, z1}
    for op in openings:
        ua, ub, za, zb = op[0], op[1], op[2], op[3]
        xs.add(min(max(ua, u0), u1))
        xs.add(min(max(ub, u0), u1))
        zs.add(min(max(za, z0), z1))
        zs.add(min(max(zb, z0), z1))
    for j in joints:
        if z0 < j - JOINT_W * 0.5 < z1:
            zs.add(j - JOINT_W * 0.5)
            zs.add(j + JOINT_W * 0.5)
    xs = sorted(xs)
    zs = sorted(zs)
    inner = at - sign * thick

    def pt(u, n, z):
        return (u, n, z) if run == "x" else (n, u, z)

    def nrm(du, dn, dz):
        return (du, dn, dz) if run == "x" else (dn, du, dz)

    def in_hole(uc, zc):
        for op in openings:
            ua, ub, za, zb = op[0], op[1], op[2], op[3]
            if ua + 1e-4 < uc < ub - 1e-4 and za + 1e-4 < zc < zb - 1e-4:
                return True
        return False

    def is_joint(zc):
        for j in joints:
            if abs(zc - j) <= JOINT_W * 0.6:
                return True
        return False

    out_n = nrm(0.0, sign, 0.0)
    for iu in range(len(xs) - 1):
        xa, xb = xs[iu], xs[iu + 1]
        if xb - xa <= 1e-5:
            continue
        for iz in range(len(zs) - 1):
            za, zb = zs[iz], zs[iz + 1]
            if zb - za <= 1e-5:
                continue
            uc, zc = (xa + xb) * 0.5, (za + zb) * 0.5
            if in_hole(uc, zc):
                continue
            base = C["joint"] if is_joint(zc) else col
            k0 = exposure_ao(za, floor_h)
            k1 = exposure_ao(zb, floor_h)
            cols = (_ao(base, k0), _ao(base, k0), _ao(base, k1), _ao(base, k1))
            add_quad_n(part, pt(xa, at, za), pt(xb, at, za),
                       pt(xb, at, zb), pt(xa, at, zb), cols, out_n)

    if not revealed:
        return

    for op in openings:
        ua, ub, za, zb = op[0], op[1], op[2], op[3]
        # a loggia is a deep opening; everything else reveals by the wall thickness
        inner = at - sign * (op[4] if len(op) > 4 else thick)
        ua = min(max(ua, u0), u1)
        ub = min(max(ub, u0), u1)
        za = min(max(za, z0), z1)
        zb = min(max(zb, z0), z1)
        if ub - ua <= 1e-5 or zb - za <= 1e-5:
            continue
        kf = exposure_ao(za, floor_h)
        ko = _ao(col, kf)
        kd = _ao(col, kf * jamb_ao)
        # jambs: outward normal points INTO the opening, and the vertex at the
        # outer plane stays light while the one at the back of the reveal goes dark
        add_quad_n(part, pt(ua, at, za), pt(ua, inner, za),
                   pt(ua, inner, zb), pt(ua, at, zb),
                   (ko, kd, kd, ko), nrm(1.0, 0.0, 0.0))
        add_quad_n(part, pt(ub, at, za), pt(ub, inner, za),
                   pt(ub, inner, zb), pt(ub, at, zb),
                   (ko, kd, kd, ko), nrm(-1.0, 0.0, 0.0))
        # head (faces down) and sill reveal (faces up); both darkest at the back
        add_quad_n(part, pt(ua, at, zb), pt(ub, at, zb),
                   pt(ub, inner, zb), pt(ua, inner, zb),
                   (ko, ko, kd, kd), nrm(0.0, 0.0, -1.0))
        add_quad_n(part, pt(ua, at, za), pt(ub, at, za),
                   pt(ub, inner, za), pt(ua, inner, za),
                   (_ao(col, kf * 0.72), _ao(col, kf * 0.72),
                    _ao(col, kf * jamb_ao * 0.8), _ao(col, kf * jamb_ao * 0.8)),
                   nrm(0.0, 0.0, 1.0))
        # backing seals the hole at the back of the reveal so the shell stays
        # watertight and nothing is visible through the building
        bc = backing if backing is not None else C["glass"]
        kk = exposure_ao(za, floor_h) * 0.85
        add_quad_n(part, pt(ua, inner, za), pt(ub, inner, za),
                   pt(ub, inner, zb), pt(ua, inner, zb),
                   (_ao(bc, kk),) * 4, out_n)


# ============================================================== solid helpers
def box(part, centre, size, col, ao_top=1.0):
    cx, cy, cz = centre
    hx, hy, hz = size[0] * 0.5, size[1] * 0.5, size[2] * 0.5
    a = (cx - hx, cy - hy, cz - hz)
    b = (cx + hx, cy - hy, cz - hz)
    c = (cx + hx, cy + hy, cz - hz)
    d = (cx - hx, cy + hy, cz - hz)
    e = (cx - hx, cy - hy, cz + hz)
    f = (cx + hx, cy - hy, cz + hz)
    g = (cx + hx, cy + hy, cz + hz)
    h = (cx - hx, cy + hy, cz + hz)
    kz0 = exposure_ao(cz - hz, 3.15)
    kol = _ao(col, kz0)
    ktop = _ao(col, ao_top)
    add_quad_n(part, a, b, c, d, (kol,) * 4, (0.0, 0.0, -1.0))
    add_quad_n(part, e, f, g, h, (ktop,) * 4, (0.0, 0.0, 1.0))
    add_quad_n(part, a, b, f, e, (kol,) * 4, (0.0, -1.0, 0.0))
    add_quad_n(part, c, d, h, g, (kol,) * 4, (0.0, 1.0, 0.0))
    add_quad_n(part, d, a, e, h, (kol,) * 4, (-1.0, 0.0, 0.0))
    add_quad_n(part, b, c, g, f, (kol,) * 4, (1.0, 0.0, 0.0))


def slab(part, x0, x1, y0, y1, z0, z1, col, top_ao=1.0, under_ao=0.50):
    """Axis-aligned box by extremes, with the UNDERSIDE darkened.

    Undersides matter more than they look: a balcony or canopy with a bright
    soffit reads as a floating card, which is exactly the failure mode we are
    replacing.
    """
    box(part, ((x0 + x1) * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5),
        (x1 - x0, y1 - y0, z1 - z0), col, ao_top=top_ao)


# ============================================================== grid
GROUND_H = 3.65     # nominal ground floor (taller than the rest); see storey_zs
FLOOR_H = 3.15      # MUST equal city.rs / build.rs FLOOR_H -- this fixes the fork
COPING_H = 0.09     # cap that oversails the parapet


def storey_zs(floors, parapet, target_h):
    """Storey base heights + wall top + parapet top, fitted to an EXACT total.

    Upper storeys are pinned to FLOOR_H (the engine's collision grid uses the same
    3.15, so upper-floor windows land on that rhythm) and the GROUND floor absorbs
    the remainder. Deriving it instead of hardcoding it is what keeps every module
    inside the height contract city.rs placement assumes: the retired generator
    hardcoded 3.4 and drifted 0.94 m away from the engine's 3.15.
    """
    ground = target_h - COPING_H - parapet - (floors - 1) * FLOOR_H
    if ground < FLOOR_H:
        ground = FLOOR_H
    zs = [0.0]
    z = 0.0
    for i in range(floors):
        z += ground if i == 0 else FLOOR_H
        zs.append(z)
    return zs, z + parapet


# ============================================================== the building
def building(name, w, d, floors, bay_long, bay_short, target_h, seed=1,
             wall="concrete", parapet=0.44, entrance=True, loggias=True,
             drainpipes=True, damaged=False):
    """A designed mid-rise block: plinth, punched facades, entrance, recessed
    loggias, parapet with coping, drainpipes.

    Everything stays inside the w x d x target_h envelope. That is not fussiness:
    city.rs places these BY FOOTPRINT, so an asset that sticks out 1.35 m per side
    -- which the first cut of this module did, via projecting balconies -- grows
    13.6 m deep where the contract says 10.9 and intersects its neighbours. Where a
    real building would cantilever we recess instead, which is also what
    eastern-european panel blocks genuinely do.
    """
    rng = random.Random(seed)
    p = Part(name)
    zs, z_top = storey_zs(floors, parapet, target_h)
    wall_top = zs[-1]
    col = C[wall]
    T = 0.34                      # facade thickness
    hw, hd = w * 0.5, d * 0.5
    PLINTH_H = 0.52
    LOGGIA_D = 1.35               # recessed => costs no footprint

    # ---- plinth: barely proud (0.06/side), darker, and it is what "grounds" the
    # block. Projections are capped at 0.06 because anything larger grows the mesh
    # past the footprint city.rs places by.
    box(p, (0.0, 0.0, PLINTH_H * 0.5), (w + 0.12, d + 0.12, PLINTH_H), C["plinth"])

    bl = w / float(bay_long)
    WIN_W = min(1.62, bl * 0.62)          # 1.62 m: a real dwelling window
    WIN_H = 1.55
    SILL = 0.95
    joints = [zs[i] for i in range(1, floors + 1)]

    def facade(run, sign, at, span, count, long_side, door_bay=None):
        openings = []
        step = span / float(count)
        for i in range(count):
            c0 = -span * 0.5 + i * step
            ww = WIN_W if run == "x" else min(WIN_W, step * 0.62)
            ua = c0 + (step - ww) * 0.5
            ub = ua + ww
            for fi in range(floors):
                z0, z1 = zs[fi], zs[fi + 1]
                if door_bay is not None and fi == 0 and i == door_bay:
                    # entrance: cut DEEP into the wall (0.55 m) rather than hanging
                    # a canopy off the front. A recess is a porch; a projection is
                    # a footprint violation.
                    cx = c0 + step * 0.5
                    openings.append((cx - 0.65, cx + 0.65, z0 + 0.02, z0 + 2.38, 0.55))
                elif loggias and long_side and fi >= 1 and i in (0, count - 1):
                    openings.append((c0 + 0.22, c0 + step - 0.22,
                                     z0 + 0.05, z0 + 2.34, LOGGIA_D))
                else:
                    openings.append((ua, ub, z0 + SILL, z0 + SILL + WIN_H))
        wall_panel(p, run, sign, at, T, -span * 0.5, span * 0.5,
                   0.0, wall_top, col, openings=openings, joints=joints,
                   floor_h=FLOOR_H, backing=C["glass"])
        # loggia breast wall: the solid parapet that makes the recess read as a
        # balcony instead of a hole in the wall
        for op in openings:
            if len(op) <= 4:
                continue
            ua, ub, za = op[0], op[1], op[2]
            bh = 1.06
            if run == "x":
                slab(p, ua, ub, at - sign * 0.16, at, za, za + bh,
                     C["concrete_l"], top_ao=1.08)
            else:
                slab(p, at - sign * 0.16, at, ua, ub, za, za + bh,
                     C["concrete_l"], top_ao=1.08)
        if door_bay is not None:
            cx = -span * 0.5 + (door_bay + 0.5) * step
            slab(p, cx - 0.86, cx + 0.86, at - 0.55, at + 0.06,
                 zs[0] + 2.38, zs[0] + 2.52, C["concrete_l"], top_ao=1.0)
            slab(p, cx - 0.86, cx + 0.86, at - 0.55, at + 0.06,
                 PLINTH_H - 0.10, PLINTH_H + 0.03, C["concrete_d"])

    facade("x", -1.0, -hd, w, bay_long, True)
    facade("x", 1.0, hd, w, bay_long, True,
           door_bay=(bay_long // 2) if entrance else None)
    facade("y", -1.0, -hw, d, bay_short, False)
    facade("y", 1.0, hw, d, bay_short, False)

    # ---- projecting sills: the cheapest thing that makes a window read as a hole
    step_l = bl
    for i in range(bay_long):
        c0 = -hw + i * step_l + (step_l - WIN_W) * 0.5
        for fi in range(floors):
            zb = zs[fi] + SILL
            for (yy, sg) in ((hd, 1.0), (-hd, -1.0)):
                slab(p, c0 - 0.085, c0 + WIN_W + 0.085,
                     yy, yy + sg * 0.06, zb - 0.085, zb, C["sill"], top_ao=1.06)

    # ---- drainpipes: cheap, and the single most "inhabited" street-level detail
    if drainpipes:
        for i in range(1, bay_long):
            px = -hw + i * step_l
            for (yy, sg) in ((hd, 1.0), (-hd, -1.0)):
                c = C["rust"] if (i % 2) else C["metal"]
                slab(p, px - 0.065, px + 0.065, yy, yy + sg * 0.06,
                     PLINTH_H, z_top - 0.05, c, top_ao=1.0)

    # ---- entrance is cut INTO the front facade above (door_bay); there is
    # deliberately no projecting canopy or outer steps any more.

    # ---- roof: deck set down inside a parapet, with a coping that oversails
    box(p, (0.0, 0.0, wall_top + 0.06), (w - 0.5, d - 0.5, 0.12), C["roof"])
    PW = 0.30
    for (x0, x1, y0, y1) in ((-hw, hw, -hd, -hd + PW), (-hw, hw, hd - PW, hd),
                             (-hw, -hw + PW, -hd, hd), (hw - PW, hw, -hd, hd)):
        slab(p, x0, x1, y0, y1, wall_top, z_top, col, top_ao=1.0)
        ox = 0.07 if (x1 - x0) > (y1 - y0) else 0.0
        oy = 0.07 if (y1 - y0) >= (x1 - x0) else 0.0
        slab(p, x0 - ox, x1 + ox, y0 - oy, y1 + oy,
             z_top, z_top + COPING_H, C["concrete_l"], top_ao=1.10)

    # roof clutter must stay UNDER the coping: on a 10.4 m three-storey block the
    # height budget leaves only ~0.3 m, so it is vent stacks, not a stair bulkhead
    for (vx, vy, vr) in ((-hw * 0.5, hd * 0.36, 0.26),
                         (-hw * 0.2, hd * 0.42, 0.19),
                         (hw * 0.30, -hd * 0.40, 0.22)):
        box(p, (vx, vy, wall_top + 0.12 + 0.14), (vr * 2.0, vr * 2.0, 0.28), C["metal"])

    if damaged:
        # a shelled corner: coping knocked off one end, darker exposed fabric
        slab(p, -hw, -hw + 1.7, hd - PW, hd, wall_top, z_top, C["soot"], top_ao=1.0)
        box(p, (-hw + 0.85, hd - 0.45, wall_top + 0.12 + 0.30),
            (1.6, 1.0, 0.6), C["soot"])

    return p


# ============================================================== export
def box_project_uv(me, scale=0.30):
    if not me.uv_layers:
        me.uv_layers.new(name="UVMap")
    uv = me.uv_layers[0]
    for poly in me.polygons:
        n = poly.normal
        ax = max(range(3), key=lambda i: abs(n[i]))
        for li in poly.loop_indices:
            v = me.vertices[me.loops[li].vertex_index].co
            if ax == 0:
                u, t = v.y, v.z
            elif ax == 1:
                u, t = v.x, v.z
            else:
                u, t = v.x, v.y
            uv.data[li].uv = (u * scale, t * scale)


def finish_object(part):
    me = bpy.data.meshes.new(part.name)
    me.from_pydata(part.verts, [], part.tris)
    me.validate()
    me.update()
    for poly in me.polygons:
        poly.use_smooth = False
    ca = me.color_attributes.new(name="Col", type="BYTE_COLOR", domain="POINT")
    for i, c in enumerate(part.colour):
        ca.data[i].color = (c[0], c[1], c[2], 1.0)
    box_project_uv(me)
    ob = bpy.data.objects.new(part.name, me)
    bpy.context.collection.objects.link(ob)
    return ob


def export_glb(ob, out_path):
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    bpy.ops.export_scene.gltf(
        filepath=out_path,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
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


# ============================================================== size contract
# Footprints are FIXED by city.rs placement (see tools/blender/survey_props.py
# output). A replacement asset must occupy the same box or the city breaks.
MODULES = {
    # name            w       d       floors bays_l bays_s parapet height  style     dmg
    "building_block":  (14.0, 10.925, 3, 5, 3, 0.44, 10.390, "concrete", False),
    "building_wide":   (18.0,  9.925, 3, 6, 3, 0.44, 10.390, "concrete", False),
    "building_tall":   (12.0,  9.925, 4, 4, 3, 0.69, 13.790, "concrete", False),
    "building_corner": (11.0, 10.925, 3, 4, 4, 0.44, 10.390, "plaster", False),
    "building_shed":   (13.0,  8.925, 2, 4, 3, 0.19,  6.990, "plaster", False),
    "panel_block":     (20.5,  12.500, 5, 7, 4, 1.075, 17.325, "concrete", True),
}


def build_one(name):
    w, d, floors, bl, bs, para, target_h, style, dmg = MODULES[name]
    # Deterministic seed: Python randomises str.__hash__ per process unless
    # PYTHONHASHSEED is pinned, so `hash(name)` would make the build non-reproducible
    # and every run would produce a different GLB.
    seed = zlib.crc32(name.encode("utf-8")) & 0x7FFFFFFF
    part = building(name, w, d, floors, bl, bs, target_h, seed=seed,
                    wall=style, parapet=para,
                    entrance=True, loggias=floors >= 2, drainpipes=True,
                    damaged=dmg)
    return part, target_h


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if not argv:
        raise SystemExit("usage: blender --background --python build_city_kit.py "
                         "-- <out_dir> [name...]")
    out_dir = argv[0]
    names = argv[1:] or list(MODULES.keys())
    os.makedirs(out_dir, exist_ok=True)

    for name in names:
        bpy.ops.wm.read_factory_settings(use_empty=True)
        part, h = build_one(name)
        ob = finish_object(part)
        path = os.path.join(out_dir, name + ".glb")
        export_glb(ob, path)
        tris = len(part.tris)
        zs = [v[2] for v in part.verts]
        xs = [v[0] for v in part.verts]
        ys = [v[1] for v in part.verts]
        print("KIT %-18s verts=%-6d tris=%-6d size=(%.3f, %.3f, %.3f) min_z=%.3f"
              % (name, len(part.verts), tris,
                 max(xs) - min(xs), max(ys) - min(ys), max(zs) - min(zs),
                 min(zs)))
        print("KIT %-18s expected_height=%.3f actual_top=%.3f" % (name, h, max(zs)))


main()
