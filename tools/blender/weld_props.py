"""Weld duplicate vertices in the existing prop GLBs -- no regeneration.

WHY (measured 2026-09-13)
  The props are VERTEX bound. Evidence from this machine:
      no props               +37.6% fps
      4x fewer pixels        only +12%     => NOT fill-rate bound
      1 draw vs 164 bins     -38%          => vertex count drives fps
      tree_oak  2264 verts x 372 placements = 842k = 54% of the 1.56M prop buffer

  `build_city_kit.finish_object` builds meshes with `from_pydata(verts, [], tris)`
  where every quad pushed 4 fresh vertices, so nothing is ever shared. `weld_report.py`
  shows what blocks welding:

      tree_oak     verts=2264   pos=458   pos+col=458   pos+col+uv=2038
      (and every other asset looks the same: pos-only welding is 3-7x better than pos+uv)

  **The UV is the blocker**, because `box_project_uv` gives every face its own island.

WHY DROPPING THE UV IS SAFE HERE
  Props are exported with `export_materials="NONE"` and `Shape::Authored` in the engine
  (tint.w == 6.0 => flat_flag 1.25) explicitly SKIPS the four procedural surface effects
  that would sample a UV (window_dark / glass_shade+fresnel / is_canopy noise /
  marker concrete skin). Prop appearance comes entirely from vertex colour. So a constant
  UV costs nothing and lets position+colour welding actually fire.

  The engine's vertex format still needs the uv field (stride 32: pos/color/uv), so we
  keep the attribute -- just make it constant.

WHAT IT DOES
  For each GLB: import, dedup vertices by (position, colour) with a tenth-millimetre
  position tolerance, rewrite the triangles, set every UV to (0.5, 0.5), export with the
  SAME conventions as build_city_kit (single mesh, no transform, vertex colour "Col",
  materials NONE). Geometry is untouched, so the size contract still holds.

Usage:
  blender.exe --background --python tools/blender/weld_props.py -- <in_dir> <out_dir> [name...]
"""

import glob
import os
import sys

import bpy  # noqa: E402


def weld_one(src, out_dir):
    name = os.path.splitext(os.path.basename(src))[0]
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=src)

    ob = None
    for o in bpy.data.objects:
        if o.type == "MESH":
            ob = o
            break
    if ob is None:
        print(f"WELD {name}: no mesh, skipped")
        return None

    me = ob.data
    me.calc_loop_triangles()

    ca = None
    for c in me.color_attributes:
        ca = c
        break

    # Dedup key: quantised position + quantised colour (1e-4 m, 1/1000 colour step).
    key_of = {}
    new_verts = []
    new_cols = []
    remap = []
    for i, v in enumerate(me.vertices):
        p = (round(v.co.x, 4), round(v.co.y, 4), round(v.co.z, 4))
        col = (0.5, 0.5, 0.5, 1.0)
        if ca is not None:
            c = ca.data[i].color
            col = (round(c[0], 3), round(c[1], 3), round(c[2], 3), 1.0)
        k = (p, col)
        j = key_of.get(k)
        if j is None:
            j = len(new_verts)
            key_of[k] = j
            new_verts.append(v.co.copy())
            new_cols.append(col)
        remap.append(j)

    tris = []
    for lt in me.loop_triangles:
        a, b, c = (remap[me.loops[lt.loops[0]].vertex_index],
                   remap[me.loops[lt.loops[1]].vertex_index],
                   remap[me.loops[lt.loops[2]].vertex_index])
        if a == b or b == c or a == c:
            continue  # degenerate after welding (shared edge between coplanar quads)
        tris.append((a, b, c))

    old_v, old_t = len(me.vertices), len(me.loop_triangles)

    nm = bpy.data.meshes.new(name)
    nm.from_pydata([v[:] for v in new_verts], [], tris)
    nm.validate()
    nm.update()
    for poly in nm.polygons:
        poly.use_smooth = False
    nca = nm.color_attributes.new(name="Col", type="BYTE_COLOR", domain="POINT")
    for i, c in enumerate(new_cols):
        nca.data[i].color = c
    # Constant UV: lets position+colour welding be the only constraint. Props never
    # sample a UV (see the module docstring), so this is free.
    uvl = nm.uv_layers.new(name="UVMap")
    for d in uvl.data:
        d.uv = (0.5, 0.5)

    nob = bpy.data.objects.new(name, nm)
    bpy.context.collection.objects.link(nob)

    os.makedirs(out_dir, exist_ok=True)
    out = os.path.join(out_dir, name + ".glb")
    bpy.ops.object.select_all(action="DESELECT")
    nob.select_set(True)
    bpy.context.view_layer.objects.active = nob
    bpy.ops.export_scene.gltf(
        filepath=out,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=True,
        # 🔴🔴 必须关掉！这是焊接能否生效的**总开关**。
        # 纯平着色需要逐面法线，glTF 的 NORMAL 是逐顶点的 ⇒ 导出器会**按面拆分顶点**
        # 来承载不同的法线 ⇒ 焊接成果被完全抵消。
        # 实测（2026-09-13）：开着它时 tree_oak 焊到 458 顶点，落盘后又是 2264（原样）；
        # 整个道具缓冲只从 1563020 降到 1459292（−6.6%），fps 只 +1.7%。
        # 而引擎的顶点格式是 `stride=32, pos/color/uv` —— **根本没有法线槽位**
        # （法线由屏幕空间导数重建，见 AGENTS.md 铁律 B），
        # `assets.rs` 也只把 POSITION 当硬要求（缺失的 NORMAL/UV 有显式回退）。
        export_normals=False,
        export_texcoords=True,
        export_vertex_color="NAME",
        export_vertex_color_name="Col",
        export_materials="NONE",
        export_extras=False,
    )
    print(
        "WELD %-26s verts %5d -> %5d (%.0f%%)  tris %5d -> %5d"
        % (name, old_v, len(new_verts), 100.0 * len(new_verts) / max(1, old_v),
           old_t, len(tris))
    )
    return len(new_verts)


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(argv) < 2:
        print("usage: weld_props.py -- <in_dir> <out_dir> [name...]")
        return
    in_dir, out_dir = argv[0], argv[1]
    names = argv[2:]
    files = sorted(glob.glob(os.path.join(in_dir, "*.glb")))
    if names:
        files = [f for f in files if os.path.splitext(os.path.basename(f))[0] in names]
    print(f"WELD in={in_dir} out={out_dir} files={len(files)}")
    total = 0
    for f in files:
        n = weld_one(f, out_dir)
        if n:
            total += n
    print(f"WELD total verts written: {total}")


if __name__ == "__main__":
    main()
