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
            "keep_uv": False, "decimate": None, "rotate_z": 0.0,
            "cutout": 0.0, "cutout_cuts": 3, "double_sided": False,
            "atlas_slot": None, "atlas_out": "assets/textures/prop_atlas.png",
            "slim_trunk": 1.0}
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
        elif a == "--cutout":
            args["cutout"] = float(argv[i + 1])
            i += 2
        elif a == "--cutout-cuts":
            args["cutout_cuts"] = int(argv[i + 1])
            i += 2
        elif a == "--double-sided":
            args["double_sided"] = True
            i += 1
        elif a == "--atlas-slot":
            args["atlas_slot"] = int(argv[i + 1])
            i += 2
        elif a == "--atlas-out":
            args["atlas_out"] = argv[i + 1]
            i += 2
        elif a == "--slim-trunk":
            args["slim_trunk"] = float(argv[i + 1])
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


def _rgba_sampler(mat, threshold: float):
    """返回 (uv)->alpha 的采样器；材质没有贴图时返回 None。"""
    if not (mat and mat.use_nodes):
        return None
    img = None
    for n in mat.node_tree.nodes:
        if n.type == "TEX_IMAGE" and n.image:
            img = n.image
            break
    if img is None:
        return None
    w, h = img.size
    if w == 0 or h == 0:
        return None
    px = list(img.pixels)  # 一次性取，逐点访问太慢

    def sample(u, v):
        px_x = min(w - 1, max(0, int((u % 1.0) * w)))
        px_y = min(h - 1, max(0, int((v % 1.0) * h)))
        i = (px_y * w + px_x) * 4
        return (px[i], px[i + 1], px[i + 2], px[i + 3])

    return sample


