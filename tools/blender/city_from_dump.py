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
    ap.add_argument("--cell", type=float, default=0.0,
                    help="按边长为 cell 米的方格**分批导出**（0=整城一个文件）")
    ap.add_argument("--out-dir", default="", help="--cell 模式下的输出目录")
    # 诊断用（2026-11-03）：把范围压到一个世界坐标盒里，逐件打印行号 —— 用来给"屏幕上某块东西"
    # 指名。宽度为 0 表示不过滤。
    ap.add_argument("--only-box", default="", help="x0,z0,x1,z1（只导出该盒内的构件）")
    ap.add_argument("--debug-pieces", action="store_true", help="逐件打印 行号/kind/尺寸/位置")
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

    box = None
    if a.only_box:
        try:
            x0, z0, x1, z1 = (float(v) for v in a.only_box.split(","))
            box = (min(x0, x1), min(z0, z1), max(x0, x1), max(z0, z1))
        except ValueError:
            print(f"  --only-box 解析失败：{a.only_box}")

    made = 0
    for row_no, r in enumerate(rows, start=2):
        try:
            x, y, z = float(r["x"]), float(r["y"]), float(r["z"])
            hw, hd, hh = float(r["half_w"]), float(r["half_d"]), float(r["half_h"])
            col = (float(r["r"]), float(r["g"]), float(r["b"]))
        except (KeyError, ValueError):
            continue
        if hw <= 0.0 or hd <= 0.0 or hh <= 0.0:
            continue
        if box is not None and not (box[0] <= x <= box[2] and box[1] <= z <= box[3]):
            continue
        # 🔴 **建筑一律不进 chunk**（2026-11-01）：楼体已经由 `assets/props/building_*.glb`
        # 画出来了，chunk 里再来一个同位置灰盒就是共面重复（z-fighting / 灰壳盖住棕色立面）。
        # 判据：导出日志里的 kind 计数应满足 building=0。
        # 🔴 **只排"高"的 Building**（2026-11-05）：楼体由 building_*.glb 画，chunk 再来一个
        # 同位置灰盒会盖住立面。但**扁平的 Building 是台面/水面**（喷泉石台 9×9×0.22、
        # 水面 8.2×8.2×0.30），它们没有对应的 GLB —— 一律排掉的话，喷泉就只剩中间那根柱子
        # 孤零零立在广场中央（玩家报的"路中那三个东西一直没去掉"，其实剩下的正是喷泉柱）。
        if r["kind"] == "Building" and hh * 2.0 > 0.5:
            continue
        # 🔴 **树/灌木也不算 chunk**（2026-11-01 玩家截图：树干"下半截变浅粉色"）：
        # 那是程序化的树干圆柱/灌木球，与 `tree_oak.glb` / `bush_real.glb` 叠在同一位置，
        # 一个受光、一个走平面着色 ⇒ 看起来像"整棵树被一条线切成两色"。
        # 可见的植被已由 GLB 提供，导出时排除 kind=Tree 与所有球体（灌木/花坛球）。
        if r["kind"] == "Tree":
            continue
        if r["shape"] == "Sphere":
            continue
        # 🔴 **离地 4m 以上的装饰件一律不进 chunk**（2026-11-01 玩家截图：天空里漂着灰方块）。
        # 它们（檐口/窗带/女儿墙）是挂在建筑盒上的，而建筑盒已由 GLB 画出来、marker 路径也
        # 早就跳过它们（`lib.rs` 的 4m 规则）—— 导出时漏了这条，于是整片挂件飘在半空。
        if y - hh > 4.0:
            continue
        # 🔴 **薄而离地的构件不进 chunk**（2026-11-03 逐块隐藏定案）：
        # 实测悬在半空的是围墙的**压顶带**（77.8m × 1.6m × 0.2m、离地 2.35m）——
        # 墙身是 `Building`（已被排除），压顶带单独留下就成了"天空里的米色大薄板"。
        # 判据：厚度 ≤ 0.35m 且底面离地 ≥ 1.0m ⇒ 一律视为立面附件，交给建筑 GLB。
        if hh * 2.0 <= 0.35 and (y - hh) >= 1.0:
            continue
        shape = r["shape"]
        if shape == "Cylinder":
            # 引擎的圆柱模板：r=1、y∈[-0.5,0.5]、实例缩放 = 半径(xz) / 半高(y)
            # 🔴 **必须把圆柱转成"轴朝上"**（2026-11-04 定位到的大 bug）：
            # Blender 的 `primitive_cylinder_add` 圆柱轴是 **Z**（Blender 的 up），而我们把
            # 世界坐标直接塞进 (x, z, y) ⇒ 轴指向世界 Z，柱廊柱就"躺"成一根 4.65m 长的横筒，
            # 屏幕上读作斜着横过画面的大板。绕 X 转 90° 让轴对上世界 Y，缩放语义才成立：
            # 半径 → x/z（hw / hd），半高 → y（hh）。
            bpy.ops.mesh.primitive_cylinder_add(
                vertices=SEG, radius=1.0, depth=2.0,
                location=(x, z, y), rotation=(math.pi * 0.5, 0.0, 0.0),
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
        if a.debug_pieces:
            print(
                f"[piece] tsv行={row_no} kind={r['kind']} shape={shape} "
                f"底={y - hh:.2f} 高={hh * 2:.2f} 尺寸={hw * 2:.2f}x{hd * 2:.2f} @({x:.1f},{z:.1f})"
            )
        if made % 300 == 0:
            print(f"   … 已建 {made} 件")

    if a.cell > 0.0:
        # 按 40m 格分批：每格内的构件合并成一个 chunk 导出（引擎侧按 chunk 当大件绘制）。
        # 🔴 分格必须在**建几何之后、合并之前**做：按物体的世界坐标中心归格。
        out_dir = a.out_dir or "."
        buckets = {}
        for o in bpy.context.scene.objects:
            if o.type != "MESH":
                continue
            c = o.matrix_world.translation
            key = (math.floor(c.x / a.cell), math.floor(c.z / a.cell))
            buckets.setdefault(key, []).append(o)
        print(f"  分格：{len(buckets)} 个 chunk（边长 {a.cell} m）")
        total_v = total_t = 0
        for (cx, cz), objs in sorted(buckets.items()):
            for o in bpy.context.scene.objects:
                o.select_set(False)
            for o in objs:
                o.select_set(True)
            bpy.context.view_layer.objects.active = objs[0]
            bpy.ops.object.join()
            obj = bpy.context.object
            bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
            me = obj.data
            tris = sum(len(p.vertices) - 2 for p in me.polygons)
            path = f"{out_dir}/city_cell_{cx}_{cz}.glb"
            bpy.ops.export_scene.gltf(
                filepath=path, export_format="GLB", use_selection=True, export_apply=True,
                export_yup=True, export_normals=False, export_texcoords=False,
                export_vertex_color="NAME", export_vertex_color_name="Col",
                export_materials="NONE",
            )
            total_v += len(me.vertices)
            total_t += tris
            print(f"[city] {path}: {len(me.vertices)} 顶点 / {tris} 三角（{len(objs)} 件）")
        print(f"[city] 合计 {len(buckets)} chunk / {total_v} 顶点 / {total_t} 三角")
        return

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
