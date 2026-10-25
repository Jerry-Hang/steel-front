"""Blender 真建模：给一部分窗洞点亮（顶点色暖光，零几何）。

现状：所有窗洞背板是同一种冷灰，傍晚/低太阳时整栋楼像没人住。
按 crc32(窗洞位置) 抽签：约 32% 点暖光（室内灯），其中约 1/4 更亮（顶灯/临窗）。
只改背板顶点色，不动几何 ⇒ 包围盒/顶点数/三角形数全不变。

用法： blender --background --python tools/blender/lit_windows.py -- in.glb out.glb
"""
import sys, zlib, bpy


def pick(c, salt=3):
    h = zlib.crc32(f"{c[0]:.2f},{c[1]:.2f},{c[2]:.2f},{salt}".encode()) & 0xFFFF
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
    bbox0 = (min(xs), max(xs), min(ys), max(ys), min(zs), max(zs))

    # 要"像亮着灯"而不是"像封了木板"：亮度必须明显高于墙面（墙 ~0.55），
    # 且比封板（0.46,0.33,0.17）更白、更不饱和。
    warm = (1.30, 1.10, 0.78)
    bright = (1.55, 1.35, 0.95)

    # 窗背板被三角化：同窗的两片三角质心差半米级，按质心分组合不到一起。
    # 这里按「同一平面（轴向 + 符号 + 平面坐标 1cm 网格）」分组，再在平面内按
    # **矩形重叠** 做贪心聚类 ⇒ 一扇窗 = 一个 cluster，整组一起判、一起上色。
    planes = {}
    for poly in me.polygons:
        if poly.area < 0.30:
            continue
        n = poly.normal
        ax = max(range(3), key=lambda i: abs(n[i]))
        if abs(n[ax]) < 0.9 or lum(avg(poly)) > 0.20:
            continue
        vs = [me.vertices[vi].co for vi in poly.vertices]
        lat = [i for i in range(3) if i != ax]
        u0 = min(v[lat[0]] for v in vs); u1 = max(v[lat[0]] for v in vs)
        w0 = min(v[lat[1]] for v in vs); w1 = max(v[lat[1]] for v in vs)
        plane = sum(v[ax] for v in vs) / len(vs)
        key = (ax, 1 if n[ax] > 0 else -1, round(plane / 0.01))
        planes.setdefault(key, []).append((poly.index, u0, u1, w0, w1))

    def overlaps(a, b, tol=0.06):
        return (min(a[2], b[2]) - max(a[1], b[1]) > -tol
                and min(a[4], b[4]) - max(a[3], b[3]) > -tol)

    windows = lit = bright_lit = 0
    for _key, items in planes.items():
        clusters = []
        for it in items:
            placed = False
            for cl in clusters:
                if any(overlaps(it, other) for other in cl):
                    cl.append(it)
                    placed = True
                    break
            if not placed:
                clusters.append([it])
        for cl in clusters:
            u0 = min(i[1] for i in cl); u1 = max(i[2] for i in cl)
            w0 = min(i[3] for i in cl); w1 = max(i[4] for i in cl)
            if not (1.4 <= u1 - u0 <= 1.8 and 1.3 <= w1 - w0 <= 1.8):
                continue
            windows += 1
            c = (0.0, 0.0, 0.0)  # 用簇的矩形中心 + 平面索引做抽签键（确定性）
            r = pick((round((u0 + u1) * 0.5, 2), round((w0 + w1) * 0.5, 2), _key[2] * 0.01))
            if r < 0.08:
                col, tag = bright, 2
            elif r < 0.32:
                col, tag = warm, 1
            else:
                continue
            for (pidx, *_rest) in cl:
                for li in me.polygons[pidx].loop_indices:
                    ca.data[li].color = (col[0], col[1], col[2], 1.0)
            if tag == 2:
                bright_lit += 1
            else:
                lit += 1

    xs = [v.co.x for v in me.vertices]
    ys = [v.co.y for v in me.vertices]
    zs = [v.co.z for v in me.vertices]
    bbox1 = (min(xs), max(xs), min(ys), max(ys), min(zs), max(zs))
    assert all(abs(a - b) < 1e-4 for a, b in zip(bbox0, bbox1)), "点亮窗洞改了包围盒"
    print(f"LITW windows={windows} warm={lit} bright={bright_lit} verts={len(me.vertices)}")
    bpy.ops.export_scene.gltf(
        filepath=out, export_format="GLB", use_selection=True, export_yup=True,
        export_apply=True, export_normals=True, export_texcoords=True,
        export_vertex_color="NAME", export_vertex_color_name="Col",
        export_materials="NONE", export_extras=False, export_animations=False,
        export_morph=False, export_skins=False,
    )


main()
