"""从引擎的布局导出（`RV3D_DUMP_ALL=<path>` 产出的 TSV）在 Blender 里**重建整座城**。

为什么这么做（2026-11-01）：全城是 **1230 个程序化构件**（box/cylinder/sphere 堆出来的路缘、
柱廊、花坛、护柱…），逐实例绘制既贵又难看。路线是：布局仍由 `city.rs` 唯一决定（碰撞、
寻路、40+ 条布局判据全部不动），但**可见几何改由 Blender 建模并合并导出** —— 于是同一条街
从"几百个薄壳盒子"变成少量合并网格，也才有地方把路缘/柱头/树冠做成真模型。

用法：
  blender -b --python tools/blender/city_from_dump.py -- <in.tsv> <out.glb> [--limit N]

⚠️ 本脚本只做**粗块重建**（按 kind/shape 摆盒子/圆柱/球），目的是拿到"可编辑的整城"，
不是最终美术。真正的细节要在 Blender 里对具体构件重塑。
"""

import argparse
import math
import sys

import bpy

SEG = 12  # 圆柱分段（骨架够用；最终美术会在 Blender 里重塑）


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    ap = argparse.ArgumentParser()
    ap.add_argument("tsv")
    ap.add_argument("out")
    ap.add_argument("--limit", type=int, default=0, help="只重建前 N 件（调试用）")
    return ap.parse_args(argv)


def paint(obj, rgb):
    me = obj.data
    attr = me.color_attributes.new(name="Col", type="FLOAT_COLOR", domain="CORNER")
    for poly in me.polygons:
        for li in poly.loop_indices:
            attr.data[li].color = (rgb[0], rgb[1], rgb[2], 1.0)
    me.color_attributes.active_color = attr


def main():
    a = args()
    bpy.ops.wm.read_factory_settings(use_empty=True)

    rows = []
    with open(a.tsv, encoding="utf-8") as f:
        hdr = f.readline().rstrip("\n").split("\t")
        for line in f:
            v = line.rstrip("\n").split("\t")
            if len(v) < len(hdr):
                continue
            rows.append(dict(zip(hdr, v)))
    if a.limit:
        rows = rows[: a.limit]
    print(f"  读入构件 {len(rows)} 件")

    made = 0
    for r in rows:
        try:
            x, y, z = float(r["x"]), float(r["y"]), float(r["z"])
            hw, hd, hh = float(r["half_w"]), float(r["half_d"]), float(r["half_h"])
            col = (float(r["r"]), float(r["g"]), float(r["b"]))
        except (KeyError, ValueError):
            continue
        if hw <= 0.0 or hd <= 0.0 or hh <= 0.0:
            continue
        shape = r["shape"]
        if shape == "Cylinder":
            # 引擎的圆柱模板：r=1、y∈[-0.5,0.5]、实例缩放 = 半径(xz) / 半高(y)
            bpy.ops.mesh.primitive_cylinder_add(
                vertices=SEG, radius=1.0, depth=2.0, location=(x, z, y)
            )
            o = bpy.context.object
            o.scale = (hw, hh, hd)
        elif shape == "Sphere":
            bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1, radius=1.0, location=(x, z, y))
            o = bpy.context.object
            o.scale = (hw, hh, hd)
        else:  # Legacy / Authored / 其它都按盒子
            bpy.ops.mesh.primitive_cube_add(size=2.0, location=(x, z, y))
            o = bpy.context.object
            o.scale = (hw, hd, hh)
        paint(o, col)
        made += 1
        if made % 300 == 0:
            print(f"   … 已建 {made} 件")

    objs = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.join()
    obj = bpy.context.object
    obj.name = "city_blockout"
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)

    me = obj.data
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
    print(f"[city] {a.out}: {len(me.vertices)} 顶点 / {tris} 三角，来自 {made} 个构件")


main()
