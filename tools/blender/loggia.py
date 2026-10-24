"""Blender 真建模：给建筑 GLB 切一个 loggia（凹阳台）凹龛。

微几何（≤7 cm，合同上限）在 30–60 m 视距读不出来（PROGRESS 续27）。凹龛是**向内**的
0.9 m 体量：不破坏契约包围盒，却能产生真实阴影与轮廓。

流程：import GLB → 删掉洞口处墙面补丁 → bmesh 补出龛体（背板/侧壁/底板/顶板）→
加楼板/压顶/栏板 → 导出同约定 GLB。
用法： blender --background --python tools/blender/loggia.py -- in.glb out.glb
"""
import sys, bmesh, bpy

LOGGIA_W = 3.4
LOGGIA_H = 2.85
LOGGIA_D = 0.90


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    src, out = argv[0], argv[1]
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=src)
    obs = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    assert len(obs) == 1, f"期望单 mesh，实际 {len(obs)}"
    ob = obs[0]
    me = ob.data
    for a in list(me.color_attributes):
        if a.name != "Col":
            a.name = "Col"
    ca = me.color_attributes[0]

    def face_avg(poly):
        cs = [tuple(ca.data[li].color)[:3] for li in poly.loop_indices]
        n = max(1, len(cs))
        return [sum(c[i] for c in cs) / n for i in range(3)]

    def lum(c):
        return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]

    xs = [v.co.x for v in me.vertices]
    ys = [v.co.y for v in me.vertices]
    zs = [v.co.z for v in me.vertices]
    x0, x1, y0, y1, z0, z1 = min(xs), max(xs), min(ys), max(ys), min(zs), max(zs)
    wall = [0.55, 0.55, 0.52]
    best = -1.0
    for poly in me.polygons:
        avg = face_avg(poly)
        if lum(avg) > best:
            best, wall = lum(avg), avg

    # 从几何反解窗台线：取"从下往上第二排"，并夹在屋顶以内（shed 只有 2 层）。
    rows = set()
    for poly in me.polygons:
        if abs(poly.normal.z) > 0.25 or poly.area < 0.9 or poly.area > 3.0:
            continue
        if lum(face_avg(poly)) > 0.20:
            continue
        rows.add(round(min(me.vertices[vi].co.z for vi in poly.vertices), 2))
    rows = sorted(r for r in rows if z0 + 1.0 < r < z1 - 1.0)
    cz0 = cz1 = None
    if len(rows) > 1:
        cz0 = rows[1] + 0.10
        cz1 = min(cz0 + LOGGIA_H, z1 - 0.35)
    if cz0 is None or cz1 - cz0 < 2.0:
        print("LOGGIA skip: 无可用层位")
    else:
        cx0, cx1 = -LOGGIA_W * 0.5, LOGGIA_W * 0.5
        cy_in = y1 - LOGGIA_D
        print(f"LOGGIA box x[{cx0:.2f},{cx1:.2f}] y[{cy_in:.2f},{y1:.2f}] z[{cz0:.2f},{cz1:.2f}]")

        # 1) 开洞：删掉洞口范围内的墙面（薄壳网格用 Boolean 会留切刀外皮）
        bm = bmesh.new()
        bm.from_mesh(me)
        dead = [f for f in bm.faces
                if cx0 - 1e-4 <= f.calc_center_median().x <= cx1 + 1e-4
                and cz0 - 1e-4 <= f.calc_center_median().z <= cz1 + 1e-4
                and f.calc_center_median().y > cy_in - 1e-4]
        bmesh.ops.delete(bm, geom=dead, context="FACES")
        clay = bm.loops.layers.color.get("Col") or bm.loops.layers.color.new("Col")
        panel = [c * 0.90 for c in wall]

        def add_quad(pts, col):
            vs = [bm.verts.new(p) for p in pts]
            f = bm.faces.new(vs)
            f.normal_update()
            for loop in f.loops:
                loop[clay] = (col[0], col[1], col[2], 1.0)
            return f

        # 龛体六面（朝向按"看到里面"的顺序给）
        add_quad([(cx0, cy_in, cz0), (cx1, cy_in, cz0), (cx1, cy_in, cz1), (cx0, cy_in, cz1)], panel)
        add_quad([(cx0, y1, cz0), (cx0, cy_in, cz0), (cx0, cy_in, cz1), (cx0, y1, cz1)], panel)
        add_quad([(cx1, cy_in, cz0), (cx1, y1, cz0), (cx1, y1, cz1), (cx1, cy_in, cz1)], panel)
        add_quad([(cx0, y1, cz0), (cx1, y1, cz0), (cx1, cy_in, cz0), (cx0, cy_in, cz0)], panel)
        add_quad([(cx0, cy_in, cz1), (cx1, cy_in, cz1), (cx1, y1, cz1), (cx0, y1, cz1)], panel)

        def add_box(center, size, col):
            ret = bmesh.ops.create_cube(bm, size=1.0)
            for v in ret["verts"]:
                v.co = (center[0] + v.co.x * size[0],
                        center[1] + v.co.y * size[1],
                        center[2] + v.co.z * size[2])
            for f in {f for v in ret["verts"] for f in v.link_faces}:
                for loop in f.loops:
                    loop[clay] = (col[0], col[1], col[2], 1.0)

        slab = [c * 0.80 for c in wall]
        rail = [c * 0.72 for c in wall]
        ceil = [c * 0.86 for c in wall]
        add_box(((cx0 + cx1) * 0.5, (cy_in + y1) * 0.5, cz0 + 0.07),
                (LOGGIA_W, LOGGIA_D - 0.04, 0.14), slab)
        add_box(((cx0 + cx1) * 0.5, (cy_in + y1) * 0.5, cz1 - 0.05),
                (LOGGIA_W, LOGGIA_D - 0.04, 0.10), ceil)
        add_box(((cx0 + cx1) * 0.5, y1 - 0.045, cz0 + 0.14 + 0.46),
                (LOGGIA_W - 0.04, 0.09, 0.92), rail)
        bm.to_mesh(me)
        bm.free()
        me.update()

    xs = [v.co.x for v in me.vertices]
    ys = [v.co.y for v in me.vertices]
    zs = [v.co.z for v in me.vertices]
    print(f"LOGGIA bbox {min(xs):.3f}..{max(xs):.3f} / {min(ys):.3f}..{max(ys):.3f} / "
          f"{min(zs):.3f}..{max(zs):.3f} verts={len(me.vertices)}")
    bpy.ops.export_scene.gltf(
        filepath=out, export_format="GLB", use_selection=True, export_yup=True,
        export_apply=True, export_normals=True, export_texcoords=True,
        export_vertex_color="NAME", export_vertex_color_name="Col",
        export_materials="NONE", export_extras=False, export_animations=False,
        export_morph=False, export_skins=False,
    )


main()
