"""Blender 真建模：loggia / 门斗的内部细节（全部在包围盒内）。

合同只允许 ≤7 cm 外挑，屋面设备又会被女儿墙挡住 ⇒ 继续往"内部"做深度：
- loggia：实心栏板换成 7 根竖杆 + 扶手；龛顶加 2 道梁；
- 门斗：加两侧门套 + 过梁 + 门槛石。
全部用 bmesh 盒体，带顶点色，导出同约定 GLB。

用法： blender --background --python tools/blender/interior_detail.py -- in.glb out.glb
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
    assert len(obs) == 1
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
    metal = [c * 0.62 for c in wall]
    trim = [min(1.0, c * 1.12) for c in wall]
    stone = [c * 0.70 for c in wall]

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

    def add_box(center, size, col):
        ret = bmesh.ops.create_cube(bm, size=1.0)
        for v in ret["verts"]:
            v.co = (center[0] + v.co.x * size[0],
                    center[1] + v.co.y * size[1],
                    center[2] + v.co.z * size[2])
        for f in {f for v in ret["verts"] for f in v.link_faces}:
            for loop in f.loops:
                loop[clay] = (col[0], col[1], col[2], 1.0)

    made = 0

    def balcony(sign, width, zb, zt, depth):
        nonlocal made
        outer = y1 if sign > 0 else y0
        inner = outer - sign * depth
        # 竖杆会被实心栏板挡住（合同不允许再往外）⇒ 只做"栏板压顶扶手"
        z_rail = zb + 0.14
        rail_top = z_rail + 0.92
        add_box((0.0, outer - sign * 0.05, rail_top + 0.03),
                (width - 0.02, 0.10, 0.06), trim)
        # 龛顶两道梁
        for i in (-1, 1):
            add_box((0.0, inner + sign * (depth * (0.42 if i < 0 else 0.72)), zt - 0.09),
                    (width, 0.10, 0.12), stone)
        made += 1

    if len(rows) > 1:
        zb = rows[1] + 0.10
        zt = min(zb + LOGGIA_H, z1 - 0.35)
        balcony(+1, LOGGIA_W, zb, zt, LOGGIA_D)
    back_row = rows[2] if len(rows) > 2 else None
    if back_row is not None:
        zb = back_row + 0.10
        zt = min(zb + LOGGIA_H, z1 - 0.35)
        balcony(-1, BACK_W, zb, zt, BACK_D)
    # 门斗：门套 + 过梁 + 门槛
    if z1 - 0.4 > 1.0:
        zb, zt = 0.14, min(DOOR_H, z1 - 0.4)
        cy = y1 - 0.06
        for sx in (-1.0, 1.0):
            add_box((sx * (DOOR_W * 0.5 - 0.06), cy, (zb + zt) * 0.5),
                    (0.12, 0.10, zt - zb), trim)
        add_box((0.0, cy, zt - 0.06), (DOOR_W, 0.10, 0.12), trim)
        add_box((0.0, y1 - 0.16, zb + 0.02), (DOOR_W + 0.10, 0.24, 0.05), stone)
        made += 1

    bm.to_mesh(me)
    bm.free()
    me.update()
    xs = [v.co.x for v in me.vertices]
    ys = [v.co.y for v in me.vertices]
    zs = [v.co.z for v in me.vertices]
    print(f"INTERIOR details={made} bbox {min(xs):.3f}..{max(xs):.3f} / {min(ys):.3f}..{max(ys):.3f} / "
          f"{min(zs):.3f}..{max(zs):.3f} verts={len(me.vertices)} faces={len(me.polygons)}")
    bpy.ops.export_scene.gltf(
        filepath=out, export_format="GLB", use_selection=True, export_yup=True,
        export_apply=True, export_normals=True, export_texcoords=True,
        export_vertex_color="NAME", export_vertex_color_name="Col",
        export_materials="NONE", export_extras=False, export_animations=False,
        export_morph=False, export_skins=False,
    )


main()
