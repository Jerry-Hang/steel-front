import bpy
import math
import mathutils

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=r'D:\Rust\steel-front\assets\guns\ak12.glb')

for mat in bpy.data.materials:
    if not mat.use_nodes:
        continue
    bsdf = next((n for n in mat.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
    if bsdf:
        bsdf.inputs['Base Color'].default_value = (0.32, 0.34, 0.38, 1.0)
        bsdf.inputs['Metallic'].default_value = 0.85
        bsdf.inputs['Roughness'].default_value = 0.35

meshes = [o for o in bpy.data.objects if o.type == 'MESH']
mins = mathutils.Vector((1e9, 1e9, 1e9))
maxs = mathutils.Vector((-1e9, -1e9, -1e9))
for o in meshes:
    for cc in o.bound_box:
        w = o.matrix_world @ mathutils.Vector(cc)
        mins = mathutils.Vector((min(mins[i], w[i]) for i in range(3)))
        maxs = mathutils.Vector((max(maxs[i], w[i]) for i in range(3)))
c = (mins + maxs) * 0.5
size = (maxs - mins).length

cam_data = bpy.data.cameras.new('Cam')
cam = bpy.data.objects.new('Cam', cam_data)
bpy.context.scene.collection.objects.link(cam)
cam.location = (c.x + size * 0.7, c.y - size * 0.8, c.z + size * 0.5)
# look_at：-Z 前向、Y 上
direction = c - cam.location
cam.rotation_euler = direction.to_track_quat('-Z', 'Y').to_euler()
cam.data.lens = 50
bpy.context.scene.camera = cam

key = bpy.data.lights.new('Key', 'AREA'); key.energy = 1500; key.size = 3
ko = bpy.data.objects.new('Key', key); bpy.context.scene.collection.objects.link(ko)
ko.location = (c.x + size * 0.6, c.y - size * 0.7, c.z + size * 0.9)
ko.rotation_euler = (math.radians(40), math.radians(12), 0)

fill = bpy.data.lights.new('Fill', 'AREA'); fill.energy = 500; fill.size = 4
fo = bpy.data.objects.new('Fill', fill); bpy.context.scene.collection.objects.link(fo)
fo.location = (c.x - size * 0.8, c.y - size * 0.3, c.z + size * 0.3)
fo.rotation_euler = (math.radians(70), 0, math.radians(-50))

world = bpy.data.worlds.new('W')
bpy.context.scene.world = world
world.use_nodes = True
world.node_tree.nodes['Background'].inputs[0].default_value = (0.5, 0.5, 0.53, 1.0)

scene = bpy.context.scene
scene.render.engine = 'BLENDER_EEVEE'
scene.render.resolution_x = 1280
scene.render.resolution_y = 800
scene.render.filepath = r'D:\Rust\steel-front\screenshots\blender_demo.png'
bpy.ops.render.render(write_still=True)
print('DONE')