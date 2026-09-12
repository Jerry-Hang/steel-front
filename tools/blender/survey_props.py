"""Survey every GLB in a directory: footprint, height, base offset, verts, tris.

One Blender launch for the whole directory -- importing 24 files one at a time in
separate launches is ~3s of startup each for nothing.

These numbers are the SIZE CONTRACT: city.rs places props by index and the
footprint/height of each mesh decides how the city reads, so a replacement asset
must fit the same box.

Run:
  blender.exe --background --python tools/blender/survey_props.py -- <dir> [glob]
"""
import sys
import os
import glob
import bpy
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
if not argv:
    raise SystemExit("usage: blender --background --python survey_props.py -- "
                     "<dir> [glob]")
root = argv[0]
pattern = argv[1] if len(argv) > 1 else "*.glb"

files = sorted(glob.glob(os.path.join(root, pattern)))
print("SURVEY dir=%s pattern=%s files=%d" % (root, pattern, len(files)))
print("SURVEY %-24s %8s %8s %8s %8s %8s %8s"
      % ("name", "size_x", "size_y", "size_z", "min_z", "verts", "tris"))

for f in files:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=f)
    meshes = [o for o in bpy.context.scene.objects if o.type == 'MESH']
    if not meshes:
        print("SURVEY %-24s  NO MESH" % os.path.basename(f))
        continue

    verts = 0
    tris = 0
    for o in meshes:
        verts += len(o.data.vertices)
        o.data.calc_loop_triangles()
        tris += len(o.data.loop_triangles)

    mn = Vector((1e9, 1e9, 1e9))
    mx = Vector((-1e9, -1e9, -1e9))
    for o in meshes:
        for c in o.bound_box:
            w = o.matrix_world @ Vector(c)
            for i in range(3):
                mn[i] = min(mn[i], w[i])
                mx[i] = max(mx[i], w[i])

    size = mx - mn
    print("SURVEY %-24s %8.3f %8.3f %8.3f %8.3f %8d %8d"
          % (os.path.basename(f), size.x, size.y, size.z, mn.z, verts, tris))
