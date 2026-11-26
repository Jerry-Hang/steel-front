"""造一件**带内嵌贴图**的道具（2026-11-26）：`crate_textured`。

## 为什么需要它

引擎的贴图链路（GLB 内嵌贴图 → 图集 → 采样）在 2026-11-25/26 已经通了并且有判据，
但 `asset.atlas_pack` 实测 `packed=0 skipped=52` —— **现有 52 件道具一件都没有贴图**。
没有带贴图的资产，整条链在画面上永远是"看不出变化"。
所以造一件**真的带贴图**的道具，作为链路的活样本。

## 遵守的约定（铁律 D）

- 1 单位 = 1 米；**原点在底面中心**（z ≥ 0）；Blender +Z 上；`export_yup=True`
- 单 mesh / 单 primitive / 节点不带变换
- **不导出法线**（引擎按位置平均重算），但**要 UV**（这次的目的就是贴图）
- AO/明暗烘进顶点色（引擎的顶点色是唯一的"烘焙通道"）
- 贴图**内嵌**进 GLB（`export_image_format` 走 AUTO，图片被打包进 BIN）

## 尺寸

0.80 × 0.80 × 0.80 m —— 一个可堆叠的木箱。**契约表必须同步**：
`props.rs::CONTRACT_SIZES` 里加一行，尺寸差必须 < 1mm（判据 `contract_matches_loaded_assets`）。

用法：
    blender --background --python tools/blender/make_textured_crate.py -- <out.glb>
"""

import math
import sys

import bpy

OUT = sys.argv[sys.argv.index("--") + 1] if "--" in sys.argv else "/tmp/crate_textured.glb"

HALF = 0.40  # 半边长 ⇒ 0.80 m 立方
TEX = 64  # 贴图边长（小图足够：道具贴图就是要糊一点才像）

# 六个面的法线方向（用于 UV 投影与顶点色明暗）
FACES = {
    "+X": (1, 0, 0),
    "-X": (-1, 0, 0),
    "+Y": (0, 1, 0),
    "-Y": (0, -1, 0),
    "+Z": (0, 0, 1),
    "-Z": (0, 0, -1),
}


def clear_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def make_box():
    """手搓一个**木箱结构**：箱体 + 4 根角柱 + 8 根横档。

    为什么不是"一个裸立方体"：本仓有硬性判据 `verts.len() > 24`
    （`glb_prop_kit_loads_with_valid_range` / `prop_meshes_follow_the_meter_and_base_origin_rules`），
    目的就是挡住"一个盒子冒充道具"。24 个顶点的立方体正好卡在线上 ⇒ 必须做出真实结构。
    顺带也更有木箱的样子，贴图能看出板条与铆钉。

    顶点色烘 AO：底面最暗、顶面最亮，侧面之间也分层次 —— 引擎不算法线细节，明暗全看顶点色。
    """
    verts, faces, uvs, cols = [], [], [], []

    def add_box(cx, cy, cz, sx, sy, sz, shade):
        """加一个轴对齐盒子（中心 + 半尺寸），六面各一张 UV。"""
        hx, hy, hz = sx * 0.5, sy * 0.5, sz * 0.5
        quads = [
            ([(1, 0, 0), (1, 0, 1), (1, 1, 1), (1, 1, 0)], 0.92),
            ([(0, 0, 0), (0, 1, 0), (0, 1, 1), (0, 0, 1)], 0.86),
            ([(1, 1, 0), (0, 1, 0), (0, 1, 1), (1, 1, 1)], 0.90),
            ([(0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1)], 0.84),
            ([(0, 0, 1), (1, 0, 1), (1, 1, 1), (0, 1, 1)], 1.00),
            ([(0, 0, 0), (0, 1, 0), (1, 1, 0), (1, 0, 0)], 0.55),
        ]
        for corners, face_shade in quads:
            base = len(verts)
            for (bx, by, bz) in corners:
                verts.append(
                    (
                        cx + (bx * 2 - 1) * hx,
                        cy + (by * 2 - 1) * hy,
                        cz + (bz * 2 - 1) * hz,
                    )
                )
                c = shade * face_shade * (0.94 + 0.06 * bz)
                cols.append((c, c * 0.96, c * 0.9))
            uvs.extend([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])
            faces.append((base, base + 1, base + 2, base + 3))

    # 1) 箱体（略小于外轮廓，让角柱凸出来一点）
    add_box(0, 0, HALF, HALF * 1.92, HALF * 1.92, HALF * 2.0, 0.95)
    # 2) 4 根角柱：沿 Z 的竖条
    post = 0.055
    for sx in (-1, 1):
        for sy in (-1, 1):
            # 🔴 角柱**必须从 z=0 起**：原点在底面是硬约定，
            # 第一版用 `cz=HALF, sz=HALF*2.02` ⇒ 底端伸到 -0.004（原点不在底面）。
            post_h = HALF * 2.0 + 0.008
            add_box(
                sx * (HALF - post * 0.5),
                sy * (HALF - post * 0.5),
                post_h * 0.5,
                post,
                post,
                post_h,
                1.0,
            )
    # 3) 8 根横档：每个高度的四条边（底/中/上三档各四面）
    rail = 0.05
    for z in (0.06, HALF, HALF * 2.0 - 0.06):
        for side in range(4):
            if side == 0:
                add_box(0, -HALF + rail * 0.5, z, HALF * 2.0, rail, rail, 1.05)
            elif side == 1:
                add_box(0, HALF - rail * 0.5, z, HALF * 2.0, rail, rail, 1.05)
            elif side == 2:
                add_box(-HALF + rail * 0.5, 0, z, rail, HALF * 2.0, rail, 1.02)
            else:
                add_box(HALF - rail * 0.5, 0, z, rail, HALF * 2.0, rail, 1.02)

    mesh = bpy.data.meshes.new("crate_textured")
    mesh.from_pydata(verts, [], faces)
    mesh.update()

    uv_layer = mesh.uv_layers.new(name="UVMap")
    for poly in mesh.polygons:
        for k, li in enumerate(poly.loop_indices):
            uv_layer.data[li].uv = uvs[poly.index * 4 + k]

    col = mesh.color_attributes.new(name="Col", type="FLOAT_COLOR", domain="CORNER")
    for poly in mesh.polygons:
        for k, li in enumerate(poly.loop_indices):
            r, g, b = cols[poly.index * 4 + k]
            col.data[li].color = (r, min(g, 1.0), min(b, 1.0), 1.0)
    return mesh


