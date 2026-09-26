"""Headless GLB preview renderer.

Imports a .glb and renders several camera angles to PNG so the asset can actually
be LOOKED AT without opening the Blender GUI. The GUI steals focus and the mouse,
which violates the mouse-safety protocol (AGENTS.md 铁律 D) -- never automate it.

Run:
  blender.exe --background --python tools/blender/preview_glb.py -- \
      <in.glb> <out_prefix> [view_count]

Writes <out_prefix>_<i>_<label>.png and prints PREVIEW stats (mesh count, verts,
tris, world bbox) which are the numbers the engine loader cares about.
"""
import sys
import math
import os
import bpy
from mathutils import Vector

# ------------------------------------------------------------------ argv
argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
if len(argv) < 2:
    raise SystemExit("usage: blender --background --python preview_glb.py -- "
                     "<in.glb> <out_prefix> [view_count]")
src_path = argv[0]
# 🔴 2026-09-26：`render.filepath` 是**相对 Blender 自己的基准目录**解析的，不是进程 CWD
# （实测：传 `logs\svdprev` 时文件写到了 `C:\logs\`，而本脚本照样打印
# "PREVIEW wrote logs\svdprev_0_front.png" —— 又是一次"工具说写了、其实写在别处"）。
# ⇒ 在脚本里就把前缀解析成绝对路径、并把目录建出来，打印的也就是真路径。
out_prefix = os.path.abspath(argv[1])
_out_dir = os.path.dirname(out_prefix)
if _out_dir:
    os.makedirs(_out_dir, exist_ok=True)
view_count = int(argv[2]) if len(argv) > 2 else 4

# ------------------------------------------------------------------ clean scene
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene

# ------------------------------------------------------------------ import
bpy.ops.import_scene.gltf(filepath=src_path)

meshes = [o for o in scene.objects if o.type == 'MESH']
if not meshes:
    raise SystemExit("no mesh objects imported from %s" % src_path)

# ------------------------------------------------------------------ stats + bbox
verts = 0
tris = 0
for o in meshes:
    verts += len(o.data.vertices)
    o.data.calc_loop_triangles()
    tris += len(o.data.loop_triangles)

mn = Vector((1e9, 1e9, 1e9))
mx = Vector((-1e9, -1e9, -1e9))
for o in meshes:
    for corner in o.bound_box:
        w = o.matrix_world @ Vector(corner)
        for i in range(3):
            mn[i] = min(mn[i], w[i])
            mx[i] = max(mx[i], w[i])

size = mx - mn
center = (mx + mn) * 0.5
radius = max(size.x, size.y, size.z) * 0.5
if radius <= 1e-6:
    radius = 1.0

print("PREVIEW src=%s" % src_path)
print("PREVIEW meshes=%d verts=%d tris=%d" % (len(meshes), verts, tris))
print("PREVIEW size=(%.3f, %.3f, %.3f) min_z=%.3f center=(%.3f, %.3f, %.3f)"
      % (size.x, size.y, size.z, mn.z, center.x, center.y, center.z))

# ------------------------------------------------------------------ render setup
def pick_engine():
    for name in ("BLENDER_EEVEE_NEXT", "BLENDER_EEVEE", "CYCLES"):
        try:
            scene.render.engine = name
            return name
        except TypeError:
            continue
    return scene.render.engine

engine = pick_engine()
print("PREVIEW engine=%s" % engine)
if engine == "CYCLES":
    scene.cycles.samples = 24
    scene.cycles.use_denoising = False
else:
    try:
        scene.eevee.taa_render_samples = 16
    except Exception:
        pass

scene.render.resolution_x = 900
scene.render.resolution_y = 620
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = 'PNG'
scene.render.film_transparent = False

world = bpy.data.worlds.new("previewWorld")
scene.world = world
world.use_nodes = True
bg = world.node_tree.nodes.get("Background")
if bg:
    bg.inputs[0].default_value = (0.30, 0.34, 0.40, 1.0)
    bg.inputs[1].default_value = 1.0

sun_data = bpy.data.lights.new("prevSun", type='SUN')
sun_data.energy = 3.5
sun_data.angle = math.radians(6.0)
sun = bpy.data.objects.new("prevSun", sun_data)
scene.collection.objects.link(sun)
sun.rotation_euler = (math.radians(52.0), 0.0, math.radians(38.0))

fill_data = bpy.data.lights.new("prevFill", type='SUN')
fill_data.energy = 1.1
fill = bpy.data.objects.new("prevFill", fill_data)
scene.collection.objects.link(fill)
fill.rotation_euler = (math.radians(70.0), 0.0, math.radians(220.0))

# ground plane: lets us judge how the asset SITS (base/plinth) and gives contact
bpy.ops.mesh.primitive_plane_add(size=max(radius * 20.0, 20.0),
                                 location=(center.x, center.y, mn.z))
ground = bpy.context.active_object
gmat = bpy.data.materials.new("prevGround")
gmat.use_nodes = True
gbsdf = gmat.node_tree.nodes.get("Principled BSDF")
if gbsdf:
    gbsdf.inputs["Base Color"].default_value = (0.22, 0.22, 0.23, 1.0)
    gbsdf.inputs["Roughness"].default_value = 0.95
ground.data.materials.append(gmat)

target = bpy.data.objects.new("prevTarget", None)
target.location = center
scene.collection.objects.link(target)

cam_data = bpy.data.cameras.new("prevCam")
cam_data.lens = 50.0
cam = bpy.data.objects.new("prevCam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
con = cam.constraints.new(type='TRACK_TO')
con.target = target
con.track_axis = 'TRACK_NEGATIVE_Z'
con.up_axis = 'UP_Y'

# azimuth (deg, 0 = looking from -Y), elevation (deg from horizon), label
ANGLES = [
    (0.0, 8.0, "front"),
    (90.0, 8.0, "right"),
    (45.0, 26.0, "threequarter"),
    (0.0, 78.0, "top"),
]

dist = radius * 4.2
for i in range(min(view_count, len(ANGLES))):
    az, el, label = ANGLES[i]
    a = math.radians(az)
    e = math.radians(el)
    cam.location = (
        center.x + dist * math.cos(e) * math.sin(a),
        center.y - dist * math.cos(e) * math.cos(a),
        center.z + dist * math.sin(e),
    )
    scene.render.filepath = "%s_%d_%s.png" % (out_prefix, i, label)
    bpy.ops.render.render(write_still=True)
    print("PREVIEW wrote %s" % scene.render.filepath)
