"""A designed soldier module for Steel Front -- replaces the 18-box stack.

WHY THIS FILE EXISTS
  `renderer.rs::soldier_part_matrices` builds a soldier from 18 instanced primitives
  (10 boxes + 8 cylinders) whose sizes live in a table. As a distant LOD that is fine:
  at 40 m a box stack reads as a person and 18 instances is cheap. But at 3-5 m -- the
  range the user complained about ("looks like a god", i.e. not human) -- a box stack
  reads as machinery, and no amount of retuning that table fixes it. AGENTS.md 閾佸緥 D:
  with no normal slot, detail has to become REAL GEOMETRY.

  Authoring it here also settles an argument we lost on 2026-09-12: in this file every
  number is a metre, so the dimension is never in doubt. The 18-box table's header
  comment already said so ("box scale = (width, height, thickness)") -- we misread it
  as a half-extent for three rounds and "corrected" correct values. See 鏁欒 34.

WHAT THIS BUILDS
  1.79 m infantryman, real proportions: head 0.23 m (13% of height), shoulders 0.44 m,
  foot 0.26 m, rifle 0.94 m (AK-12 overall). Sky-exposure AO is baked into vertex
  colour with `exposure_ao`. ONE mesh, ONE instance slot per soldier (the box path
  spends 18).

ENGINE CONVENTIONS (AGENTS.md 閾佸緥 D -- do not deviate)
  1 unit = 1 m; origin at the BOTTOM CENTRE of the footprint; z = 0 is ground.
  Blender +Z up; export_yup=True. Single mesh, single primitive, NO object transform.
  Winding outward-CCW (props::merge reverses it). Vertex colour layer "Col", BYTE_COLOR
  on POINT domain. export_materials="NONE" -- all appearance comes from vertex colour.

Usage (headless only -- AGENTS.md 閾佸緥 D forbids driving the GUI):
  blender.exe --background --python tools/blender/build_soldier.py -- <out_dir> [name]
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import bpy  # noqa: E402

from build_city_kit import (  # noqa: E402
    Part,
    _ao,
    add_quad_n,
    box,
    exposure_ao,
    export_glb,
    finish_object,
)

# ------------------------------------------------------------------ palette
FATIGUE = [0.196, 0.205, 0.146]
FATIGUE_DARK = [0.150, 0.160, 0.113]
ARMOUR = [0.108, 0.119, 0.090]
ARMOUR_LIGHT = [0.142, 0.154, 0.117]
BOOT = [0.048, 0.046, 0.044]
SKIN = [0.352, 0.266, 0.207]
HELMET = [0.124, 0.132, 0.109]
WEBBING = [0.170, 0.160, 0.132]
GUNMETAL = [0.070, 0.072, 0.076]
WOOD = [0.150, 0.108, 0.066]

TOTAL_H = 1.79
# `exposure_ao(z, ref)` returns the sky-exposure factor for a point at height z, where
# `ref` is the height at which exposure reaches 1.0. The city kit passes a BUILDING
# storey (3.15). For a 1.79 m man that is wrong: it squashes every body part into the
# dark end of the curve, and it disagreed with `limb()` below, which made the trousers
# render lighter than the jacket -- two different uniforms on one soldier (first
# preview). One reference for the whole figure:
FLOOR_H = 1.79


def sbox(part, centre, size, col, ao_top=1.0, under_ao=0.42):
    """Axis-aligned box with sky-exposure AO on its four sides.

    Local instead of the kit's `box()` because that one hardcodes a 3.15 m storey.
    Sides take the exposure of their own per-face SHADING normal's height band, which
    for a box means the box's vertical centre -- enough to make a helmet read brighter
    than a boot without a normal slot.
    """
    cx, cy, cz = centre
    hx, hy, hz = size[0] * 0.5, size[1] * 0.5, size[2] * 0.5
    lo, hi = cz - hz, cz + hz
    k_side = _ao(col, exposure_ao((lo + hi) * 0.5, FLOOR_H))
    k_top = _ao(col, exposure_ao(hi, FLOOR_H) * ao_top)
    k_bot = _ao(col, exposure_ao(lo, FLOOR_H) * under_ao)
    a = (cx - hx, cy - hy, lo)
    b = (cx + hx, cy - hy, lo)
    c = (cx + hx, cy + hy, lo)
    d = (cx - hx, cy + hy, lo)
    e = (cx - hx, cy - hy, hi)
    f = (cx + hx, cy - hy, hi)
    g = (cx + hx, cy + hy, hi)
    h = (cx - hx, cy + hy, hi)
    four = (k_side, k_side, k_side, k_side)
    add_quad_n(part, a, b, c, d, (k_bot,) * 4, (0.0, 0.0, -1.0))
    add_quad_n(part, e, f, g, h, (k_top,) * 4, (0.0, 0.0, 1.0))
    add_quad_n(part, a, b, f, e, four, (0.0, -1.0, 0.0))
    add_quad_n(part, b, c, g, f, four, (1.0, 0.0, 0.0))
    add_quad_n(part, c, d, h, g, four, (0.0, 1.0, 0.0))
    add_quad_n(part, d, a, e, h, four, (-1.0, 0.0, 0.0))


def limb(part, p0, p1, r0, r1, col, seg=6, mid=0.5):
    """A tapered prism from p0 to p1. Hexagonal by default.

    Hex, not a 4-gon: at 3-5 m the silhouette of a square limb reads as a post, while a
    hexagon already gives two shaded facets per side. 6 sides x 3 rings ~= 36 quads
    ~= 72 tris per limb, which is affordable once per soldier instead of per segment.
    """
    p0 = list(map(float, p0))
    p1 = list(map(float, p1))
    d = [p1[i] - p0[i] for i in range(3)]
    ln = math.sqrt(sum(v * v for v in d)) or 1e-6
    d = [v / ln for v in d]
    # any vector not parallel to d, to seed the frame
    up = (0.0, 0.0, 1.0) if abs(d[2]) < 0.9 else (1.0, 0.0, 0.0)
    u = [d[1] * up[2] - d[2] * up[1], d[2] * up[0] - d[0] * up[2], d[0] * up[1] - d[1] * up[0]]
    ul = math.sqrt(sum(v * v for v in u)) or 1e-6
    u = [v / ul for v in u]
    v = [d[1] * u[2] - d[2] * u[1], d[2] * u[0] - d[0] * u[2], d[0] * u[1] - d[1] * u[0]]

    steps = ((0.0, r0), (mid, (r0 + r1) * 0.5), (1.0, r1))
    rings = []
    for t, r in steps:
        c = [p0[i] + d[i] * ln * t for i in range(3)]
        ring = []
        for i in range(seg):
            a = 2.0 * math.pi * (i + 0.5) / seg
            ring.append((c[0] + (u[0] * math.cos(a) + v[0] * math.sin(a)) * r,
                         c[1] + (u[1] * math.cos(a) + v[1] * math.sin(a)) * r,
                         c[2] + (u[2] * math.cos(a) + v[2] * math.sin(a)) * r))
        rings.append(ring)

    for k in range(len(rings) - 1):
        lo, hi = rings[k], rings[k + 1]
        for i in range(seg):
            j = (i + 1) % seg
            # the facet mid-angle, used as the INTENDED outward normal
            a = 2.0 * math.pi * (i + 1.0) / seg
            n = (u[0] * math.cos(a) + v[0] * math.sin(a),
                 u[1] * math.cos(a) + v[1] * math.sin(a),
                 u[2] * math.cos(a) + v[2] * math.sin(a))
            k0 = exposure_ao(lo[i][2], FLOOR_H)
            k1 = exposure_ao(hi[i][2], FLOOR_H)
            c = _ao(col, (k0 + k1) * 0.5)
            add_quad_n(part, lo[i], lo[j], hi[j], hi[i], (c, c, c, c), n)


def main() -> int:
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    out_dir = argv[0] if argv else "."
    name = argv[1] if len(argv) > 1 else "soldier"

    p = Part(name)

    # ------------------------------------------------------------ legs (z 0 .. 0.94)
    for s in (-1.0, 1.0):
        x = s * 0.105
        # foot: 0.26 long -- heel behind the ankle, toe forward; 0.10 wide, 0.08 tall
        sbox(p, (x, 0.040, 0.040), (0.100, 0.260, 0.080), BOOT, ao_top=0.58)
        # ankle
        limb(p, (x, 0.0, 0.070), (x, 0.005, 0.140), 0.052, 0.058, BOOT, mid=0.5)
        # shin 0.36 m, tapering
        limb(p, (x, 0.005, 0.130), (x, 0.010, 0.500), 0.058, 0.066, FATIGUE)
        # knee pad -- a real silhouette break at this height
        sbox(p, (x, 0.058, 0.505), (0.130, 0.070, 0.105), ARMOUR, ao_top=0.82)
        # thigh 0.42 m
        limb(p, (x, 0.010, 0.490), (x, 0.0, 0.920), 0.086, 0.072, FATIGUE)

    # ------------------------------------------------------------ pelvis + torso
    sbox(p, (0.0, 0.0, 1.010), (0.310, 0.230, 0.180), FATIGUE_DARK, ao_top=0.88)
    # chest: 0.44 m across the shoulders (real adult male), 0.25 deep, 0.32 tall
    sbox(p, (0.0, 0.0, 1.320), (0.400, 0.250, 0.320), FATIGUE, ao_top=0.97)
    # armour vest, 0.46 x 0.32 x 0.38 -- deliberately the widest body layer
    sbox(p, (0.0, 0.005, 1.305), (0.460, 0.320, 0.380), ARMOUR, ao_top=1.0)
    # front plate so the vest is not one flat slab
    sbox(p, (0.0, 0.170, 1.300), (0.300, 0.045, 0.250), ARMOUR_LIGHT, ao_top=1.0)
    # three pouches across the belt line
    for dx in (-0.130, 0.0, 0.130):
        sbox(p, (dx, 0.198, 1.180), (0.110, 0.055, 0.130), WEBBING, ao_top=0.94)
    # shoulder straps
    for s in (-1.0, 1.0):
        sbox(p, (s * 0.150, 0.0, 1.495), (0.085, 0.290, 0.060), ARMOUR_LIGHT, ao_top=1.0)

    # ------------------------------------------------------------ head + helmet
    limb(p, (0.0, 0.0, 1.510), (0.0, 0.0, 1.600), 0.056, 0.056, SKIN, seg=6)
    # head 0.17 x 0.21 x 0.23 -- 13% of body height, the real proportion
    sbox(p, (0.0, 0.005, 1.665), (0.170, 0.210, 0.230), SKIN, ao_top=0.84)
    # helmet shell 0.255 x 0.285 x 0.145 -- pulled DOWN over the skull so it reads as a
    # helmet rather than a cap floating above it (first preview showed exactly that).
    sbox(p, (0.0, 0.005, 1.765), (0.255, 0.285, 0.145), HELMET, ao_top=1.0)
    # brim
    sbox(p, (0.0, 0.125, 1.735), (0.230, 0.095, 0.048), HELMET, ao_top=0.88)

    # ------------------------------------------------------------ arms (carry pose)
    # Right hand on the pistol grip, left hand under the handguard -- which is what a
    # soldier actually does, and it puts BOTH hands on the weapon instead of leaving
    # them hanging beside it (the failure the old table's comment describes).
    #   shoulder -> elbow: 0.29 m   elbow -> wrist: 0.27 m
    for s in (-1.0, 1.0):
        # Shoulder pivots sit OUTSIDE the 0.46 m vest (+-0.23). At +-0.225 the arms were
        # half-buried in it and the front view simply had no arms (first preview).
        sx = s * 0.268
        sz = 1.455
        if s > 0:  # right: trigger hand, elbow drops back and down
            ex, ey, ez = sx - 0.020, 0.115, 1.230
            wx, wy, wz = sx - 0.055, 0.235, 1.115
        else:      # left: support hand, reaches further forward
            ex, ey, ez = sx + 0.020, 0.145, 1.215
            wx, wy, wz = sx + 0.055, 0.330, 1.170
        limb(p, (sx, 0.0, sz), (ex, ey, ez), 0.062, 0.050, FATIGUE)
        limb(p, (ex, ey, ez), (wx, wy, wz), 0.050, 0.042, FATIGUE)

    # ------------------------------------------------------------ rifle (0.94 m)
    # Held across the chest, muzzle forward (+Y is the model's facing direction here;
    # props::merge/placement handles world orientation). AK-12 overall length 0.94 m,
    # split receiver 0.62 + stock 0.24 + muzzle device, exactly as the 18-box table
    # describes -- but now the number IS metres and there is no half-extent ambiguity.
    ry, rz = 0.285, 1.135
    sbox(p, (0.02, ry + 0.060, rz), (0.075, 0.620, 0.105), GUNMETAL, ao_top=0.92)   # receiver
    sbox(p, (0.02, ry - 0.230, rz - 0.010), (0.062, 0.240, 0.130), WOOD, ao_top=0.90)  # stock
    sbox(p, (0.02, ry + 0.400, rz + 0.010), (0.038, 0.190, 0.052), GUNMETAL, ao_top=0.95)  # barrel
    sbox(p, (0.02, ry + 0.030, rz - 0.105), (0.055, 0.150, 0.130), GUNMETAL, ao_top=0.80)  # magazine
    sbox(p, (0.02, ry + 0.150, rz + 0.085), (0.045, 0.220, 0.045), GUNMETAL, ao_top=0.98)  # optic

    ob = finish_object(p)
    out = os.path.join(out_dir, name + ".glb")
    export_glb(ob, out)
    me = ob.data
    me.calc_loop_triangles()
    print(f"SOLDIER {name}: verts={len(me.vertices)} tris={len(me.loop_triangles)} -> {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