def make_texture():
    """造一张 64×64 的板条纹理，**存成 PNG 文件再打包进 GLB**。

    图案：深色木底 + 横向板条 + 边缘铆钉。用 numpy 直接写像素比调 procedural 节点稳。
    """
    import numpy as np

    a = np.zeros((TEX, TEX, 4), dtype=np.float32)
    yy, xx = np.mgrid[0:TEX, 0:TEX]
    # 底色：温暖的木褐
    a[..., 0] = 0.42
    a[..., 1] = 0.28
    a[..., 2] = 0.16
    # 横向板条：每 16 像素一条暗缝
    seam = (yy % 16) < 2
    a[seam, 0] = 0.16
    a[seam, 1] = 0.10
    a[seam, 2] = 0.06
    # 木纹：正弦扰动
    grain = 0.06 * np.sin(xx * 0.55 + (yy // 16) * 1.7)
    a[..., 0] += grain
    a[..., 1] += grain * 0.8
    # 铆钉：四个角的亮点
    for (cx, cy) in [(6, 6), (TEX - 7, 6), (6, TEX - 7), (TEX - 7, TEX - 7)]:
        m = (xx - cx) ** 2 + (yy - cy) ** 2 <= 9
        a[m, 0] = 0.75
        a[m, 1] = 0.72
        a[m, 2] = 0.66
    a[..., 3] = 1.0

    img = bpy.data.images.new("crate_albedo", width=TEX, height=TEX, alpha=True)
    # Blender 的像素是**从下往上**存的，所以要把 numpy 的行序翻一次
    img.pixels = a[::-1].ravel().tolist()
    img.filepath_raw = "/tmp/crate_albedo.png"
    img.file_format = "PNG"
    img.save()
    return img


def make_material(img):
    mat = bpy.data.materials.new("crate_textured_mat")
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = nt.nodes.get("Principled BSDF")
    tex = nt.nodes.new("ShaderNodeTexImage")
    tex.image = img
    tex.interpolation = "Closest"  # 像素风：保住硬边，缩放时不糊
    nt.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    return mat


def main():
    clear_scene()
    mesh = make_box()
    obj = bpy.data.objects.new("crate_textured", mesh)
    bpy.context.collection.objects.link(obj)
    img = make_texture()
    mesh.materials.append(make_material(img))

    # 选中并导出
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=OUT,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=False,
        # 🔴 引擎不需要 NORMAL（按位置平均重算），但**必须**要 UV 与 COLOR_0
        export_normals=False,
        export_texcoords=True,
        export_vertex_color="NAME",
        export_vertex_color_name="Col",
        export_image_format="AUTO",  # 图片打包进 GLB 的 BIN 块
        export_materials="EXPORT",
    )
    print(f"crate_textured: 写出 {OUT}")


main()
