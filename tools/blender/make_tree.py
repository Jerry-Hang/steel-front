"""生成**实心树冠**的树（替代会变成"爆炸头"的叶片卡片树）。

为什么不用原资产（2026-11-02）：`tree_oak.glb` 的树冠是**叶片卡片**，靠贴图 alpha 抠出叶形。
本流水线把颜色烘进顶点色、**不导出 alpha** ⇒ 每片叶卡都成了实心多边形，剪影就是玩家说的
"爆炸头"。与其想办法把 alpha 塞进一条只吃顶点色的管线，不如把树冠做成**真几何**：
干 + 3~4 团多面体冠，远看仍是树，近看不再是碎纸片。

契约：单 mesh / POSITION + 顶点色 / 原点在底面中心 / 1 单位=1 米 / 不导出法线。

用法：
  blender -b --python tools/blender/make_tree.py -- <out.glb> [--height 7.708] [--seed 1]
"""

import argparse
import math
import random
import sys

import bpy

BARK = (0.30, 0.22, 0.15)
LEAF_A = (0.17, 0.30, 0.10)
LEAF_B = (0.22, 0.36, 0.13)
LEAF_C = (0.13, 0.24, 0.08)


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--height", type=float, default=7.708)
    ap.add_argument("--seed", type=int, default=1)
    return ap.parse_args(argv)


def paint(obj, rgb):
    me = obj.data
    attr = me.color_attributes.get("Col")
    if attr is None:
        attr = me.color_attributes.new(name="Col", type="FLOAT_COLOR", domain="CORNER")
    for d in attr.data:
        d.color = (rgb[0], rgb[1], rgb[2], 1.0)
    me.color_attributes.active_color = attr


def main():
    a = args()
    rnd = random.Random(a.seed)
    bpy.ops.wm.read_factory_settings(use_empty=True)

    h = a.height
    trunk_h = h * 0.42
    trunk_r = h * 0.035

    # 干：略微收分的圆柱（12 段足够；树皮是平面着色也读得出来）
    bpy.ops.mesh.primitive_cone_add(
        vertices=12, radius1=trunk_r, radius2=trunk_r * 0.55, depth=trunk_h,
        location=(0, 0, trunk_h * 0.5),
    )
    trunk = bpy.context.object
    paint(trunk, BARK)

    # 冠：一团主冠 + 2~3 团副冠，用二级细分的二十面体（80 三角/团）⇒ 圆润不扎眼
    parts = [trunk]
    main_r = h * 0.26
    bpy.ops.mesh.primitive_ico_sphere_add(
        subdivisions=2, radius=main_r, location=(0, 0, trunk_h + main_r * 0.75)
    )
    main_canopy = bpy.context.object
    paint(main_canopy, LEAF_A)
    parts.append(main_canopy)

    for i, col in enumerate((LEAF_B, LEAF_C, LEAF_B)):
        ang = i * (2.0 * math.pi / 3.0) + rnd.uniform(-0.3, 0.3)
        rr = main_r * rnd.uniform(0.55, 0.70)
        dist = main_r * rnd.uniform(0.62, 0.78)
        bpy.ops.mesh.primitive_ico_sphere_add(
            subdivisions=2,
            radius=rr,
            location=(
                math.cos(ang) * dist,
                math.sin(ang) * dist,
                trunk_h + main_r * rnd.uniform(0.35, 0.95),
            ),
        )
        o = bpy.context.object
        paint(o, col)
        parts.append(o)

    for o in bpy.context.scene.objects:
        o.select_set(o.type == "MESH")
    bpy.context.view_layer.objects.active = trunk
    bpy.ops.object.join()
    obj = bpy.context.object
    obj.name = "tree_oak"
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)

    # 贴地 + 缩放到目标高度（先 apply 再在顶点空间缩放：顺序反了会把 location 加回来）
    me = obj.data
    zs = [v.co.z for v in me.vertices]
    k = h / max(1e-4, max(zs) - min(zs))
    zmin = min(zs)
    for v in me.vertices:
        v.co.z = (v.co.z - zmin) * k
        v.co.x *= k
        v.co.y *= k
    me.update()

    bpy.ops.export_scene.gltf(
        filepath=a.out, export_format="GLB", use_selection=True, export_apply=True,
        export_yup=True, export_normals=False, export_texcoords=False,
        export_vertex_color="NAME", export_vertex_color_name="Col", export_materials="NONE",
    )
    tris = sum(len(p.vertices) - 2 for p in me.polygons)
    print(f"[tree] {a.out}: {len(me.vertices)} 顶点 / {tris} 三角，高 {h:.3f} m")


main()
