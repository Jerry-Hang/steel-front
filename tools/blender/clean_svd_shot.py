"""Turn the `svd_63_-_dragunov` product shot into ONE usable rifle (headless, no GUI).

Why this step exists
--------------------
The raw source (`D:\\Rust\\3D\\svd_63_-_dragunov.glb`, 44 MB) is a Sketchfab **product
shot**: two complete Dragunov bodies 90 degrees apart (a side view and a top view), a loose
scope, a spare magazine, loose bullets, and a set of "Wire" outline helpers, all laid out on
a backdrop. `prep_guns.py` joins every mesh in the file, so on this source its own report
says *"the merged mesh's length and up axes are then meaningless"* (`dup_warn: true`) and
`install_guns.py` SKIPs it -- which is why the engine currently draws `svd12` from the
procedural box fallback in `src/engine/guns/dmr.rs`.

What it does
------------
1. keeps `Dragunov_Unwrapped_0` -- the SIDE view (length along X, height along Z, so it is
   already upright in Blender's Z-up frame);
2. keeps `Scope_Unwrapped_0` and **seats it on the receiver**: the mount height is
   *measured* from the body (max Z over the receiver's X range), never guessed, and the
   scope is centred across the body's thickness;
3. drops everything else;
4. exports one GLB for `prep_guns.py`, which then does the usual canonical rotation
   (muzzle -> +Z), texture-to-vertex-colour bake, decimation and budget check.

🔴 Keep `export_materials="EXPORT"` here: this source has **no vertex colours** -- the colour
lives in its textures, and `prep_guns.py` bakes texture -> COLOR_0 itself before exporting
with materials off. Exporting the intermediate with materials OFF (the first attempt) threw
the textures away and the whole rifle came out flat 0.35 grey, which `prep_guns` then
reported as NOT CLEAN (`distinct >= 2` fails).

Usage
-----
  blender.exe --background --python tools/blender/clean_svd_shot.py -- <out.glb>

Then:
  blender.exe --background --python tools/blender/prep_guns.py -- \
      --in <dir with the cleaned glb> --out assets/guns_ext --only svd
  python tools/install_guns.py
"""
import os
import sys

import bpy

SRC = r"D:\Rust\3D\svd_63_-_dragunov.glb"
KEEP_BODY = "Dragunov_Unwrapped_0"
KEEP_SCOPE = "Scope_Unwrapped_0"
# Receiver X range (world units, before normalisation) over which the top surface is the
# mounting plane. The front sight block and the muzzle brake sit outside it.
RECEIVER_X = (-0.16, 0.14)
# A Dragunov's scope sits slightly forward of the receiver's middle.
SCOPE_FORWARD_M = 0.02


def world_pts(ob):
    return [ob.matrix_world @ v.co for v in ob.data.vertices]


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if not argv:
        raise SystemExit("usage: clean_svd_shot.py -- <out.glb>")
    out = argv[0]

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=SRC)

    body = bpy.data.objects.get(KEEP_BODY)
    scope = bpy.data.objects.get(KEEP_SCOPE)
    if body is None or scope is None:
        raise SystemExit("clean_svd_shot: missing object: body=%s scope=%s" % (body, scope))

    pts = world_pts(body)
    xs = [p.x for p in pts]
    zs = [p.z for p in pts]
    print("SVDPROBE body x=[%.3f, %.3f] z=[%.3f, %.3f]"
          % (min(xs), max(xs), min(zs), max(zs)))

    top = max(p.z for p in pts if RECEIVER_X[0] <= p.x <= RECEIVER_X[1])
    sp = world_pts(scope)
    s_cx = (max(p.x for p in sp) + min(p.x for p in sp)) * 0.5
    s_cy = (max(p.y for p in sp) + min(p.y for p in sp)) * 0.5
    s_zmin = min(p.z for p in sp)
    print("SVDPROBE receiver top z=%.4f ; scope zmin=%.4f xcentre=%.4f" % (top, s_zmin, s_cx))

    scope.location.x += (0.0 - s_cx) + SCOPE_FORWARD_M
    scope.location.y += 0.0 - s_cy
    scope.location.z += top - s_zmin
    bpy.context.view_layer.update()
    print("SVDPROBE scope moved by (%.4f, %.4f, %.4f)"
          % ((0.0 - s_cx) + SCOPE_FORWARD_M, -s_cy, top - s_zmin))

    dropped = []
    for ob in list(bpy.data.objects):
        if ob.name in (KEEP_BODY, KEEP_SCOPE):
            continue
        dropped.append(ob.name)
        bpy.data.objects.remove(ob, do_unlink=True)
    print("SVDPROBE dropped %d object(s): %s" % (len(dropped), ", ".join(sorted(dropped))))

    bpy.ops.object.select_all(action="SELECT")
    d = os.path.dirname(os.path.abspath(out))
    if d:
        os.makedirs(d, exist_ok=True)
    bpy.ops.export_scene.gltf(
        filepath=out,
        export_format="GLB",
        export_apply=True,
        export_yup=True,
        export_normals=False,
        export_materials="EXPORT",   # see the module docstring: prep bakes texture -> COLOR_0
        export_vertex_color="NAME",
        export_vertex_color_name="Col",
    )
    print("SVDPROBE wrote %s (%d bytes)" % (out, os.path.getsize(out)))


if __name__ == "__main__":
    main()
