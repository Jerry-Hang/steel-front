"""生成**程序化 LOD 代理网格**（树干 + 若干球冠），用于远景替换高模。

为什么需要它（2026-10-31）：树的真实资产是"叶片卡片 + 顶点色"，抽面到 421 三角后
顶点数**卡在 2166**（每片叶子的色块都要拆顶点），做不出真正的低模。LOD 代理是行业
常规做法：远景只保留剪影（一根干 + 两三团冠），近景仍用高模。

产物遵循引擎道具契约：单 mesh / 单 primitive、POSITION + 顶点色、原点在底面中心、
1 单位 = 1 米、节点不带变换、不导出法线。

用法：
  blender -b --python tools/blender/make_lod_proxy.py -- <out.glb> --height 7.708 \
      --trunk-radius 0.18 --blobs 3 [--name tree_oak_lod1]
"""

import argparse
import math
import sys

import bpy

TRUNK = (0.30, 0.22, 0.14)
CANOPY_A = (0.20, 0.34, 0.14)
CANOPY_B = (0.17, 0.30, 0.12)


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--height", type=float, default=7.0)
    ap.add_argument("--trunk-radius", type=float, default=0.16)
    ap.add_argument("--blobs", type=int, default=3)
    ap.add_argument("--blob-scale", type=float, default=0.62)
    ap.add_argument("--name", default="lod_proxy")
    return ap.parse_args(argv)


def add_cone(radius, height, z0, color):
    bpy.ops.mesh.primitive_cone_add(
        vertices=7, radius1=radius, radius2=radius * 0.55, depth=height, location=(0, 0, z0 + height * 0.5)
    )
    o = bpy.context.object
    paint(o, color)
    return o


def add_ico(radius, loc, color):
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1, radius=radius, location=loc)
    o = bpy.context.object
    paint(o, color)
    return o


def paint(obj, color):
    mesh = obj.data
    attr = mesh.color_attributes.get("Col")
    if attr is None:
        attr = mesh.color_attributes.new(name="Col", type="FLOAT_COLOR", domain="CORNER")
    for d in attr.data:
        d.color = (color[0], color[1], color[2], 1.0)
    mesh.color_attributes.active_color = attr


def main():
    a = parse_args()
    bpy.ops.wm.read_factory_settings(use_empty=True)

    h = a.height
    trunk_h = h * 0.55
    add_cone(a.trunk_radius, trunk_h, 0.0, TRUNK)

    # 冠：一团主冠 + 若干副冠，整体高度压在 h 以内
    r0 = h * a.blob_scale * 0.5
    add_ico(r0, (0.0, 0.0, trunk_h + r0 * 0.85), CANOPY_A)
    for i in range(max(0, a.blobs - 1)):
        ang = (i + 1) * (2.0 * math.pi / max(1, a.blobs - 1))
        rr = r0 * 0.68
        add_ico(
            rr,
            (math.cos(ang) * r0 * 0.72, math.sin(ang) * r0 * 0.72, trunk_h + r0 * 0.55),
            CANOPY_B,
        )

    # 合并成一个 mesh
    for o in bpy.context.scene.objects:
        o.select_set(o.type == "MESH")
    bpy.context.view_layer.objects.active = bpy.context.scene.objects[0]
    bpy.ops.object.join()
    obj = bpy.context.object
    obj.name = a.name

    # 契约：无节点变换 → 先 apply，再在**顶点空间**做"贴地 + 缩放到目标高度"。
    # 🔴 顺序不能反（2026-10-31 实测）：先减 zmin 再 apply(location) 会把物体自身的
    # location 又加回来 ⇒ 代理整体悬空（首次生成 y∈[2.12, 11.07]，高 11m 而不是 7.7m）。
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    me = obj.data
    zmin = min(v.co.z for v in me.vertices)
    zmax = max(v.co.z for v in me.vertices)
    k = a.height / max(1e-4, zmax - zmin)
    for v in me.vertices:
        v.co.z = (v.co.z - zmin) * k
        v.co.x *= k
        v.co.y *= k
    me.update()

    verts = len(me.vertices)
    tris = sum(len(p.vertices) - 2 for p in me.polygons)
    bpy.ops.export_scene.gltf(
        filepath=a.out,
        export_format="GLB",
        use_selection=True,
        export_apply=True,
        export_yup=True,
        export_normals=False,
        export_texcoords=False,
        export_vertex_color="NAME",
        export_vertex_color_name="Col",
        export_materials="NONE",
    )
    print(f"[lod_proxy] {a.out}: {verts} 顶点 / {tris} 三角，高 {max(v.co.z for v in me.vertices):.3f} m")


main()
