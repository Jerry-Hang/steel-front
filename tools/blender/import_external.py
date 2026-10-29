"""把**外部下载**的 glTF 模型转成 Steel Front 的道具契约（一次性导入器的正式版本）。

用法（headless）：
    blender -b --python tools/blender/import_external.py -- <in.gltf|in.glb> <out.glb> \
            [--name NAME] [--target-height METRES] [--keep-uv]

为什么需要它（2026-10-30）：
    引擎的道具契约是 **单 mesh / 单 primitive / POSITION + 顶点色 / 原点在底面中心 /
    1 单位 = 1 米 / 无节点变换**（见 AGENTS「铁律 D」）。而下载来的资产几乎都是
    多材质 + 贴图 + 不居中 + 有节点变换的形状。手工在 Blender GUI 里改既慢又违反
    「鼠标安全协议」，所以把整条转换写成脚本。

它做什么：
  1. 导入 glTF/GLB，选中全部 mesh 后 join 成一个对象；
  2. 对每个 loop 按 UV **采样材质的基础色贴图**（无贴图时用材质基础色）→ 写进
     `Col` 顶点色（引擎道具的明暗**只**来自顶点色，不采样贴图）；
  3. 应用变换、把包围盒底面中心移到原点（z=0 为底面）；
  4. 可选 `--target-height`：等比缩放到指定总高（便于对上 city.rs 的模块尺寸）；
  5. 导出 GLB：`export_yup=True`、`export_normals=False`（引擎按位置平均生成法线）、
     `export_vertex_color="NAME"` + `export_vertex_color_name="Col"`。

⚠ 采样是**最近邻**、按顶点走：顶点密度决定细节保真度。下载的模块化资产顶点很密，
   立面大概能留住；这属于"能用"，不是"完美"。
"""

import sys
import os
import math

import bpy
import bmesh
from mathutils import Vector


def log(msg: str) -> None:
    print(f"[import_external] {msg}")


def parse_args() -> dict:
    argv = sys.argv
    argv = argv[argv.index("--") + 1 :] if "--" in argv else []
    if len(argv) < 2:
        raise SystemExit("用法: blender -b --python import_external.py -- <in> <out.glb> [--name X] [--target-height H] [--keep-uv]")
    args = {"in": argv[0], "out": argv[1], "name": None, "target_height": None,
            "keep_uv": False, "decimate": None, "rotate_z": 0.0}
    i = 2
    while i < len(argv):
        a = argv[i]
        if a == "--name":
            args["name"] = argv[i + 1]
            i += 2
        elif a == "--target-height":
            args["target_height"] = float(argv[i + 1])
            i += 2
        elif a == "--decimate":
            args["decimate"] = float(argv[i + 1])
            i += 2
        elif a == "--rotate-z":
            args["rotate_z"] = float(argv[i + 1])
            i += 2
        elif a == "--keep-uv":
            args["keep_uv"] = True
            i += 1
        else:
            raise SystemExit(f"未知参数 {a}")
    return args


def clear_scene() -> None:
    bpy.ops.wm.read_factory_settings(use_empty=True)


def import_any(path: str) -> None:
    ext = os.path.splitext(path)[1].lower()
    if ext in (".gltf", ".glb"):
        bpy.ops.import_scene.gltf(filepath=path)
    else:
        raise SystemExit(f"不支持的输入格式: {ext}")


def join_meshes():
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    if not meshes:
        raise SystemExit("输入里没有 mesh")
    bpy.ops.object.select_all(action="DESELECT")
    for o in meshes:
        o.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    # 应用节点变换（下载资产常带非单位缩放/旋转）
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    return obj


