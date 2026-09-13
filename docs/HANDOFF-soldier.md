# 🔖 给"压缩后的我"的恢复note —— 士兵 GLB 接入

> **如果你是在上下文被压缩之后读到这行的：** 你正在做**把士兵 GLB 接进 NPC 渲染**这件事。
> 下面是你需要的全部状态，不用去猜。

## 已完成（都已提交推送，HEAD 会变，看 git log 找 `feat(soldier)`）

| 产物 | 说明 |
|---|---|
| `tools/blender/build_soldier.py` | 生成器，**四轮预览迭代**过 |
| `assets/soldier/soldier.glb` | **1082 顶点 / 540 三角形，1.840 m 高，min_z=0**（原点在底面中心 ✓） |
| `assets/soldier/` | **故意不放 `assets/props/`** —— 那里会被 `PropSet::load_dir` 扫到并当建筑加载 |

**模型已用眼睛看过四轮**（`build/_soldier/soldier_{0..3}_*.png`，该目录已 gitignore）：
有盔（喇叭形帽檐）、有脸（含护目镜条）、双手在枪上、腿分开、靴朝前。
**头/盔是六棱柱**（盒子颅骨在 3-5m 最像机械）。

## 待做：接入

**卡在哪（2026-09-13 已动手查实，不是推测）**：`renderer.rs` 的实例场走 mesh 着色器，
而 `build.rs::MeshOutput` 是：

```wgsl
struct MeshOutput {
    @builtin(vertex_count) vertex_count: u32,
    @builtin(primitive_count) primitive_count: u32,
    @builtin(vertices) vertices: array<VertexOutput, 50>,     // 每 workgroup 最多 50 顶点
    @builtin(primitives) primitives: array<MeshPrimitive, 96>, // 最多 96 图元
}
```
注释写明上限来自"NPC 四肢圆柱 50 顶点 / 96 三角形"。

**⇒ 每个 workgroup（=一个实例）最多 50 顶点 / 96 图元，而士兵是 1082 / 540 ——
结构上装不下。**（Vulkan 规范只保证 256/256，即使提到上限也装不下 540 三角形。）

**唯一可行的路**：**道具路径** —— 它的顶点是 CPU 侧烘好位姿的，没有这个上限。

### 我试到哪一步、为什么停

已经写过并**已回退**（保持 0 警告红线）：
- `MAX_DYNAMIC_SOLDIERS = 24` / `SOLDIER_VERTS_EACH = 1200` / `SOLDIER_INDICES_EACH = 1800` 三个常量；
- `soldier_mesh` / `soldier_dyn_first_vert` / `soldier_dyn_first_index` / `soldier_dyn_count` 字段；
- `frame_cam_pos`（由 `render()` 里 `view.inverse().w_axis.truncate()` 填）；
- `set_soldier_mesh()` 与 `write_dynamic_soldiers()` 的完整实现。

**剩下的（约 5 处协同改动，必须一次做完才有意义）**：
1. `prop_index_mapped: *mut u32` —— 索引缓冲也要持久映射（现在只映射了顶点）；
2. `soldier_dyn_index_span: u32` 字段；
3. `upload_props` 里把容量改成 `need + MAX_DYNAMIC_SOLDIERS × 每件上限`，并把
   `soldier_dyn_first_vert/index` 设成静态部分的末尾；
4. 在 `set_npc_visuals` 末尾调用 `write_dynamic_soldiers(visuals)`；
5. 在道具桶循环之后加**一次** `cmd_draw_indexed(soldier_dyn_index_span, 1, soldier_dyn_first_index, 0, PROP_INSTANCE_INDEX)`。

**⚠️ 最容易踩的一处**：`Vertex` 的字段映射是
`pos=[v0,v1,v2] / color=[v8,v9,v10] / uv=[v6,v7]`（`PropMesh.verts` 是 `[pos, normal, uv, color]`，
**不是** `[pos, normal, color, uv]`）。`upload_props` 里就是这么读的。

**⚠️ 顶点变换请复用 `merge_binned`，不要手写** —— 它已实现缩放/绕 Y 旋转/落地面/**绕序反转**，
手写一份必然走样，而绕序错了的面在本引擎里**直接黑掉且不报错**。

**必守纪律**：改前先跑冒烟确认基线 `VUID=0`；改后跑 `scripts/run_smoke_pm.ps1`；
`0 警告` 是硬红线（半成品留下的 unused 会直接破线）；**做不完就 `git checkout --` 回退，别留半成品**。


## 今晚已经付过代价的三条（别重犯）

1. **盒子 `scale` 是全尺寸**（段表注释写着「圆柱=半径；盒=宽/高/厚」）—— 我按"半宽"改了三轮，全错、全回退；
2. **给"人"烘 AO 的基准是身高（1.79）不是建筑层高（3.15）** —— 用错了裤腿会比上衣亮；
3. **"看起来变好了"不等于"数值变对了"** —— 观感改善可能来自别的因素。

## 复现命令（**路径必须绝对或正斜杠**，否则 Blender 会写到 `C:\build\`）

```powershell
$bl = "D:\3D_Work\blender\blender-5.2.1-windows-x64\blender.exe"
& $bl --background --python tools/blender/build_soldier.py -- "D:/Rust/steel-front/build/_soldier" soldier
& $bl --background --python tools/blender/preview_glb.py -- "D:/Rust/steel-front/build/_soldier/soldier.glb" "D:/Rust/steel-front/build/_soldier/soldier" 4
& $bl --background --python tools/blender/survey_props.py -- assets/soldier "*.glb"
```

