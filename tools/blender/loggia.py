"""Blender 真建模：给建筑 GLB 切 loggia 凹龛 + 底层门斗（全部向内，包围盒不变）。

微几何（≤7 cm）在 30–60 m 视距读不出来（PROGRESS 续27）；0.8–0.9 m 的凹体量才读得出（续28）。
本轮：正面一个凹阳台龛、背面一个、底层一个门斗，全部用"删墙面补丁 + bmesh 补出龛体"。

用法： blender --background --python tools/blender/loggia.py -- in.glb out.glb
"""
import sys, bmesh, bpy

LOGGIA_W = 3.4
LOGGIA_H = 2.85
LOGGIA_D = 0.90
BACK_W = 3.0
BACK_D = 0.80
DOOR_W = 2.2
DOOR_D = 0.90
DOOR_H = 2.55


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

    # 窗台线（从几何反解）
    rows = set()
    for poly in me.polygons:
        if abs(poly.normal.z) > 0.25 or poly.area < 0.9 or poly.area > 3.0:
            continue
        if lum(face_avg(poly)) > 0.20:
            continue
        rows.add(round(min(me.vertices[vi].co.z for vi in poly.vertices), 2))
    rows = sorted(r for r in rows if z0 + 1.0 < r < z1 - 1.0)

    bm = bmesh.new()
    bm.from_mesh(me)
    clay = bm.loops.layers.color.get("Col") or bm.loops.layers.color.new("Col")
    panel = [c * 0.90 for c in wall]
    slab = [c * 0.80 for c in wall]
    rail = [c * 0.72 for c in wall]
    ceil = [c * 0.86 for c in wall]
    door_col = [c * 0.55 for c in wall]

    def add_quad(pts, col, expected_normal):
        vs = [bm.verts.new(p) for p in pts]
        f = bm.faces.new(vs)
        f.normal_update()
        if f.normal.dot(expected_normal) < 0.0:
            f.normal_flip()
        for loop in f.loops:
            loop[clay] = (col[0], col[1], col[2], 1.0)
        return f

    def add_box(center, size, col):
        ret = bmesh.ops.create_cube(bm, size=1.0)
        for v in ret["verts"]:
            v.co = (center[0] + v.co.x * size[0],
                    center[1] + v.co.y * size[1],
                    center[2] + v.co.z * size[2])
        for f in {f for v in ret["verts"] for f in v.link_faces}:
            for loop in f.loops:
                loop[clay] = (col[0], col[1], col[2], 1.0)

    from mathutils import Vector

    def recess(sign, width, zb, zt, depth, door=False):
        """sign=+1 在 +Y 面，-1 在 -Y 面。返回 (是否执行, 中心 x)。"""
        if zt - zb < 0.8 or zt > z1 - 0.05:
            return False
        outer = y1 if sign > 0 else y0
        inner = outer - sign * depth
        u0, u1 = -width * 0.5, width * 0.5
        dead = [f for f in bm.faces
                if u0 - 1e-4 <= f.calc_center_median().x <= u1 + 1e-4
                and zb - 1e-4 <= f.calc_center_median().z <= zt + 1e-4
                and min(inner, outer) - 1e-4 <= f.calc_center_median().y <= max(inner, outer) + 0.05]
        bmesh.ops.delete(bm, geom=dead, context="FACES")
        n_out = Vector((0.0, float(sign), 0.0))
        n_in = -n_out
        # 背板（朝向开口外侧）
        add_quad([(u0, inner, zb), (u1, inner, zb), (u1, inner, zt), (u0, inner, zt)], panel, n_out)
        # 侧壁（法线朝龛内）
        add_quad([(u0, outer, zb), (u0, inner, zb), (u0, inner, zt), (u0, outer, zt)], panel, Vector((1, 0, 0)) if True else n_in)
        add_quad([(u1, inner, zb), (u1, outer, zb), (u1, outer, zt), (u1, inner, zt)], panel, Vector((-1, 0, 0)))
        # 底板（+Z）/ 顶板（-Z）
        add_quad([(u0, outer, zb), (u1, outer, zb), (u1, inner, zb), (u0, inner, zb)], panel, Vector((0, 0, 1)))
        add_quad([(u0, inner, zt), (u1, inner, zt), (u1, outer, zt), (u0, outer, zt)], panel, Vector((0, 0, -1)))
        # 楼板 / 压顶 / 栏板（朝外一侧）
        add_box((0.0, (inner + outer) * 0.5, zb + 0.07),
                (width, depth - 0.04, 0.14), slab)
        add_box((0.0, (inner + outer) * 0.5, zt - 0.05),
                (width, depth - 0.04, 0.10), ceil)
        if not door:
            add_box((0.0, outer - sign * 0.045, zb + 0.14 + 0.46),
                    (width - 0.04, 0.09, 0.92), rail)
        else:
            add_box((0.0, inner + sign * 0.03, (zb + zt) * 0.5),
                    (width - 0.30, 0.06, zt - zb - 0.12), door_col)
        print(f"RECESS sign={sign:+d} w={width:.2f} z[{zb:.2f},{zt:.2f}] d={depth:.2f} door={door}")
        return True

    made = 0
    # 正面 loggia：取第二排窗台所在层
    if len(rows) > 1:
        zb = rows[1] + 0.10
        zt = min(zb + LOGGIA_H, z1 - 0.35)
        made += int(recess(+1, LOGGIA_W, zb, zt, LOGGIA_D))
    # 背面 loggia：再上一层（若有），否则同层
    back_row = rows[2] if len(rows) > 2 else (rows[1] if len(rows) > 1 else None)
    if back_row is not None:
        zb = back_row + 0.10
        zt = min(zb + LOGGIA_H, z1 - 0.35)
        made += int(recess(-1, BACK_W, zb, zt, BACK_D))
    # 底层门斗
    made += int(recess(+1, DOOR_W, 0.14, min(DOOR_H, z1 - 0.4), DOOR_D, door=True))

    if made:
        bm.to_mesh(me)
    else:
        print("LOGGIA skip: 无可用层位")
    bm.free()
    me.update()

    xs = [v.co.x for v in me.vertices]
    ys = [v.co.y for v in me.vertices]
    zs = [v.co.z for v in me.vertices]
    print(f"LOGGIA bbox {min(xs):.3f}..{max(xs):.3f} / {min(ys):.3f}..{max(ys):.3f} / "
          f"{min(zs):.3f}..{max(zs):.3f} verts={len(me.vertices)} recesses={made}")
    bpy.ops.export_scene.gltf(
        filepath=out, export_format="GLB", use_selection=True, export_yup=True,
        export_apply=True, export_normals=True, export_texcoords=True,
        export_vertex_color="NAME", export_vertex_color_name="Col",
        export_materials="NONE", export_extras=False, export_animations=False,
        export_morph=False, export_skins=False,
    )


main()