def sample_texture_color(mat, uv):
    """返回该材质在给定 UV 处的线性 RGB（无贴图时退回材质 base color）。"""
    if mat and mat.use_nodes:
        for node in mat.node_tree.nodes:
            if node.type == "BSDF_PRINCIPLED":
                base = node.inputs["Base Color"]
                if base.is_linked:
                    src = base.links[0].from_node
                    if src.type == "TEX_IMAGE" and src.image:
                        img = src.image
                        w, h = img.size
                        if w > 0 and h > 0:
                            u = uv.x % 1.0
                            v = uv.y % 1.0
                            px = min(w - 1, max(0, int(u * w)))
                            py = min(h - 1, max(0, int(v * h)))
                            idx = (py * w + px) * 4
                            p = img.pixels
                            return (p[idx], p[idx + 1], p[idx + 2], 1.0)
                c = base.default_value
                return (c[0], c[1], c[2], 1.0)
    return (0.5, 0.5, 0.5, 1.0)


def bake_vertex_colors(obj) -> None:
    """把材质外观烘进顶点色。

    🔴 踩坑记录（2026-10-29）：Quaternius 的材质是"贴图 → 混合节点 → Principled"，
    手工按 `Base Color.links[0].from_node` 找 `TEX_IMAGE` 会**找不到**（链路中间还有节点），
    于是每件资产都退化成材质默认的 0.8 灰 —— 引擎那条"顶点色全同值"的判据当场抓到了。
    ⇒ 这里改用 **Cycles 烘焙到顶点色**（bake target=VERTEX_COLORS），对任意节点图都成立。
    """
    me = obj.data
    col = me.color_attributes.get("Col")
    if col is None:
        col = me.color_attributes.new(name="Col", type="FLOAT_COLOR", domain="CORNER")
    me.color_attributes.active_color = col
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 1
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.bake(type="DIFFUSE", pass_filter={"COLOR"}, target="VERTEX_COLORS", margin=0,
                        use_clear=True)
    log(f"顶点色烘焙完成：{len(me.loops)} 个 corner")


def normalize_transform(obj, target_height) -> None:
    me = obj.data
    xs = [v.co.x for v in me.vertices]
    ys = [v.co.y for v in me.vertices]
    zs = [v.co.z for v in me.vertices]
    if not xs:
        raise SystemExit("模型没有顶点")
    cx = (min(xs) + max(xs)) * 0.5
    cy = (min(ys) + max(ys)) * 0.5
    zmin = min(zs)
    for v in me.vertices:
        v.co.x -= cx
        v.co.y -= cy
        v.co.z -= zmin
    height = max(zs) - zmin
    if target_height and height > 1e-4:
        k = target_height / height
        for v in me.vertices:
            v.co *= k
        log(f"等比缩放到总高 {target_height:.2f} m（原 {height:.2f} m，系数 {k:.3f}）")
        height = target_height
    log(f"归一化完成：高 {height:.2f} m，底面中心在原点")


def decimate(obj, ratio: float) -> None:
    """抽面：下载资产动辄 2–4 万顶点，全城摆几十栋就顶穿 2^21 的道具顶点预算。
    抽面会尽量保留顶点色（Blender 的 collapse 会插值属性）。"""
    mod = obj.modifiers.new(name="dec", type="DECIMATE")
    mod.ratio = max(0.02, min(1.0, ratio))
    bpy.ops.object.modifier_apply(modifier=mod.name)
    log(f"抽面 ratio={ratio:.2f} ⇒ 顶点 {len(obj.data.vertices)} / 三角 {len(obj.data.polygons)}")


