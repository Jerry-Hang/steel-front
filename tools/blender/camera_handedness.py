# -*- coding: utf-8 -*-
"""标定 prep_guns.py `shoot()` 的相机手性：把两个明显不同的色块放到世界 +Y / -Y 上，
用 shoot() 里同一台侧视相机（位于 -X、to_track_quat("-Z","Y")）渲染一次，
直接看图就知道"画面左右"对应哪个 Y 方向。不靠任何推导。

用法：
  blender --background --python tools/blender/camera_handedness.py -- [输出PNG]
"""
import bpy
import sys
from mathutils import Vector

ARGV = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
OUT = ARGV[0] if ARGV else r"D:\Rust\steel-front\logs\gunprobe\handedness.png"

for ob in list(bpy.data.objects):
    bpy.data.objects.remove(ob, do_unlink=True)

sc = bpy.context.scene
sc.render.engine = "BLENDER_WORKBENCH"


def block(name, loc, rgb, size=(0.5, 0.5, 0.5)):
    me = bpy.data.meshes.new(name)
    ob = bpy.data.objects.new(name, me)
    bpy.context.collection.objects.link(ob)
    verts = [(x, y, z) for x in (-1, 1) for y in (-1, 1) for z in (-1, 1)]
    faces = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 2, 6, 4),
             (1, 5, 7, 3), (2, 3, 7, 6), (0, 4, 5, 1)]
    me.from_pydata([(v[0] * size[0], v[1] * size[1], v[2] * size[2]) for v in verts], [], faces)
    me.update()
    ob.location = loc
    mat = bpy.data.materials.new(name + "_m")
    mat.diffuse_color = (*rgb, 1.0)
    ob.data.materials.append(mat)
    return ob


# +Y 红、-Y 蓝、+X 绿（绿块用来同时标定画面里 -X 是"靠相机"这一事实）
block("plusY", (0.0, 1.2, 0.0), (1.0, 0.0, 0.0))
block("minusY", (0.0, -1.2, 0.0), (0.0, 0.0, 1.0))
block("origin", (0.0, 0.0, 0.0), (0.6, 0.6, 0.6), (0.15, 0.15, 0.15))

sc.render.resolution_x = 720
sc.render.resolution_y = 480

# 与 prep_guns.py shoot() 的 "side" 机位完全同款：相机在 -X，看向 +X，up 取世界 +Z
cd = bpy.data.cameras.new("shotcam")
cd.lens = 50.0
cam = bpy.data.objects.new("shotcam", cd)
bpy.context.collection.objects.link(cam)
cam.location = Vector((-3.3, 0.0, 0.12))
dire = (Vector((0, 0, 0)) - cam.location).normalized()
cam.rotation_euler = dire.to_track_quat("-Z", "Y").to_euler()
sc.camera = cam

sh = sc.display.shading
sh.color_type = "MATERIAL"
sh.light = "FLAT"
sh.show_shadows = False

sc.render.filepath = OUT
bpy.ops.render.render(write_still=True)
print("SAVED %s" % OUT)
print("RED=+Y  BLUE=-Y  灰=原点.  看红色块在画面左边还是右边即可判定画面左右对应的 Y 方向。")
