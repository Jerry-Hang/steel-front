"""Blender 真建模：给窗洞做"封板 / 破窗"变体（确定性抽签，按位置哈希）。

现状：一栋楼 16 个窗洞完全相同 ⇒ 立面像壁纸。按 `crc32(位置)` 抽：
  · 封板（约 18%）：洞内 6 cm 处加一块轻质木板（暖色），带 2 cm 缝；
  · 破窗（约 12%）：底板压暗到 0.55×，底边留 3 片深色碎玻璃。
全部在包围盒内，三角形 +0～30/洞。

用法： blender --background --python tools/blender/window_variants.py -- in.glb out.glb
"""
import sys, zlib, bmesh, bpy


def pick(centroid, salt):
    h = zlib.crc32(
        f"{centroid[0]:.2f},{centroid[1]:.2f},{centroid[2]:.2f},{salt}".encode()
    ) & 0xFFFF
    return h / 65535.0


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

    xs = [v.co.x for v in me.vertices]
    ys = [v.co.y for v in me.vertices]
    zs = [v.co.z for v in me.vertices]
    bbox = (min(xs), max(xs), min(ys), max(ys), min(zs), max(zs))
    wall = [0.55, 0.55, 0.52]
    best = -1.0
    for poly in me.polygons:
        c = avg(poly)
        if lum(c) > best:
            best, wall = lum(c), c
    plywood = [0.46, 0.33, 0.17]
    shard = [0.10, 0.11, 0.12]

    windows = []
    for poly in me.polygons:
        if poly.area < 0.9 or poly.area > 3.0:
            continue
        n = poly.normal
        if max(abs(n.x), abs(n.y), abs(n.z)) < 0.9:
            continue
        if lum(avg(poly)) > 0.20:
            continue
        vs = [me.vertices[vi].co for vi in poly.vertices]
        ax = max(range(3), key=lambda i: abs(n[i]))
        lat = [i for i in range(3) if i != ax]
        u0 = min(v[lat[0]] for v in vs); u1 = max(v[lat[0]] for v in vs)
        w0 = min(v[lat[1]] for v in vs); w1 = max(v[lat[1]] for v in vs)
        if not (1.4 <= u1 - u0 <= 1.8 and 1.3 <= w1 - w0 <= 1.8):
            continue
        c = [sum(v[i] for v in vs) / len(vs) for i in range(3)]
        windows.append((poly.index, ax, n.copy(), c, u0, u1, w0, w1))

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

    boarded = broken = 0
    for (pidx, ax, n, c, u0, u1, w0, w1) in windows:
        r = pick(c, 1)
        if r < 0.18:
            # 封板：贴洞口向内 6 cm，四周留 3 cm 缝
            inset = 0.03
            center = [0.0, 0.0, 0.0]
            size = [0.0, 0.0, 0.0]
            center[ax] = c[ax] + n[ax] * 0.06
            size[ax] = 0.03
            lat = [i for i in range(3) if i != ax]
            center[lat[0]] = (u0 + u1) * 0.5
            size[lat[0]] = (u1 - u0) - 2 * inset
            center[lat[1]] = (w0 + w1) * 0.5
            size[lat[1]] = (w1 - w0) - 2 * inset
            add_box(tuple(center), tuple(size), plywood)
            boarded += 1
        elif r < 0.30:
            # 破窗：底板压暗 + 底边 3 片碎玻璃
            poly = me.polygons[pidx]
            for li in poly.loop_indices:
                cc = ca.data[li].color
                ca.data[li].color = (cc[0] * 0.55, cc[1] * 0.55, cc[2] * 0.55, cc[3])
            lat = [i for i in range(3) if i != ax]
            for k in range(3):
                center = [0.0, 0.0, 0.0]
                size = [0.0, 0.0, 0.0]
                center[ax] = c[ax] + n[ax] * 0.04
                size[ax] = 0.02
                center[lat[0]] = u0 + (u1 - u0) * (0.2 + 0.25 * k)
                size[lat[0]] = 0.12
                center[lat[1]] = w0 + 0.05
                size[lat[1]] = 0.06
                add_box(tuple(center), tuple(size), shard)
            broken += 1

    bm.to_mesh(me)
    bm.free()
    me.update()
    xs = [v.co.x for v in me.vertices]
    ys = [v.co.y for v in me.vertices]
    zs = [v.co.z for v in me.vertices]
    print(f"VARIANTS windows={len(windows)} boarded={boarded} broken={broken} "
          f"bbox {min(xs):.3f}..{max(xs):.3f} / {min(ys):.3f}..{max(ys):.3f} / "
          f"{min(zs):.3f}..{max(zs):.3f} verts={len(me.vertices)}")
    assert (min(xs), max(xs), min(ys), max(ys), min(zs), max(zs)) == bbox or (
        abs(min(xs) - bbox[0]) < 1e-4 and abs(max(xs) - bbox[1]) < 1e-4
        and abs(min(ys) - bbox[2]) < 1e-4 and abs(max(ys) - bbox[3]) < 1e-4
        and abs(min(zs) - bbox[4]) < 1e-4 and abs(max(zs) - bbox[5]) < 1e-4
    ), "变体越界：包围盒变了"
    bpy.ops.export_scene.gltf(
        filepath=out, export_format="GLB", use_selection=True, export_yup=True,
        export_apply=True, export_normals=True, export_texcoords=True,
        export_vertex_color="NAME", export_vertex_color_name="Col",
        export_materials="NONE", export_extras=False, export_animations=False,
        export_morph=False, export_skins=False,
    )


main()
