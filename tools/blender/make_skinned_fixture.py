"""生成一件**带骨骼与动画**的测试用 GLB（引擎化 P4d）。

这不是游戏美术资产，而是**验证资产**：本仓 53 件 GLB 一件骨骼都没有，
所有骨骼代码此前只跑过手工拼的字节。用 Blender 的真实导出可以暴露
"合成夹具没覆盖的字段"（比如 Blender 的节点组织方式、动画通道的分组方式）。

用法（headless，勿开 GUI —— 会抢焦点）：
    blender --background --python tools/blender/make_skinned_fixture.py -- <out.glb>

模型：一根 2 段竖杆（3 个顶点），两节骨骼依次串联；
顶点权重：底端 → b0、顶端 → b1、中间 50/50。
动画：b1 在 1 秒内绕 Y 轴转 90°（25 帧 @ 24fps ≈ 1.04s）。
"""

import math
import sys

import bpy

argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
out = argv[0] if argv else "/tmp/skinned_bar.glb"

# 空场景（read_factory_settings 在 --background 下可用）
bpy.ops.wm.read_factory_settings(use_empty=True)


def add_armature():
    arm_data = bpy.data.armatures.new("rig")
    arm = bpy.data.objects.new("rig", arm_data)
    bpy.context.collection.objects.link(arm)
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.mode_set(mode="EDIT")
    b0 = arm_data.edit_bones.new("b0")
    b0.head = (0.0, 0.0, 0.0)
    b0.tail = (0.0, 0.0, 1.0)
    b1 = arm_data.edit_bones.new("b1")
    b1.head = (0.0, 0.0, 1.0)
    b1.tail = (0.0, 0.0, 2.0)
    b1.parent = b0
    b1.use_connect = True
    bpy.ops.object.mode_set(mode="OBJECT")
    return arm


def add_mesh(arm):
    """3 个顶点的竖杆；顶点组权重体现"两端各归一节、中间对半分"。"""
    mesh = bpy.data.meshes.new("bar")
    mesh.from_pydata(
        [(0.0, 0.0, 0.0), (0.0, 0.0, 1.0), (0.0, 0.0, 2.0)],
        [],
        [(0, 1, 2)],
    )
    mesh.update()
    obj = bpy.data.objects.new("bar", mesh)
    bpy.context.collection.objects.link(obj)

    g0 = obj.vertex_groups.new(name="b0")
    g1 = obj.vertex_groups.new(name="b1")
    g0.add([0], 1.0, "REPLACE")
    g1.add([1], 1.0, "REPLACE")
    g0.add([2], 0.5, "REPLACE")
    g1.add([2], 0.5, "REPLACE")

    obj.parent = arm
    mod = obj.modifiers.new("Armature", "ARMATURE")
    mod.object = arm
    return obj


def add_animation(arm):
    """b1 绕 Z（Blender 里是竖直轴）转 90°：导出成 Y-up 后正好是绕 Y 轴。"""
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.mode_set(mode="POSE")
    pb = arm.pose.bones["b1"]
    pb.rotation_mode = "XYZ"
    pb.rotation_euler = (0.0, 0.0, 0.0)
    pb.keyframe_insert("rotation_euler", frame=1)
    pb.rotation_euler = (0.0, 0.0, math.radians(90.0))
    pb.keyframe_insert("rotation_euler", frame=25)
    # 🔴 关键帧插值必须显式设成 LINEAR。
    # 不设的话这台 Blender 的默认是 CONSTANT ⇒ 导出成 glTF 的 **STEP**，
    # 于是"最后一帧那一刻"会突然跳到末值：实测 t=1.041 时 v2 位移 0.707，
    # 到 t=1.042（正好末帧）v1 突然位移 1.414 —— 看起来像解析器坏了，
    # 其实是夹具把动画做成了阶跃。判据要测"连续的旋转"，就得让它是连续的。
    # Blender 5.x 的 `Action` 没有 `fcurves`（分层动作），改走 slotted action 的 channelbag。
    action = arm.animation_data.action
    fcurves = []
    if hasattr(action, "fcurves"):
        fcurves = list(action.fcurves)
    else:
        for slot in getattr(action, "slots", []):
            layer = action.layers[0] if getattr(action, "layers", None) else None
            if layer is None:
                continue
            for strip in layer.strips:
                bag = strip.channelbag(slot)
                if bag is not None:
                    fcurves.extend(bag.fcurves)
    for fc in fcurves:
        for kp in fc.keyframe_points:
            kp.interpolation = "LINEAR"
    print(f"[fixture] 设为 LINEAR 的曲线数：{len(fcurves)}")
    bpy.ops.object.mode_set(mode="OBJECT")


arm = add_armature()
add_mesh(arm)
add_animation(arm)

bpy.ops.export_scene.gltf(
    filepath=out,
    export_format="GLB",
    export_yup=True,
    export_skins=True,
    export_animations=True,
    export_apply=False,
)
print(f"[fixture] 已写出 {out}")
