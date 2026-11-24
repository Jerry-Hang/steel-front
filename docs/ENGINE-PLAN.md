# 引擎化路线（2026-11-24 立）

> 用户决定：**游戏继续做，同时把底层做成引擎**。目标形态 = 像 Unity/Unreal 那样
> **可视化编辑**（拖拽/旋转物体、导入骨骼与动画、导入贴图、场景树与检视面板），
> 差异化 = **原生 AI 接口**：人用编辑器的时候，AI 可以通过 API/插件**同时**操作同一个场景，
> 并拿到**实时数据反馈**。
>
> 前提判断（用户原话）：渲染、物理、GLB 资产管线这几块最难的在做游戏时已经做完了，
> 现在缺的是**把它们组织成可编辑的东西** ⇒ 这是一次**工具化**，不是重写。

---

## 0. 铁律（本路线新增，不动既有铁律）

1. **不重写**。每一阶段结束，`steel-front` 必须仍是一局可玩、可通关的游戏（既有 953 条测试不许退化）。
2. **不新增第三方依赖**（既有红线）。⇒ 编辑器 UI 用仓库自己的 immediate-mode quad HUD（`ui.rs`），
   **不引入 egui/bevy 之类**。
3. **分层方向单向**：`core ← engine ← {editor, game} ← app / aipc`。
   反向引用一律判红（见 §3 的棘轮判据）。
4. **一切可被 AI 操作的东西，必须也是人能操作的**（同一个命令面）。不允许"只有 API 能做"的暗路。

---

## 1. 目标分层

```
core/     math / ids / time / cfg / log 包装        （不依赖任何本仓模块）
engine/   renderer  physics  assets  scene  ui      （只依赖 core）
editor/   场景树 检视面板 gizmo 拾取 存盘            （只依赖 core+engine）
game/     steel-front 的内容：城市/AI/波次/武器/目标   （只依赖 core+engine）
app/      winit 壳、帧循环、装配                      （可用全部）
aipc/     AI 控制面：命令 API + 遥测推送              （可用 core+engine+editor，**不许**碰 game 内部）
```

**当前实际形状（2026-11-24 实测）**：`src/engine` = 28,432 行 / 26 文件，
其中 `engine/renderer` 21,151 行、**`engine/game` 8,498 行（玩法就住在 engine 里）**；
另有 `weapons.rs`/`ai.rs`/`city.rs`/`objective.rs`/`map.rs` 这些**玩法文件散落在 `engine/` 顶层**。
`src/app_render.rs`（1138 行）与 `src/lib.rs` 直接驱动渲染器 ⇒ **游戏与引擎之间没有缝**。

---

## 2. 阶段与出口判据

每阶段的"出口"= 一条**能反向变红**的判据 + 四道闸门全绿 + 游戏仍可玩。

### P0 场景层（`engine::scene`）—— 本轮
- `EntityId`（含 generation，防悬空引用）、`Transform`、父子层级、级联删除、世界矩阵。
- 出口判据：`scene_transform_composes_through_parents`（手算值比对）、
  `scene_rejects_cycles`、`scene_despawn_cascades`。
- **为什么先做它**：编辑器拖拽 = 改 `Transform`；渲染 = 读 `Transform`；AI 操作 = 改 `Transform`。
  三者共用一张表，才谈得上"同一场景人机共编"。

### P1 渲染读场景（结束"渲染器直连玩法"）
- 引入 `SceneView`（实体 → draw item 的只读快照）；renderer 只吃 `SceneView`。
- 出口判据：**同帧像素一致** —— 用 `SceneView` 提交与今天的直连路径渲染同一机位，
  `png_diff` 差异 0；否则说明迁移漏了东西。

### P2 选择与 gizmo
- 射线拾取（复用 `physics` 的射线-AABB）、三轴平移/旋转/缩放 gizmo、拖动写回 `Transform`。
- 出口判据：模拟拖动 `dx=1.0m` ⇒ `Transform.translation` 增量 = 1.0（沿 gizmo 轴），误差 < 1e-4。

### P3 编辑器壳
- 场景树 / 检视面板 / 资产浏览器 / 存盘（TOML，复用 `map.rs` 的零依赖解析器）。
- 出口判据：`scene_roundtrip_toml_preserves_transforms`（含负零与极小值）。

### P4 资产导入（骨骼 / 动画 / 贴图）
- ⚠️ 现状：`props.rs` 只读 POSITION（顶点色已烘进去），**没有 JOINTS_0/WEIGHTS_0/动画采样**。
- 出口判据：导入一个带骨骼的 GLB 后，`skin_matrix × inverse_bind × joint_world` 与手算一致；
  动画采样在 `t=0/mid/end` 三点与关键帧数值一致。

### P5 AI 控制面（差异化重点）
- 本地 JSON API（NDJSON over TCP，零依赖；`llm_cmd.rs` 已有手写 HTTP 先例）：
  - 命令：`scene.spawn` / `scene.select` / `scene.set_transform` / `asset.import` / `scene.save` /
    `play.start|stop` / `editor.camera` / `pick.ray`
  - 遥测：`tel.frame`（fps/帧时间/阶段耗时）、`tel.scene`（实体数/选中/脏标记）、`tel.pick`
- 出口判据：脚本走一遍「spawn → set_transform → save → load → 逐字段相等」；
  **且同一条命令走编辑器 UI 与走 API 的可观测结果一致**（否则违反铁律 4）。

### P6 Play / Edit 分离
- 编辑器态暂停 `game/` 的逻辑推进，播放态才跑；两态共用同一 `Scene`。
- 出口判据：编辑态推进 600 帧后实体数/变换不变；播放态推进 600 帧后世界推进（且波次仍能刷出）。

---

## 3. 分层棘轮判据（现在就生效）

`src/engine/**`（除 `engine/game/**`）出现 `crate::engine::game` 或 `crate::game` 的引用 ⇒ 判红。
当前基线为 **0**，所以这条从第一天就是硬判据（不需要棘轮放宽）。

`core` 尚未拆出；等 `engine::scene` 稳定后再把 `math/ids/time` 提到 `core`，
到时补第二条判据：`core/**` 不得引用 `engine`。

---

## 4. 明确不做（避免范围失控）

- 不做可视化着色器图（材质仍走现有的 tint + 程序化皮肤 + GLB 顶点色三条路）。
- 不做 C# 级脚本系统；AI 接口 + TOML 数据就是扩展面。
- 不做动态库插件 ABI（Rust 无稳定 ABI）；"插件"= 同进程注册的系统 + 数据驱动内容。
- 不在本路线里改玩法数值/平衡（那是 `docs/PROGRESS.md` 的另一条线）。
