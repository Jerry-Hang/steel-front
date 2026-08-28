import bpy
import mathutils

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=r'D:\Rust\steel-front\assets\guns\ak12.glb')
meshes = [o for o in bpy.data.objects if o.type == 'MESH']
for obj in meshes:
    obj.parent = None
    obj.matrix_world = mathutils.Matrix.Identity(4)
bpy.context.view_layer.update()

for obj in meshes:
    for mat in obj.data.materials:
        if not mat or not mat.use_nodes:
            continue
        nt = mat.node_tree
        bsdf = next((n for n in nt.nodes if n.type == 'BSDF_PRINCIPLED'), None)
        if not bsdf:
            continue
        vc = nt.nodes.new('ShaderNodeVertexColor')
        vc.layer_name = 'Color'
        vc.location = (-300, 200)
        nt.links.new(vc.outputs['Color'], bsdf.inputs['Base Color'])

for obj in meshes:
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.select_all(action='DESELECT')
    obj.select_set(True)
    for attr in obj.data.color_attributes:
        obj.data.color_attributes.remove(attr)
    attr = obj.data.color_attributes.new(name='Color', type='FLOAT_COLOR', domain='CORNER')
    mats = list(obj.data.materials)
    for poly in obj.data.polygons:
        base = (0.055, 0.056, 0.06, 1.0)
        mi = poly.material_index
        if mi < len(mats) and mats[mi] is not None and mats[mi].use_nodes:
            b = next((n for n in mats[mi].node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
            if b:
                raw = b.inputs['Base Color'].default_value
                base = (max(raw[0], 0.05), max(raw[1], 0.052), max(raw[2], 0.058), 1.0)
        for li in poly.loop_indices:
            attr.data[li].color = base

    try:
        bpy.ops.paint.vertex_color_dirt(blur_strength=1.5, blur_iterations=2, clean_angle=0.8, dirt_angle=0.0, dirt_only=False, normalize=False)
    except Exception as e:
        print('dirt:', str(e)[:50])
    obj.select_set(False)

bpy.ops.export_scene.gltf(
    filepath=r'D:\Rust\steel-front\assets\guns\ak12_baked.glb',
    export_format='GLB',
    export_yup=True,
    export_all_vertex_colors=True,
    export_materials='EXPORT',
)
print('EXPORT DONE')