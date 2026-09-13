"""Measure how many vertices a generated mesh wastes on unwelded duplicates.

WHY
  `build_city_kit.finish_object` builds the mesh with `from_pydata(verts, [], tris)`,
  where every quad pushed 4 brand-new vertices. Nothing is ever welded, so a mesh with
  N triangles can carry ~3N vertices instead of the ~N/2 a welded mesh would need.

  That matters a lot here: the engine renders with FLAT SHADING (no normal slot; normals
  come from screen-space derivatives), so welded vertices cannot break the shading. And
  the props are VERTEX bound -- measured 2026-09-13:
      no props            +37.6% fps
      4x fewer pixels     only +12%   => not fill-rate bound
      1 draw vs 164 bins  -38%        => vertex count drives fps
      tree_oak           2264 verts x 372 placements = 54% of the 1.56M prop buffer

  So: measure pos / pos+color / pos+color+uv weld ratios for every asset.

Usage:
  blender.exe --background --python tools/blender/weld_report.py -- assets/props "*.glb"
"""

import glob
import os
import sys

import bpy  # noqa: E402


def report(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    try:
        bpy.ops.import_scene.gltf(filepath=path)
    except Exception as e:  # noqa: BLE001
        print(f"WELD {os.path.basename(path):28s} import failed: {e}")
        return
    me = None
    for ob in bpy.data.objects:
        if ob.type == "MESH":
            me = ob.data
            break
    if me is None:
        print(f"WELD {os.path.basename(path):28s} no mesh")
        return

    n_tris = len(me.loop_triangles) if me.loop_triangles else 0
    if n_tris == 0:
        me.calc_loop_triangles()
        n_tris = len(me.loop_triangles)
    n_verts = len(me.vertices)

    ca = None
    for c in me.color_attributes:
        ca = c
        break
    uv = me.uv_layers.active

    k_pos = set()
    k_pos_col = set()
    k_all = set()
    for i, v in enumerate(me.vertices):
        p = (round(v.co.x, 5), round(v.co.y, 5), round(v.co.z, 5))
        col = None
        if ca is not None:
            c = ca.data[i].color
            col = (round(c[0], 3), round(c[1], 3), round(c[2], 3))
        u = None
        if uv is not None:
            t = uv.data[i].uv
            u = (round(t[0], 4), round(t[1], 4))
        k_pos.add(p)
        k_pos_col.add((p, col))
        k_all.add((p, col, u))

    name = os.path.basename(path)
    print(
        "WELD %-28s verts=%-6d tris=%-6d pos=%-6d pos+col=%-6d pos+col+uv=%-6d  "
        "savings(pos+col+uv)=%.0f%%"
        % (
            name,
            n_verts,
            n_tris,
            len(k_pos),
            len(k_pos_col),
            len(k_all),
            100.0 * (1.0 - len(k_all) / max(1, n_verts)),
        )
    )


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    root = argv[0] if argv else "assets/props"
    pattern = argv[1] if len(argv) > 1 else "*.glb"
    files = sorted(glob.glob(os.path.join(root, pattern)))
    print(f"WELD dir={root} pattern={pattern} files={len(files)}")
    for f in files:
        report(f)


if __name__ == "__main__":
    main()