def override_colors_from_textures(obj) -> None:
    fixed = 0
    """对**带贴图**的材质，用 UV 采样的贴图 RGB 覆盖顶点色。

    为什么要这一步（实测）：Cycles 的 DIFFUSE COLOR bake 对 alpha / 半透明材质给的是**黑**，
    于是叶子烘出来全黑（预览图实锤：树冠是一团黑刺）。改为直接读贴图像素后，
    叶子有真实绿/棕，且与 alpha 裁形用的是同一张图，轮廓和颜色天然对齐。
    """
    me = obj.data
    col = me.color_attributes.get("Col")
    if col is None:
        return
    mats = [slot.material for slot in obj.material_slots]
    samplers = {i: _rgba_sampler(m, 0.5) for i, m in enumerate(mats)}
    if not any(samplers.values()):
        return
    uv_layer = me.uv_layers.active.data if me.uv_layers else None
    if uv_layer is None:
        return
    # 🔴 逐点采样对**叶片**不可靠：alpha 遮罩贴图在叶片外的 RGB 是纯黑，叶片内的 RGB
    # 也可能接近黑（颜色信息在别处）——实测 2000 个顶点里有 650 个是 (0,0,0)，树冠因此全黑。
    # 改成**按材质求平均色**（只统计 alpha>0.5 的纹素），同材质的面统一上色。
    for mi, sample in samplers.items():
        if sample is None:
            continue
        mat = mats[mi]
        img = None
        if mat and mat.use_nodes:
            for n in mat.node_tree.nodes:
                if n.type == "TEX_IMAGE" and n.image:
                    img = n.image
                    break
        if img is None:
            continue
        w, h = img.size
        px = list(img.pixels)
        sr = sg = sb = sw = 0.0
        step = max(1, (w * h) // 4096)  # 采样 4k 点即可
        for i in range(0, w * h, step):
            a = px[i * 4 + 3]
            if a <= 0.5:
                continue
            sr += px[i * 4]
            sg += px[i * 4 + 1]
            sb += px[i * 4 + 2]
            sw += 1.0
        if sw < 1.0:
            continue
        mean = (sr / sw, sg / sw, sb / sw, 1.0)
        log(f"材质 {mi} 平均色 = ({mean[0]:.2f}, {mean[1]:.2f}, {mean[2]:.2f})，来自 {sw:.0f} 个纹素")
        for poly in me.polygons:
            if poly.material_index != mi:
                continue
            for li in poly.loop_indices:
                col.data[li].color = mean
                fixed += 1
    log(f"贴图覆盖顶点色：{fixed} 个 corner（按材质平均色）")


def alpha_cutout(obj, threshold: float, cuts: int) -> None:
    """把带 alpha 的"叶子卡片"**裁成真实几何**：细分后删掉透明区的小面。

    为什么这么做（2026-10-29 的失败记录）：引擎的道具**只吃顶点色、不采样贴图、没有 alpha test**，
    所以下载的树叶子要么全黑、要么整片实心方块。与其去改渲染管线加材质通道（那是另一件事），
    不如在**导入期**把"透明就等于不存在"这件事直接烘进几何：叶片变成真实轮廓，仍然只靠顶点色。
    """
    me = obj.data
    mats = [slot.material for slot in obj.material_slots]
    samplers = {i: _rgba_sampler(m, threshold) for i, m in enumerate(mats)}
    if not any(samplers.values()):
        log("没有找到带 alpha 的贴图 ⇒ 跳过裁形")
        return
    bm = bmesh.new()
    bm.from_mesh(me)
    uv_lay = bm.loops.layers.uv.active
    if uv_lay is None:
        log("网格没有 UV ⇒ 跳过裁形")
        bm.free()
        return
    target_faces = [f for f in bm.faces if samplers.get(f.material_index)]
    target_edges = {e for f in target_faces for e in f.edges}
    if not target_edges:
        bm.free()
        return
    bmesh.ops.subdivide_edges(bm, edges=list(target_edges), cuts=cuts, use_grid_fill=True)
    dead = []
    for f in bm.faces:
        sample = samplers.get(f.material_index)
        if sample is None:
            continue
        uvs = [l[uv_lay].uv for l in f.loops]
        cu = sum(u.x for u in uvs) / len(uvs)
        cv = sum(u.y for u in uvs) / len(uvs)
        if sample(cu, cv)[3] < threshold:
            dead.append(f)
    if dead:
        bmesh.ops.delete(bm, geom=dead, context="FACES")
    bm.to_mesh(me)
    bm.free()
    me.update()
    log(f"alpha 裁形完成（阈值 {threshold:.2f}，细分 {cuts}）：删掉 {len(dead)} 个小面，剩 {len(me.polygons)} 三角")


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


def make_double_sided(obj) -> None:
    """给**带贴图的叶片材质**复制一份反面（法线翻转）。

    为什么要这一步：引擎按顶点法线做 N·L，卡片是单面的 —— 背光那一侧整片偏黑，
    而叶子本来就该两面可见（真树叶子半透明）。复制反面是"渲染端双面"的离线等价物：
    不动管线、不加 alpha test，只在导入期把另一半补上。代价是叶片三角翻倍。
    """
    me = obj.data
    mats = [slot.material for slot in obj.material_slots]
    leaf_idx = {i for i, m in enumerate(mats) if m and m.use_nodes and any(
        n.type == "TEX_IMAGE" and n.image for n in m.node_tree.nodes)}
    if not leaf_idx:
        log("没有带贴图的材质 ⇒ 跳过双面")
        return
    bm = bmesh.new()
    bm.from_mesh(me)
    faces = [f for f in bm.faces if f.material_index in leaf_idx]
    ret = bmesh.ops.duplicate(bm, geom=faces)
    dup = [g for g in ret["geom"] if isinstance(g, bmesh.types.BMFace)]
    bmesh.ops.reverse_faces(bm, faces=dup)
    bm.to_mesh(me)
    bm.free()
    me.update()
    log(f"双面叶片：复制 {len(dup)} 个面（三角 {len(me.polygons)}）")


ATLAS_SIZE = 2048
ATLAS_CELL = 512
ATLAS_GRID = ATLAS_SIZE // ATLAS_CELL  # 4


def pack_atlas_and_remap_uv(obj, slot: int, out_path: str) -> None:
    """把该物件的 base color 贴图贴进**共享图集**的第 slot 格，并把 UV 重映射进那一格。

    为什么要图集而不是每件一张贴图：引擎只有**一个** `prop_atlas_tex` 绑定（binding 12），
    所有道具共用一张图；每件占 512² 一格（2048² 共 16 格），UV 平移到 `[cx/4, (cx+1)/4]`。
    贴图按**最近邻放大**到 512²，避免引入采样器差异（细节靠纹理本身密度）。
    """
    import numpy as np
    me = obj.data
    if not me.uv_layers.active:
        log("图集：网格没有 UV ⇒ 跳过")
        return
    mats = [slot_m.material for slot_m in obj.material_slots]
    img = None
    for m in mats:
        if m and m.use_nodes:
            for n in m.node_tree.nodes:
                if n.type == "TEX_IMAGE" and n.image:
                    img = n.image
                    break
        if img:
            break
    # 打开/新建图集
    import os
    atlas = None
    if os.path.exists(out_path):
        try:
            atlas = bpy.data.images.load(out_path, check_existing=True)
        except Exception:
            atlas = None
    if atlas is None or tuple(atlas.size) != (ATLAS_SIZE, ATLAS_SIZE):
        atlas = bpy.data.images.new("prop_atlas", ATLAS_SIZE, ATLAS_SIZE, alpha=False)
        atlas.pixels.foreach_set(np.ones(ATLAS_SIZE * ATLAS_SIZE * 4, dtype=np.float32))
    cx = slot % ATLAS_GRID
    cy = slot // ATLAS_GRID
    if img is not None:
        w, h = img.size
        src = np.empty(w * h * 4, dtype=np.float32)
        img.pixels.foreach_get(src)
        src = src.reshape(h, w, 4)
        # 最近邻采样到 512²
        ys = (np.arange(ATLAS_CELL) * h // ATLAS_CELL).clip(0, h - 1)
        xs = (np.arange(ATLAS_CELL) * w // ATLAS_CELL).clip(0, w - 1)
        cell = src[ys][:, xs]
        dst = np.empty(ATLAS_SIZE * ATLAS_SIZE * 4, dtype=np.float32)
        atlas.pixels.foreach_get(dst)
        dst = dst.reshape(ATLAS_SIZE, ATLAS_SIZE, 4)
        # 图集原点在左下（Blender 约定），第 0 行是最下面
        x0 = cx * ATLAS_CELL
        y0 = cy * ATLAS_CELL
        dst[y0:y0 + ATLAS_CELL, x0:x0 + ATLAS_CELL] = cell
        atlas.pixels.foreach_set(dst.reshape(-1))
        log(f"图集：{img.name} → 格 {slot}（x={cx}, y={cy}）")
    else:
        log(f"图集：材质没有贴图 ⇒ 格 {slot} 留白")
    atlas.filepath_raw = os.path.abspath(out_path)
    atlas.file_format = "PNG"
    atlas.save()
    # UV 重映射进该格
    scale = 1.0 / ATLAS_GRID
    off_u = cx * scale
    off_v = cy * scale
    uv = me.uv_layers.active.data
    for d in uv:
        d.uv = (off_u + d.uv.x * scale, off_v + d.uv.y * scale)
    log(f"图集：UV 已重映射到格 {slot}（偏移 {off_u:.3f},{off_v:.3f}，缩放 {scale:.3f}）")


def slim_trunk(obj, factor: float, radius: float = 0.60, height: float = 1.8) -> None:
    """把树干（低位、细半径的顶点）径向收窄 factor 倍，向上平滑过渡。

    为什么需要它（2026-10-29 真机）：下载的风格化树树干是**上细下粗的粗锥**，近看像"圆锥墩子"。
    换成天然细干的型号不现实（`CommonTree_3` 树干 0.69m 但要 2.4 万顶点，装不进预算），
    所以在导入期直接把树干收细 —— 只动 `radial < radius` 且 `z < height` 的顶点，
    枝叶不受影响，且过渡带用 smoothstep 避免折断感。
    """
    me = obj.data
    moved = 0
    for v in me.vertices:
        z = v.co.z
        if z > height:
            continue
        rad = (v.co.x * v.co.x + v.co.y * v.co.y) ** 0.5
        if rad > radius:
            continue
        t = 1.0 - (z / height)          # 底部 1 → 顶部 0
        t = t * t * (3.0 - 2.0 * t)     # smoothstep
        k = 1.0 + (factor - 1.0) * t
        v.co.x *= k
        v.co.y *= k
        moved += 1
    me.update()
    log(f"树干收细 ×{factor:.2f}：移动 {moved} 个顶点（半径<{radius:.2f}m、高度<{height:.2f}m）")


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


def export_glb(obj, out_path, name, texcoords: bool = False) -> None:
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
        export_texcoords=texcoords,
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
    override_colors_from_textures(obj)
    if args["cutout"] > 0.0:
        alpha_cutout(obj, args["cutout"], args["cutout_cuts"])
    if args["slim_trunk"] != 1.0:
        slim_trunk(obj, args["slim_trunk"])
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
    simplify(obj, args["keep_uv"] or args["atlas_slot"] is not None)
    # ⚠ 双面必须在**焊接之后**：先复制再 remove_doubles 会把复制的反面当成重复顶点合掉。
    if args["atlas_slot"] is not None:
        pack_atlas_and_remap_uv(obj, args["atlas_slot"], args["atlas_out"])
    if args["double_sided"]:
        make_double_sided(obj)
    export_glb(obj, args["out"], args["name"], texcoords=args["atlas_slot"] is not None)


main()
