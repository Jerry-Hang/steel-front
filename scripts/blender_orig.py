import bpy
import mathutils

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=r'D:\Rust\steel-front\assets\guns\ak12.glb')
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
cam.rotation_euler = (c - cam.location).to_track_quat('-Z', 'Y').to_euler()
bpy.context.scene.camera = cam
world = bpy.data.worlds.new('W')
bpy.context.scene.world = world
world.use_nodes = True
world.node_tree.nodes['Background'].inputs[0].default_value = (0.5, 0.5, 0.53, 1.0)
scene = bpy.context.scene
scene.render.engine = 'BLENDER_EEVEE'
scene.render.resolution_x = 1280
scene.render.resolution_y = 800
scene.render.filepath = r'D:\Rust\steel-front\screenshots\ak12_original.png'
bpy.ops.render.render(write_still=True)
print('ORIG RENDER DONE')