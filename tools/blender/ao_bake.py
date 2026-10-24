"""Blender BVH 接触遮蔽烘焙：把 AO 乘进已有顶点色（零三角形）。

为什么：道具没有 NORMAL、没有贴图（UV 常量）⇒ 顶点色是唯一着色通道，而现有 AO 只随
高度变化（一维）⇒ 同一面墙四角同色、构件相交处没有接触暗带。

做法：BVHTree.FromPolygons 建自身 BVH → 按**位置**分组（关键：`weld_props` 按 pos+color
去重，同一位置必须拿到同一颜色，否则焊接失效、顶点数暴涨）→ 合并法线半球 12 方向 ×
3 个步长（0.05/0.15/0.40 m）共 36 条射线 → ao = 1 - 0.45 * hits/36 → 乘进顶点色。

用法： blender --background --python tools/blender/ao_bake.py -- in.glb out.glb [strength]
"""
import sys, math, bmesh, bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

GOLDEN = math.pi * (3.0 - math.sqrt(5.0))


def hemi_dirs(normal, n=12):
    helper = Vector((0.0, 0.0, 1.0)) if abs(normal.z) < 0.9 else Vector((1.0, 0.0, 0.0))
    t = normal.cross(helper).normalized()
    b = normal.cross(t)
    out = []
    for i in range(n):
        z = 1.0 - (i + 0.5) / n
        r = math.sqrt(max(0.0, 1.0 - z * z))
        phi = i * GOLDEN
        out.append((t * (r * math.cos(phi)) + b * (r * math.sin(phi)) + normal * z).normalized())
    return out


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    src, out = argv[0], argv[1]
    strength = float(argv[2]) if len(argv) > 2 else 0.5
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=src)
    ob = [o for o in bpy.context.scene.objects if o.type == "MESH"][0]
    me = ob.data
    for a in list(me.color_attributes):
        if a.name != "Col":
            a.name = "Col"
    ca = me.color_attributes[0]
    v0 = len(me.vertices)
    xs = [v.co.x for v in me.vertices]; ys = [v.co.y for v in me.vertices]; zs = [v.co.z for v in me.vertices]
    bbox0 = (min(xs), max(xs), min(ys), max(ys), min(zs), max(zs))

    verts = [v.co.copy() for v in me.vertices]
    polys = [tuple(p.vertices) for p in me.polygons]
    tree = BVHTree.FromPolygons(verts, polys, all_triangles=False)

    # 按位置分组（1e-4 m 网格）：同一位置共享一个 AO 值。
    groups = {}
    for i, co in enumerate(verts):
        key = (round(co.x, 4), round(co.y, 4), round(co.z, 4))
        groups.setdefault(key, []).append(i)
    # 合并法线（面法线按面积加权累加）
    normals = [Vector((0.0, 0.0, 0.0)) for _ in verts]
    for p in me.polygons:
        w = p.area if p.area > 1e-9 else 1.0
        for vi in p.vertices:
            normals[vi] += p.normal * w

    # 逐位置的"被遮挡比例"（0..1）。
    occl_by_group = {}
    for key, idxs in groups.items():
        n = Vector((0.0, 0.0, 0.0))
        for i in idxs:
            n += normals[i]
        n = n.normalized() if n.length > 1e-9 else Vector((0.0, 0.0, 1.0))
        co = verts[idxs[0]]
        hits = 0
        origin = co + n * 1e-4
        for d in hemi_dirs(n):
            for dist in (0.05, 0.15, 0.30):
                loc, _nor, _idx, _d = tree.ray_cast(origin, d, dist)
                if loc is not None:
                    hits += 1
        occl_by_group[key] = hits / 36.0

    # 🔴 以"中位遮挡"为参考：只压比中位更闭塞的位置（棱角/龛内/檐下），
    # 大面积未接触表面保持 1.0 —— 否则整面墙被均匀压暗，变成"整栋变脏"而不是接触暗带。
    vals = sorted(occl_by_group.values())
    ref = vals[int(len(vals) * 0.75)] if vals else 0.0
    ao_by_vert = {}
    for key, idxs in groups.items():
        ao = 1.0 - strength * max(0.0, occl_by_group[key] - ref)
        ao = max(0.55, min(1.0, ao))
        for i in idxs:
            ao_by_vert[i] = ao

    # 写回循环颜色（CORNER 域）；同一位置的循环颜色一致 ⇒ 焊接行为不变。
    for loop in me.loops:
        ao = ao_by_vert.get(loop.vertex_index, 1.0)
        c = ca.data[loop.index].color
        ca.data[loop.index].color = (c[0] * ao, c[1] * ao, c[2] * ao, c[3])

    xs = [v.co.x for v in me.vertices]; ys = [v.co.y for v in me.vertices]; zs = [v.co.z for v in me.vertices]
    bbox1 = (min(xs), max(xs), min(ys), max(ys), min(zs), max(zs))
    assert all(abs(a - b) < 1e-4 for a, b in zip(bbox0, bbox1)), "AO 烘焙改了包围盒"
    print(f"AO_BAKE src={src.split('/')[-1]} verts={v0} positions={len(groups)} "
          f"ao_min={min(ao_by_vert.values()):.3f} ao_mean={sum(ao_by_vert.values())/len(ao_by_vert):.3f}")
    bpy.ops.export_scene.gltf(
        filepath=out, export_format="GLB", use_selection=True, export_yup=True,
        export_apply=True, export_normals=True, export_texcoords=True,
        export_vertex_color="NAME", export_vertex_color_name="Col",
        export_materials="NONE", export_extras=False, export_animations=False,
        export_morph=False, export_skins=False,
    )


main()
