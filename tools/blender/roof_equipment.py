"""Blender 真建模：屋面设备（全部在女儿墙高度以内 + 包围盒内）。

从高处/楼上窗户看过去，平屋顶现在是一块空板。加：楼梯出屋面（penthouse）、
水箱（带四条支腿）、两根通风管。顶面一律 <= z1-0.05，绝不越契约高度。

用法： blender --background --python tools/blender/roof_equipment.py -- in.glb out.glb
"""
import sys, bmesh, bpy


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    src, out = argv[0], argv[1]
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=src)
    ob = [o for o in bpy.context.scene.objects if o.type == "MESH"][0]
    me = ob.data
    for a in list(me.color_attributes):
        if a.name != "Col":
            a.name = "Col"
    ca = me.color_attributes[0]

    def avg(poly):
        cs = [tuple(ca.data[li].color)[:3] for li in poly.loop_indices]
        n = max(1, len(cs))
        return [sum(c[i] for c in cs) / n for i in range(3)]

    def lum(c):
        return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]

    xs = [v.co.x for v in me.vertices]; ys = [v.co.y for v in me.vertices]; zs = [v.co.z for v in me.vertices]
    x0, x1, y0, y1, z0, z1 = min(xs), max(xs), min(ys), max(ys), min(zs), max(zs)
    wall = [0.55, 0.55, 0.52]
    best = -1.0
    for poly in me.polygons:
        c = avg(poly)
        if lum(c) > best:
            best, wall = lum(c), c
    metal = [c * 0.55 for c in wall]
    concrete = [c * 0.85 for c in wall]
    vent = [c * 0.70 for c in wall]

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

    margin = 0.55
    roof_top = z1 - 0.05
    # 楼梯出屋面：偏后侧，2.4 x 2.0 x 0.85
    pw, pd, ph = 2.4, 2.0, 0.85
    cx = x1 - margin - pw * 0.5 - 0.4
    cy = y0 + margin + pd * 0.5 + 0.3
    add_box((cx, cy, roof_top - ph * 0.5), (pw, pd, ph), concrete)
    # 水箱 + 四条支腿：1.3 x 1.3 x 0.55，靠另一侧
    tx = x0 + margin + 1.1
    ty = y1 - margin - 1.1
    tank_top = roof_top
    add_box((tx, ty, tank_top - 0.30), (1.3, 1.3, 0.55), metal)
    for sx in (-1.0, 1.0):
        for sy in (-1.0, 1.0):
            add_box((tx + sx * 0.45, ty + sy * 0.45, tank_top - 0.72), (0.12, 0.12, 0.30), metal)
    # 两根通风管
    for i, (vx, vy) in enumerate(((x0 + margin + 0.8, y0 + margin + 0.9),
                                  (x0 + margin + 1.6, y0 + margin + 0.9))):
        add_box((vx, vy, roof_top - 0.30), (0.26, 0.26, 0.60), vent)
        add_box((vx, vy, roof_top - 0.02), (0.36, 0.36, 0.06), vent)

    bm.to_mesh(me)
    bm.free()
    me.update()
    xs = [v.co.x for v in me.vertices]; ys = [v.co.y for v in me.vertices]; zs = [v.co.z for v in me.vertices]
    print(f"ROOF bbox {min(xs):.3f}..{max(xs):.3f} / {min(ys):.3f}..{max(ys):.3f} / "
          f"{min(zs):.3f}..{max(zs):.3f} verts={len(me.vertices)}")
    assert max(zs) <= z1 + 1e-4, "屋面设备越过了契约高度"
    assert min(xs) >= x0 - 1e-4 and max(xs) <= x1 + 1e-4, "屋面设备越界 x"
    assert min(ys) >= y0 - 1e-4 and max(ys) <= y1 + 1e-4, "屋面设备越界 y"
    bpy.ops.export_scene.gltf(
        filepath=out, export_format="GLB", use_selection=True, export_yup=True,
        export_apply=True, export_normals=True, export_texcoords=True,
        export_vertex_color="NAME", export_vertex_color_name="Col",
        export_materials="NONE", export_extras=False, export_animations=False,
        export_morph=False, export_skins=False,
    )


main()