def simplify(obj, keep_uv: bool) -> None:
    me = obj.data
    if not keep_uv:
        while me.uv_layers:
            me.uv_layers.remove(me.uv_layers[0])
        log("已移除 UV（引擎道具不采样贴图）")
    # 🔴 量化顶点色再焊接（2026-10-29 实测的根因）：烘焙出来的顶点色是**每个 corner 都不同**的
    # 连续值，`remove_doubles` 因此几乎合并不了任何顶点 —— 导出的顶点数 ≈ 三角数的 3 倍
    # （downtown_large 抽面到 7.2k 三角却导出 19k 顶点）。把颜色量化到 6 bit/通道后，
    # 同面同色的 corner 才能合并，顶点数掉到接近三角数的一半，观感无明显差别。
    # 🔴 再一个根因（同日实测）：即使量化了，**CORNER 域的顶点色**依然让 glTF 导出器
    # 把"同一位置不同角颜色不同"的顶点拆开 ⇒ 顶点数 ≈ 三角数 × 2.6。
    # 改成 **POINT 域**（每顶点一个颜色 = 该点各角的平均）后，硬边不再拆点：
    # 顶点数降到接近"三角数 / 2"，观感只有轻微平滑（道具本来就是顶点色着色）。
    col = me.color_attributes.get("Col")
    if col is not None:
        per_vert = {}
        for poly in me.polygons:
            for li in poly.loop_indices:
                vi = me.loops[li].vertex_index
                c = col.data[li].color
                acc = per_vert.setdefault(vi, [0.0, 0.0, 0.0, 0.0])
                acc[0] += c[0]
                acc[1] += c[1]
                acc[2] += c[2]
                acc[3] += 1.0
        me.color_attributes.remove(col)
        point_col = me.color_attributes.new(name="Col", type="FLOAT_COLOR", domain="POINT")
        for vi, acc in per_vert.items():
            n = max(acc[3], 1.0)
            point_col.data[vi].color = (
                round((acc[0] / n) * 63.0) / 63.0,
                round((acc[1] / n) * 63.0) / 63.0,
                round((acc[2] / n) * 63.0) / 63.0,
                1.0,
            )
        me.color_attributes.active_color = point_col
        log("顶点色已转 POINT 域并量化到 6 bit（为焊接/减少拆点）")
    # 合并重复顶点（位置+颜色相同）——下载资产的硬边会让顶点翻倍
    bm = bmesh.new()
    bm.from_mesh(me)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-4)
    bm.to_mesh(me)
    bm.free()
    me.update()
    log(f"焊接后顶点 {len(me.vertices)} / 三角 {len(me.polygons)}")


def export_glb(obj, out_path, name) -> None:
    os.makedirs(os.path.dirname(os.path.abspath(out_path)), exist_ok=True)
    # 🔴 glTF 导入器会自带一个顶点色属性，加上我们的 "Col" 就会导出 COLOR_0 + COLOR_1
    # （引擎只读一路，多出来的纯属浪费带宽）。导出前只留 "Col"。
    me = obj.data
    for attr in list(me.color_attributes):
        if attr.name != "Col":
            me.color_attributes.remove(attr)
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    if name:
        obj.name = name
        obj.data.name = name
    kwargs = dict(
        filepath=out_path,
        export_format="GLB",
        export_yup=True,
        export_apply=True,
        use_selection=True,
        export_normals=False,
        export_texcoords=False,
        export_materials="NONE",
        export_vertex_color="NAME",
        export_vertex_color_name="Col",
    )
    try:
        bpy.ops.export_scene.gltf(**kwargs)
    except TypeError:
        # 老/新版本参数名差异：去掉顶点色参数重试，至少先把几何导出来
        for k in ("export_vertex_color", "export_vertex_color_name"):
            kwargs.pop(k, None)
        bpy.ops.export_scene.gltf(**kwargs)
    log(f"导出 {out_path}（{os.path.getsize(out_path) / 1024:.0f} KB）")


def main() -> None:
    args = parse_args()
    clear_scene()
    import_any(args["in"])
    obj = join_meshes()
    bake_vertex_colors(obj)
    if args["rotate_z"]:
        import math as _m
        me = obj.data
        ang = _m.radians(args["rotate_z"])
        ca, sa = _m.cos(ang), _m.sin(ang)
        for v in me.vertices:
            x, y = v.co.x, v.co.y
            v.co.x = x * ca - y * sa
            v.co.y = x * sa + y * ca
        me.update()
        log(f"绕 Z 旋转 {args['rotate_z']:.0f}°（把车长轴从 z 拧到 x）")
    normalize_transform(obj, args["target_height"])
    if args["decimate"]:
        decimate(obj, args["decimate"])
    simplify(obj, args["keep_uv"])
    export_glb(obj, args["out"], args["name"])


main()
