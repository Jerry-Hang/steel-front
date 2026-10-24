"""Blender 真建模：给每个窗洞加一圈内挤 6 cm 的窗框（向内，包围盒不变）。

现状：窗洞是"洞 + 深色底板"，靠顶点色假装窗；近看没有厚度。
做法：按深色底板识别每个窗洞 → 沿洞沿用 bmesh 加 4 根框料（外沿贴墙内侧 0.02 m，
向内 0.06 m），框料比墙亮 15%（白窗框），形成真实的檐口阴影与反光面。

用法： blender --background --python tools/blender/window_frames.py -- in.glb out.glb
"""
import sys, bmesh, bpy

RIM_W = 0.06      # 框料宽度（沿洞沿）
RIM_D = 0.06      # 向内深度
SETBACK = 0.02    # 从墙面向内缩 2 cm，避免与墙面共面 z-fighting


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
    trim = [min(1.0, c * 1.15) for c in wall]

    # 收集窗洞：深色、轴向、面积像窗
    openings = []
    for poly in me.polygons:
        if poly.area < 0.9 or poly.area > 3.0:
            continue
        n = poly.normal
        if max(abs(n.x), abs(n.y), abs(n.z)) < 0.9:
            continue
        if lum(face_avg(poly)) > 0.20:
            continue
        vs = [me.vertices[vi].co for vi in poly.vertices]
        openings.append((n.copy(), vs))

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

    # 去重 + 尺寸过滤：同一窗洞可能由多块深色面组成（洞壁/玻璃分片），
    # 只保留"宽 1.2–2.0 m、高 0.8–1.5 m"的矩形，并按矩形键去重。
    uniq = {}
    for n, vs in openings:
        ax = max(range(3), key=lambda i: abs(n[i]))
        sign = 1.0 if n[ax] > 0.0 else -1.0
        lateral = [i for i in range(3) if i != ax]
        u0 = round(min(v[lateral[0]] for v in vs), 2); u1 = round(max(v[lateral[0]] for v in vs), 2)
        w0 = round(min(v[lateral[1]] for v in vs), 2); w1 = round(max(v[lateral[1]] for v in vs), 2)
        if not (1.4 <= u1 - u0 <= 1.8 and 1.3 <= w1 - w0 <= 1.8):
            continue
        uniq[(ax, sign, u0, u1, w0, w1)] = (n.copy(), vs)

    made = 0
    for n, vs in uniq.values():
        ax = max(range(3), key=lambda i: abs(n[i]))
        sign = 1.0 if n[ax] > 0.0 else -1.0
        # 洞口自己的平面（深色玻璃/背板所在面），不是包围盒面 —— 否则凹龛里的窗框会飘到墙外。
        wall_plane = sum(v[ax] for v in vs) / len(vs)
        lateral = [i for i in range(3) if i != ax]
        u0 = min(v[lateral[0]] for v in vs); u1 = max(v[lateral[0]] for v in vs)
        w0 = min(v[lateral[1]] for v in vs); w1 = max(v[lateral[1]] for v in vs)
        # 框料沿墙内 0.02..0.08
        d_in = wall_plane + sign * (SETBACK + RIM_D * 0.5)
        depth = RIM_D
        def box(u_c, u_sz, w_c, w_sz):
            center = [0.0, 0.0, 0.0]
            center[ax] = d_in
            center[lateral[0]] = u_c
            center[lateral[1]] = w_c
            size = [0.0, 0.0, 0.0]
            size[ax] = depth
            size[lateral[0]] = u_sz
            size[lateral[1]] = w_sz
            add_box(tuple(center), tuple(size), trim)
        # 下 / 上
        box((u0 + u1) * 0.5, (u1 - u0), w0 + RIM_W * 0.5, RIM_W)
        box((u0 + u1) * 0.5, (u1 - u0), w1 - RIM_W * 0.5, RIM_W)
        # 左 / 右（避开上下条）
        box(u0 + RIM_W * 0.5, RIM_W, (w0 + w1) * 0.5, (w1 - w0) - 2.0 * RIM_W)
        box(u1 - RIM_W * 0.5, RIM_W, (w0 + w1) * 0.5, (w1 - w0) - 2.0 * RIM_W)
        made += 1

    if made:
        bm.to_mesh(me)
    bm.free()
    me.update()
    xs = [v.co.x for v in me.vertices]
    ys = [v.co.y for v in me.vertices]
    zs = [v.co.z for v in me.vertices]
    print(f"FRAMES windows={made} bbox {min(xs):.3f}..{max(xs):.3f} / {min(ys):.3f}..{max(ys):.3f} / "
          f"{min(zs):.3f}..{max(zs):.3f} verts={len(me.vertices)} faces={len(me.polygons)}")
    bpy.ops.export_scene.gltf(
        filepath=out, export_format="GLB", use_selection=True, export_yup=True,
        export_apply=True, export_normals=True, export_texcoords=True,
        export_vertex_color="NAME", export_vertex_color_name="Col",
        export_materials="NONE", export_extras=False, export_animations=False,
        export_morph=False, export_skins=False,
    )


main()
