"""把 OpenGameArt 的 CC0 单个沙袋（OBJ）**阵列成沙袋墙**并导出 GLB。

来源：<https://opengameart.org/content/simple-sandbags>（CC0，`sandbag_model.obj` + `sandbag_diffuse.png`）。
为什么阵列而不是自己建模：素材是**下载来的**，这里只做实例摆放（3 排交错码放），
和引擎里"一件道具 = 一个 GLB"的约定对齐 —— 一堵墙必须是单个 mesh。

契约：单 mesh / POSITION + 顶点色 / 原点在底面中心 / 1 单位=1 米 / 不导出法线。
顶点色来自贴图 UV 采样（引擎道具不采样贴图，只能烘进顶点色）。

用法：
  blender -b --python tools/blender/make_sandbag_wall.py -- <in.obj> <in_diffuse.png> <out.glb>
      [--length 2.4] [--height 0.9] [--depth 0.7]
"""

import argparse
import math
import sys

import bpy

COLS = 6      # 每排袋数（默认；可用 --cols 覆盖）
ROWS = 3      # 排数（默认；可用 --rows 覆盖）


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    ap = argparse.ArgumentParser()
    ap.add_argument("obj")
    ap.add_argument("diffuse")
    ap.add_argument("out")
    ap.add_argument("--length", type=float, default=2.4)
    ap.add_argument("--height", type=float, default=0.9)
    ap.add_argument("--depth", type=float, default=0.7)
    ap.add_argument("--cols", type=int, default=COLS)
    ap.add_argument("--rows", type=int, default=ROWS)
    return ap.parse_args(argv)


def main():
    a = args()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.wm.obj_import(filepath=a.obj)

    src = [o for o in bpy.context.scene.objects if o.type == "MESH"][0]
    bpy.context.view_layer.objects.active = src

    # 单个沙袋的尺寸（OBJ 头写"centimeters"，但实际跨度未必；这里按包围盒归一化后再说）
    bb = [src.matrix_world @ v.co for v in src.data.vertices]
    sx = max(v.x for v in bb) - min(v.x for v in bb)
    sy = max(v.y for v in bb) - min(v.y for v in bb)
    sz = max(v.z for v in bb) - min(v.z for v in bb)

    cols = max(1, a.cols)
    rows = max(1, a.rows)
    # 目标：整堵墙 length × depth × height，每袋 = 墙长/cols × 墙深 × 墙高/rows（略放大做压接）
    bag_x = a.length / cols * 1.06
    bag_y = a.depth
    bag_z = a.height / rows * 1.10
    kx, ky, kz = bag_x / max(sx, 1e-6), bag_y / max(sy, 1e-6), bag_z / max(sz, 1e-6)

    clones = []
    for row in range(rows):
        stagger = 0.5 if row % 2 and cols > 1 else 0.0
        for col in range(cols):
            dup = src.copy()
            dup.data = src.data.copy()
            bpy.context.scene.collection.objects.link(dup)
            px = (-a.length * 0.5) + bag_x * (col + 0.5 + stagger)
            if px > a.length * 0.5:
                px -= a.length  # 交错排的越界袋折回另一侧，避免墙头空一截
            dup.location = (px, 0.0, bag_z * (row + 0.5))
            dup.scale = (kx, ky, kz)
            clones.append(dup)
    src.select_set(False)

    for o in clones:
        o.select_set(True)
    bpy.context.view_layer.objects.active = clones[0]
    bpy.ops.object.join()
    obj = bpy.context.object
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)

    # 贴图颜色 → 顶点色（引擎道具只吃顶点色）
    me = obj.data
    img = bpy.data.images.load(a.diffuse, check_existing=True)
    w, h = img.size
    px = list(img.pixels)
    attr = me.color_attributes.get("Col")
    if attr is None:
        attr = me.color_attributes.new(name="Col", type="FLOAT_COLOR", domain="CORNER")
    uv = me.uv_layers.active.data if me.uv_layers else None
    if uv is None:
        for d in attr.data:
            d.color = (0.62, 0.56, 0.42, 1.0)
    else:
        for poly in me.polygons:
            for li in poly.loop_indices:
                u = uv[li].uv.x % 1.0
                v = uv[li].uv.y % 1.0
                ix = min(w - 1, max(0, int(u * w)))
                iy = min(h - 1, max(0, int(v * h)))
                i = (iy * w + ix) * 4
                attr.data[li].color = (px[i], px[i + 1], px[i + 2], 1.0)
    me.color_attributes.active_color = attr

    # 贴地 + **逐轴**缩放到目标外形（阵列后 x/y 会因交错排变宽，必须逐轴校正）
    xs = [v.co.x for v in me.vertices]
    ys = [v.co.y for v in me.vertices]
    zs = [v.co.z for v in me.vertices]
    kx2 = a.length / max(1e-4, max(xs) - min(xs))
    ky2 = a.depth / max(1e-4, max(ys) - min(ys))
    kz2 = a.height / max(1e-4, max(zs) - min(zs))
    cx = (max(xs) + min(xs)) * 0.5
    cy = (max(ys) + min(ys)) * 0.5
    zmin = min(zs)
    for v in me.vertices:
        v.co.x = (v.co.x - cx) * kx2
        v.co.y = (v.co.y - cy) * ky2
        v.co.z = (v.co.z - zmin) * kz2
    me.update()

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
    tris = sum(len(p.vertices) - 2 for p in me.polygons)
    print(f"[sandbag] {a.out}: {len(me.vertices)} 顶点 / {tris} 三角，{cols}x{rows} 件")


main()
