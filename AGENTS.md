# AGENTS.md — Steel Front 项目记忆与 AI 交接文档

> **体量约定（2026-09-12 加入，请遵守）** —— 本文件每次会话被**完整注入**，是稀缺资源。
> 它曾膨胀到 200KB：三分之二是重复段落、被推翻后没删的旧结论、WSL2 失效内容，
> 甚至出现过"同一条铁律的错误版与更正版并存"（见教训 1、2）。
>
> **只写仍然生效的约束**：铁律 / 未结案 / 教训 / 验收红线。
> 被推翻的**直接删掉**，不要留"错误 + 更正"两段；已结案的只留一行结论。
> **进度与时间线一律写进 `docs/PROGRESS.md`，不要写回本文件。**
> 目标 **< 48KB**；**硬上限 65,536 B**（超了会静默截断，比超标更危险）。
> 加之前先问："这条三个月后还成立吗、别人会不会照它做错事"。

---

## 📁 进度 / 日志 / 交接历史 → `docs/PROGRESS.md`

本文件**只放仍然生效的约束与注意事项**：铁律、未结案清单、教训清单、验收红线。

**每次开工前先读 `docs/PROGRESS.md`** —— 当前状态、历次迭代记录、交接规范与模板、
历史存档指针都在那里。**进度只写进那个文件，不要写回本文件**（理由见文首约定）。

---

## 项目

**21 世纪架空世界观的大战场 FPS**（⚠ 本文件与 README 旧版曾长期误写为「二战题材」，
据此建模/选材会全错。装备是现代系，HUD 默认武器 **AK-12 风暴 7.62×39mm**，2018 年列装）。
美术基调按 **2020s 当代东欧/中东战乱城镇**走（混凝土板楼 + 抹灰老城 + 破损，冷灰色调），
不是战壕与 1940 年代道具，也**不是**旧版写的"近代复古城市"。

Rust + Vulkan，纯 bin crate。**依赖只有 10 个**（`Cargo.toml`）：
`ash` 0.38 / `ash-window` 0.13 / `winit` 0.30(rwh_06) / `glam` 0.29 / `raw-window-handle` 0.6 /
`log` 0.4 / `env_logger` 0.11 / `image` 0.25 / `naga` 30(wgsl-in,spv-out,spv-in) / `rspirv` 0.11。
**不新增第三方依赖**是硬约束。工具链：rustc 1.96.1（2026-06-26）。

### 模块地图（`src/`，按体量）

| 文件 | 行数 | 职责 |
|---|---|---|
| `engine/renderer.rs` | 11605 | 地形 LOD + 65536 实例场 + HUD 覆盖层。**改 pipeline/shader/swapchain 风险最高，须先跑冒烟验 VUID** |
| `engine/game.rs` | 8123 | 运行时中枢：每帧 `update(dt, camera)` 编排物理/武器/AI/UI/音频/网络 |
| `main.rs` | 3853 | GameApp + winit 事件循环 + 输入/光标捕获 |
| `audio.rs` | 2747 | 合成音效与音乐（`audio_out.rs` 是 waveOut 输出层） |
| `ui.rs` | 2615 | HUD / 菜单 / 设置 / 键位表 |
| `engine/city.rs` | 2004 | 程序化城市生成 |
| `net.rs` | 1732 | UDP 联机（协议魔数 'S'） |
| `engine/cjk_glyphs.rs` | **1639** | 生成的中文点阵字模，**勿手改**。2026-09-14 由 21490 行/2.26 MB 裁到 166 KB（Noto Sans SC + 只留源码用到的 1595 码点）⇒ 旧记录写"21490 行"是过期的。🔴 清单 `tools/cjk_used_codepoints.txt` 会过期（加中文没重跑 `--scan`），测试 `source_cjk_codepoints_all_have_glyphs` 重扫 `src/` 兜底；它红 = 有人加了中文，命令 `python tools/extract_cjk_glyphs.py --scan`。🔴 源字体 `noto-sc-subset.otf` 未入库 ⇒ 表没法逐字节重建：**没有它就改写文案用已有的字，别拿别的字体顶替** —— 系统 `NotoSansSC-VF.ttf` 会改掉每个字形（`灭` 只剩 7 行），红 `cjk_glyph_generates` |
| `engine/ai.rs` / `weapons.rs` / `cpu.rs` / `map.rs` / `procedural.rs` / `physics.rs` | 1435 / 1431 / 1139 / 1124 / 1096 / 1054 | AI 分层与战术 / 武器系统 / CPU 拓扑与亲和 / TOML 关卡 / **程序化贴图 + 烘焙 AO/静态天光** / 物理 |
| `llm_cmd.rs` | 549 | RV3D_LLM 战术指挥通道（HTTP 出站，见下） |

其余：`config.rs`（`$HOME/.steel_front.cfg`，原子写 + 容错加载，测试不写盘）、
`engine/objective.rs`（据点/胜负）、`engine/ai_command.rs`、`engine/ray_tracer.rs`（PT）、
**`engine/lighting.rs`（纯运行时：方向光 / 点光源 / 阴影矩阵 / 镜面参数 —— **不含烘焙**）**、
`engine/assets.rs`、`engine/props.rs`（GLB）、`engine/meshgen.rs`、`engine/gpu_caps.rs`、`perf_log.rs`。

- 地形高度纯函数在 `renderer.rs`（`terrain_height` / `terrain_height_at`），**中央 60×60 压平 y=0**。
- `GameState`：`StartMenu` / `LoadingMap` / `Playing` / `GameOver` / `Victory(Team)` / `Defeat`。
- 关卡资产 `assets/maps/*.toml` 6 张：`index` / `street_fight` / `open_field` / `factory_ambush` /
  `bridgehead` / `defense_line`。手写零依赖 TOML 解析在 `map.rs`。

### 验收约束（硬红线）

`cargo test --release` 全绿、**0 警告**（dead-code=0）、**不新增第三方依赖**、
commit 规范 `feat/fix/docs/chore` + 范围前缀（如 `fix(input)`、`docs(AGENTS.md)`）、
**一个功能一个 commit，禁止 mega-commit**。**内存 12GB：一次只跑一个 cargo，禁止并行构建。**

---

## 开发环境

> **环境铁律（勿回退）**：开发/验证 = **Windows 原生**。2026-08-15 起从 WSL2 迁出，
> WSL2 相关材料**全部作废**。

- **机器**：RTX 5060 Laptop（NVIDIA 驱动 610.88）+ AMD 8940HX，内存 12GB。
- **编译**：`cargo build --release`。**测试**：`cargo test --release`（**0 警告**是硬红线；具体 passed 数见 `docs/PROGRESS.md`）。
  UDP 回环测试在沙箱内 bind 会 PermissionDenied → 需提权跑。
- **GPU 能力（原生实测，勿回退）**：`VK_EXT_mesh_shader=true`、光追 RT pipeline/AS/ray_query=true、
  DLSS VK_NVX=true、`present_us 101–373µs`。
- **分辨率**：默认 2560x1600（`C:\Users\Jerry-Huang\.steel_front.cfg`）。
- **功率**：奥创中心手动模式 + 电源最佳性能，GPU 功耗墙解锁 111.92W（默认 55W）。
- **git**：Windows 原生直接跑；`origin = https://github.com/Jerry-Hang/steel-front.git`，
  分支 `master`，作者 `Evernight <3520143257@qq.com>`，push 走 GitHub 令牌（仅限本仓库）。

---

## 铁律 A — 渲染技术路线

> 2026-08-16 决策，取代 2026-08-11 的「网格着色器冻结、传统管线主迭代」（那条**方向完全相反**，
> 已删除，不要再引用）。

- **主开发路径 = `VK_EXT_mesh_shader`（MESH + FRAGMENT）**。所有新渲染功能、性能优化、视觉迭代
  一律在 mesh 路径上做（`build.rs` 的 `MESH_SHADER_WGSL` + `renderer.rs` mesh 管线）。
  支持扩展的设备上自动启用，无需环境变量。
- **传统 VERTEX + FRAGMENT 管线冻结维护**：只作缺扩展时的兼容回退，**不再新增任何功能**；
  冒烟基线仍要求双路径 VUID=0。
- **mesh 路径约定（勿回退）**：
  - naga 30 网格写入器对 `@builtin(vertices)` 数组内 position 的 `ADJUST_COORDINATE_SPACE`
    翻转**失效** → mesh 着色器必须在 WGSL 内显式 `v.position.y = -v.position.y`
    （`build.rs`，删掉会垂直镜像）。
  - `maxMeshWorkGroupCount[0]` 最低保证 65535 ⇒ 地面场 65536 workgroup 必须按查询上限分块下发
    （字段 `mesh_max_wg_x`）。
- **`assets/*.spv` 是运行时从磁盘读的着色器**。改 WGSL 后必须重新构建，**勿手改 .spv**；
  看到它们变 dirty：先怀疑库里的是不是过期。

---

## 铁律 B — 渲染约定（勿回退）

**坐标与深度**
- Vulkan Y 翻转**只由 shader 负责**（`triangle.vert.spv` / `hud.vert.spv` 各翻 1 次）；
  `main.rs render()` **禁止**再翻投影（`proj.y_axis = -proj.y_axis` 曾致画面上下颠倒）。
  投影用 `glam::Mat4::perspective_rh`（y-up NDC、深度 [0,1]）。
- **阴影深度映射 = `frag_depth = sp.z`**（glam `ortho_rh` 已是 Vulkan [0,1] 深度），
  `lighting.rs::world_to_shadow_uv` 同步取 `(uv, p.z)`。
  🔴 **旧写法 `clip.z*0.5+0.5` 是错的（2026-08-31 推翻）**，把错误映射固化的回归测试也已删除，
  别再从旧记录里把它捡回来。
- 阴影其它约定：UV 必须 V 镜像 `uv.y = 1-(p.y*0.5+0.5)`；光方向语义 = **表面→光源**
  （`sun.direction` 直接传，勿加负号）；采样器 `.compare_enable(false)` 手动 PCF
  （comparison sampler 非 Dref 采样报 VUID）；**地形 identity 矩阵必须写到槽位
  `INSTANCE_COUNT`(65536)**，槽位 0 每帧被 `cull_and_upload` 覆盖；
  参数 2048² D32、半宽 250m、near=1/far=500、3×3 PCF、bias 0.005/0.02；`RV3D_NO_SHADOW=1` 做 A/B。
  排阴影问题先用 **`RV3D_DEBUG_SHADOW=1`**（R=frag_depth/G=阴影图深度均值），别再静态推矩阵。

**顶点格式与着色**
- `stride=32`，`pos@0 color@12 uv@24`，**没有法线槽位**：法线全部由屏幕空间导数重建
  → 只能纯平着色；AO / 烘焙光照**必须进顶点色**；**绕序反了的面直接黑掉且不报错**。
- 实例场与障碍立方体顶点色**全部白化**，颜色只走 tint；地形实例 tint=0.7 灰、
  marker tint=`WorldMarker.tint`（勿混）。
- `flat_flag`：槽位 ≥ 65601（`NPC_SLOT_BASE`）顶点着色器置 1，片元走纯色路径跳过贴图 50% 混合。
  改槽位常量须同步 `build.rs::NPC_INSTANCE_BASE` 与 `renderer.rs::NPC_SLOT_BASE`。
- `Shape::Authored`（`tint.w = 6.0`）→ `flat_flag = 1.25`，跳过四条程序化表面效果
  （`window_dark` / `glass_shade`+菲涅尔 / `is_canopy` 值噪声 / marker 混凝土皮肤）。
  **不接这条，GLB 立面会被再画一层错位窗带（D11 重演）**。
- **调试/材质开关**：`RV3D_PROC_TEX=0` 关程序化贴图（见铁律 D）、`RV3D_NO_SHADOW=1` 关阴影、
  `RV3D_DEBUG_SHADOW=1` 看 R=frag_depth / G=阴影图深度均值（见上）、`RV3D_SKIN_TEX=1` 开皮肤贴图
  （缺省 0 纯色回退，冒烟基线不变）、`RV3D_INSPECT=1` 检视模式（实例矩阵用 `Mat4::IDENTITY`）。
  `flat_flag` 材质编码 **0=地面 / 1=marker / 2=NPC**（binding 7/8）。

**地面**
- 专用平铺 quad（`GROUND_VERTS/INDICES`，4 顶点 6 索引）：绕序必须 `[0,2,1,0,3,2]` 反向才正面朝上；
  **正向绕序 → 整片地面消失只剩 clear color，这就是它的诊断信号**。
  地面实例矩阵纯平移 y=+0.05；地形下沉 `TERRAIN_RENDER_SINK=0.35` 防 z-fighting。
- 地面细节层：`GROUND_DETAIL_BINDING=9`，图像必须 `R8G8B8A8_UNORM`（线性、**非** SRGB）；
  纹素半值编码 `lum*0.5`；`GROUND_DETAIL_SIZE=256` 使 `2.0/256 == GROUND_DETAIL_TEXEL_M=0.0078125`；
  增益 `GROUND_DETAIL_GAIN=2.0`；`build.rs` 以 `light_data.flags.w >= 0.5` 门控并在 `g<=0` 退回 ×1。
  **改其中一侧必须同时看另一侧**，否则静默"地面变糊"。
- 地形程序化高度：`terrain_height` 中央半径 **140m** 内 y=0（含 60×60 安全区、障碍环带 58–130m、
  接火区），之外 ≤15m 确定性值噪声丘陵；`terrain_height_at` 同源。冒烟依赖的站定/接火区仍全平。

**管线与缓冲**
- 主管线 `depth_test_enable=true`；**枪模用独立 `gun_pipeline`**（`depth_test=OFF` **且不写深度**，
  否则会挡 HUD/粒子）。
- GLB 绕序与引擎约定相反，已在 `props::merge` 统一换面。外部建模的顶点变换（纯旋转+等比缩放+平移）
  本身不改绕序，索引交换是**刻意**的。
- **实例 buffer 元素数只允许 `INSTANCE_BUFFER_ELEMS = PROP_INSTANCE_INDEX + 1` 单一定义
  + 编译期 `const _: () = assert!(...)`**。三处（建 buffer、主管线 `descriptor.range()`、
  阴影 pass `descriptor.range()`）必须同源 —— 不同步 = **静默越界读**（驱动不崩不报 VUID、
  返回全零 → 几何全消失）。这是本项目最贵的一类 bug。
- 道具 buffer **固定容量一次分配**，绝不照抄枪模 `next_power_of_two` 按需重建
  （destroy 在飞 buffer → NVIDIA device-lost）。`PROP_INSTANCE_INDEX = GUN_INSTANCE_INDEX + 1`（83010）单槽。
- 道具要剔除 → 在**合并阶段按街区分桶**、每桶一次 draw call，不动实例系统
  （已落地 `merge_binned(cell=40m)`，实测 fps 112→152）。
- 验证层 `RV3D_VALIDATION=1`（默认关）—— 以前开它会因 mesh.spv 过不了严格 spirv-val 而**灰屏**
  （未结案 #9 的副作用），🔴 **2026-09-15 修掉根因后它才真的能跑**。**它是本仓最强的排障工具**：
  开起来第一轮就抓出两条一直存在、此前完全看不见的 VUID（见 #23）。
  **改 pipeline / swapchain / 同步 / 描述符前先开它跑一轮。**
- 改共享计算（如 `fp_gun_pre` 顶点/矩阵管线）必须**双模式**截图验证：第一人称 + `RV3D_INSPECT=1` 检视模式。
- 性能日志里的 `marker` / `npc` 字段 = 每帧 `upload_markers` / `upload_npcs` 的 (near+far) 计数。
- 🔴 **⚠️ 有两个同名的 `npc`，别混**（据此写下的错误结论已撤回）：
  - **HUD 左上那行的 `npc: I{} P{} C{} A{}`**（`game.rs:2640`）—— 是 NPC 的**状态人数**
    （Idle / Patrol / Chase / Attack），**与渲染、与箱体实例数毫无关系**。
    ⚠️ 那个 `I` 前缀很容易看漏：`npc: I1255 P0 C0 A0` 是"**1255 个 Idle**"，不是"npc=1255"。
    `RV3D_NPC_CAM` 下 AI 不步进 ⇒ 全员 Idle ⇒ 这个数会很大。
  - **perf 日志里的 `npc=`**（`renderer.rs:8604` = `last_npc_box_near + last_npc_box_far`）——
    这才是**箱体实例数**，用于判断"活体/尸体走的是 GLB 还是回退箱体"。
  **排查"某物到底有没有被画"先看这两个计数，再看图**；拿 HUD 那个数当实例数会得出完全错误的结论。

**玩家碰撞契约（2026-09-13 变更，勿退回旧写法）**
- 🔴 `PlayerBody::push_out_of_aabb` **现在带 Y 判据**：只有玩家的垂直区间
  `[pos.y, pos.y + eye_height]` 与 `Aabb` 的 `[min.y, max.y]` **相交**时才推挤。
  - 旧契约（**已废弃**）："墙的 y 范围不包含玩家 y，**但水平碰撞仍应生效**"（不管多高，
    只要 XZ 落进盒子就被水平推开）⇒ **直接后果是永远站不到任何东西上面**，正是用户报的
    "嵌进地板 / 穿进墙里"。
  - **曾经有 5 条测试把旧契约写死**（`player_y_untouched_by_collision` 的注释甚至
    明写"但水平碰撞仍应生效"）—— 改动时它们会红，那是**预期的**，要改测试而不是改回代码。
- 🔴 **站立面 = `max(terrain_height_at(x,z), PlayerBody::support_height(world, PLAYER_STEP_UP))`**。
  `support_height` 取所有**水平范围内（按 radius 外扩）且顶面不高于 `pos.y + step_up`**
  的盒子的顶面最大值；没有任何盒子时返回 `f32::NEG_INFINITY`。
  **这两条必须成对存在**：只加 Y 判据而不取支撑，玩家会直接穿进盒子里掉下去。
- `PLAYER_STEP_UP = 0.45` m：路缘/台阶能迈上去，护栏(1.5m)/集装箱(2.6m)必须跳。

**呈现模式（2026-09-13）**
- `RV3D_PRESENT_MODE` 支持 `immediate` / `fifo` / **`mailbox`**；引擎默认 **IMMEDIATE**。
- ⚠️ **IMMEDIATE 在真实显示器上是持续撕裂**（快速转视角时读成"残影/鬼影"）。**`PrintWindow`
  抓不到它** —— 它抓的是已合成的完整帧，撕裂只发生在显示器上。**别再用静态截图去证伪"残影"。**
- **玩家路径由 `SteelFront.bat` 设 `RV3D_PRESENT_MODE=mailbox`**（不撕裂、也不像 FIFO
  那样在独显直连下等不到 vblank 而死锁）。引擎默认 IMMEDIATE 是为了基准最稳。

**建筑摆放（2026-09-13）**
- `city.rs::pick_building` 的缩放是 **`min(w/gw, d/gd)`**（**不是 `max`**）。用 `max` 会按较大方向
  贴合、另一个方向**必然溢出 footprint** ⇒ 相邻楼互相穿插（用户截图的"乱窗/透视错误"）。
  **建筑模型本身没问题**，问题一直在摆放。
- 原注释担心的"无形的墙"由调用方把**碰撞盒设成真实视觉尺寸**来消掉
  （`building_at` 里按 `half_footprint × scale` 算，±90° 时 x/z 互换）。

**路径追踪（PT，默认关）**
- 默认关的最新理由 = "整帧替换光栅画面 + 1 spp 噪声大"，属调试/烘焙参照视图，**不是"命中没修"**。
  `RV3D_PT_LIVE=0` 强制关。
- 采样种子**必须含帧索引**（`frameSeed*64+b`）；push constants 6×vec4=96B，
  `PtParams::pack` 与 GLSL `PC{a..f}` 两处 `.size(96)` 必须同步；
  累积图像逐帧 barrier 用 `GENERAL→GENERAL`（用 `old_layout=UNDEFINED` = 累积白做，且不报 VUID）；
  `pt_frame >= pt_spp_target` 即停派发；`RV3D_PT_SPP` 覆盖目标（实时默认 256，`run_pt_view` 默认 64）。
- 时域累积/缓存的变化判定量化粒度**必须粗于相机 idle 抖动幅度**（现值：位置 ~0.5m、朝向 ~3°、
  光照 ~0.01；旧的 ~1mm 正是"PT 永不收敛"的根因，已作废）。
- RT 命中判据：`rayQueryGetIntersectionTypeEXT(q, 1)` 必须是 **committed**；
  **声称"RT 已验证"必须给命中着色的图像证据，不能只给 rays/s**。
- PT 着色器改 `assets/rt/pt_panorama.glsl` → glslangValidator → `.spv`，
  `spirv-val --target-env vulkan1.3` 严格通过；用 `scripts/compile_pt.ps1`（勿手工拼装 SPIR-V）。
- PT 盒面法线用不变量 `(primitive % 12) / 2` 查表（"来射方向主轴"近似会把地面法线判成 ±Z → 地面全黑）。
- PT 资产：`PT_MAX_BOXES=1024`（**由 512 提高**）一次分配；
  **BLAS 尺寸必须按容量上限而非当前盒数**（按 4 盒算 5376B 塞满容量 → 越界写 device lost）；
  scratch 归 `PtAssets` 所有；两次构建之间加 barrier。
- 🔴 **存储图像格式必须与 GLSL 声明逐位相等**：`pt_img` 对应 `rgba8` ⇒ 图像与 view 都必须是
  `R8G8B8A8_UNORM`。**"兼容"不算数** —— 不等就是**整张图写入未定义值**（不崩不报，只是画面发灰发脏）。
  建图前查 `optimal_tiling_features.STORAGE_IMAGE`（该能力对具体格式是**可选**的），不支持就返回 Err。
- 🔴 **PT 要 blit 进交换链 ⇒ `image_usage` 必须含 `TRANSFER_DST`**（缺它 = blit 与 barrier 两条 VUID）。
- 🔴 **PT 的 HUD overlay 必须用独立管线**（1 采样、无深度、`render_pass = hud_render_pass`）：
  主 pass 那条 `hud_pipeline` 是 MSAA 4x + 带深度的，绑上去 = `renderPass-02684`（UB）。
  该 render pass 的 `initialLayout` 必须是 **`COLOR_ATTACHMENT_OPTIMAL`**（调用方在 begin 前已手动转过），
  `finalLayout = PRESENT_SRC_KHR`，**别再补收尾 barrier**（会与 finalLayout 撞车 = `oldLayout-01197`）。

**弹孔（弹着标记，2026-09-15）**
- 链路：命中障碍 → `ImpactMark`（环形缓冲 192 / 寿命 30s / 末尾 3s 收缩，`game.rs`）
  → `main.rs` 转 WorldMarker 方片 → `Renderer::append_markers`（**单独一批、只进光栅列表、
  不喂 PT**：弹孔每枪都变，混进 `pt_set_scene_markers` 会让 BLAS 指纹每帧重建）。
  `RV3D_NO_DECALS=1` = 同机位 A/B 的对照组。
- 🔴 **弹孔贴的是「可见面」，不是碰撞 AABB 面。** `WorldMarker` 的实例缩放写的是 `2*half`，
  而模板几何是 **±1** 的立方体 / **单位圆柱**（r=1、y∈[−0.5,0.5]）⇒ **可见尺寸是碰撞盒的 2 倍**
  （圆柱竖直方向恰好 1 倍）。逐轴判据 = `geom::Shape::visual_half_gain`（带单元测试与实测数据）；
  `Shape::None` 的 GLB 碰撞核不绘制 ⇒ 1 倍。**贴碰撞面上 = 被可见几何整片盖住。**
- 🔴 **命中体取「参数 t 最小」的那个**，不是"`world.bodies` 里第一个命中的" ——
  那是**建关顺序**，远处障碍可能排在近处前面 ⇒ 弹孔画在被前面柱子挡住的那面墙上。
- 方片参数：8cm 见方 / 厚 1.6cm / 沿法线外移 0.4cm（**埋进墙里一半**：既不打 z-fighting，
  也看不出是浮在表面的板）；`tint.w` 取 **`Shape::Authored`** 走纯色路径 ——
  走 marker 皮肤路径会被叠一层窗带，看起来像"贴上去的小面板"。
- 一句话判据：**"墙上没有孔" ⇒ 先问"我贴的那个面，是不是被画出来的那个面"。**

---

## 铁律 C — 输入 / 键位 / 分辨率 / 鼠标安全

### ⭐ 鼠标安全协议（用户 2026-09-03 明确要求，2026-09-12 实测**证实其正确**）

> 引擎自己抓光标，抓取期间鼠标被锁 → 用户的机器会"像死机"。
> **按键用 `PostMessage` 投给游戏窗口句柄，不用 `SendInput`、不用 `SetForegroundWindow`。**
> **`SendInput` 喂的是前台窗口的输入队列**（实测 6/6 被系统接受、游戏一个没收到，前台不是游戏就是往别的程序打字）；
> `PostMessage` 直接进目标窗口队列。截图用 `PrintWindow`，不前置窗口。
> 每次启动游戏必须用 `cap_safe.ps1`（`finally` 里 taskkill + 硬超时）。
> 手动流程 = 启动后点一下游戏窗口 → 按 R 进入操控；死亡后再按一次 R 复活。
> `RV3D_AUTOSTART=1` **不跳过菜单**（旧版写它能跳过，是错的）；
> 要进游戏态请用 `RV3D_NPC_CAM`（它自己会调 `on_any_key()`）。

### 光标捕获与视角（`main.rs::sync_cursor` / `device_event` / `window_event`）

- `want = self.focused && state==Playing && !settings_open && !esc_menu_open`。
- **`focused` 初值必须是 `false`**（commit `49d994f`）。写成 `true` 会让 `want` 从第 1 帧就成立：
  窗口被别的程序占着前台时 winit 只在 `WM_SETFOCUS` 才发 `Focused(true)`，本进程既收不到 true
  也收不到 false，一直"自认为有焦点" ⇒ ClipCursor 把指针钉成 1×1。
  **用户 2026-09-03 报告的"鼠标死锁"就是这个，与插件/残留状态无关。**
- **捕获态的视角来源只有两条且互为唯一出口**：`device_event` 的 `DeviceEvent::MouseMotion`
  与 `window_event` 的 `CursorMoved`（`cursor_locked` 时直接 `return`）。
- 🔴 **`DeviceEvent::MouseMotion` 在 Windows 上不存在**：winit 0.30 的 Windows 后端只构造
  `Added` / `Removed`。**Windows 上绝不能用 `Locked`** —— 锁定即视角失效，且编译期/运行期都不报错。
  现由常量 `RAW_MOUSE_MOTION` + 纯函数 `cursor_grab_plan` 决定（commit `2bfd767`），
  Windows 走 `Confined` + 绝对位置路径（`grab=confined, look=absolute`）。
- **非捕获态有拖拽转视角路径**（`dragging` 由左键按下置位）。该路径每个事件后把真实光标
  warp 回窗口中心并把 `last_cursor` 设为该中心。
- `MAX_LOOK_DELTA_PX = 512`（绝对位置单次位移上限，超过视为光标传送，跳过并重基准）；
  `MAX_RAW_LOOK_DELTA = 1024`（raw 单事件上限）。

### ⭐ 无焦点视角注入配方（实测标定，误差 0.3%）

`scripts/pm_play.ps1` 已实现下面四条，**照抄即可，别再重推**。缺任何一条都会静默失效：

1. **必须用 `PostMessage`，不能用 `SendInput`。**
2. **每一步前重新按下左键**（`WM_LBUTTONDOWN`），**紧接着**投移动，两条背靠背。
   否则 post 的第一次移动让 winit 调 `TrackMouseEvent`，真实光标不在窗口上 ⇒ 立刻 `WM_MOUSELEAVE`
   → `CursorLeft` → `main.rs` 把 `dragging` 置回 false：**移动照样到达并进日志，但视角被跳过。**
3. **每步偏移必须是"窗口中心 + step"，不能累加**（拖拽路径每事件后把 `last_cursor` 重设回中心，
   增量恒为 `posted - centre`）。步长 400，**连续两次坐标差 1px**，否则被 winit 位置去重丢掉。
4. **步间隔必须 > 150ms（用 300ms）**：`recenter_pending_until` 的 150ms 窗口会吞掉 `CursorMoved`。

- **实测标定**：1200px（3×400）→ **-170.30°**，模型预测 -170.79°，误差 0.3%。
  模型 = `yaw -= dx * (0.0005 + sensitivity*0.002)`；**`cam:` 日志的 yaw/pitch 单位是度**（内部弧度）。
- **`SetCursorPos` 驱动真实光标走不通**：游戏收到 **0 个** `CursorMoved`（posting `WM_LBUTTONDOWN`
  不会让 winit 调 `SetCapture`）。

### 键位与方向
- **键码一律用 winit 0.30 `KeyCode` 枚举序号**（KeyW=41 / KeyS=37 / KeyA=19 / KeyD=22 /
  KeyR=36 / Space=62 / ContextMenu=54 / Escape=114），**不是 USB HID 码**；**升级 winit 先跑**
  `ui.rs::winit_keycode_indices_match_table`（它锁死这张表）。
  🔴 **注入脚本里有另一套 Windows 虚拟键码 VK**（只有它该进 `PostMessage(WM_KEYDOWN)` 的 `wParam`；
  `R=82(0x52) C=67 Z=90 W=87 A=65 S=83 D=68 Space=32 Tab=9 Esc=27 Shift=16`）。混用 =
  「发出去了但游戏零响应」：把 winit 的 `KeyR=36` 当 VK 发，游戏收到的是 **`VK_HOME`**。
  🔴 `lParam` 的 **bit16-23 必须是扫描码**（`MapVirtualKey(vk, 0)`）；`cap_safe.ps1` 的 `-Keys`
  必须收 **VK 码**，且窗口句柄只能 `FindWindowW(None, "Steel Front - Vulkan")` + 轮询
  （`Process.MainWindowHandle` 对 winit 拿到的不是接收输入的那个窗口）；
  **`pm_play.ps1` / `gameplay_smoke_pm.py` 是正确的参照实现**。
- `config.rs bindings_version=1`：旧版 HID 键码配置整体忽略回退默认（**勿删迁移逻辑**）。
- **鼠标水平** `look()` → `yaw -= dx*sens`（右移 = 右转）；**垂直** `pitch += dy*sens`
  （winit Y 向下 ⇒ dy>0 = 低头），后坐力 `pitch -= recoil_pitch*dt`。**勿再翻 pitch。**
- **灵敏度**：`game.rs::sensitivity_rads() = 0.0005 + hud.sensitivity * 0.002`
  （默认 0.5 → 0.0015 rad/px），`main.rs` 每帧 `set_mouse_sens` 同步；
  `camera.rs::set_mouse_sens` 夹在 `[0.0005, 0.02]`。**勿改回 0.003 起步。**
- 注入脚本必须**从 `~/.steel_front.cfg` 读真实灵敏度**，不要写死。
- 死亡重开：R 或 Enter。保留键 ESC / TAB / ENTER / F12 / Q / E / N 不可重绑；
  `Tab`（VK 9）在玩法态**不切视角**，别依赖它做环绕取证。
- 🔴 **调试相机取证必须带 `RV3D_NO_NPC_CULL=1`**：`npc_occluded()` 是**以「玩家眼位」为中心**的剔除，
  正常玩法里玩家就是相机 ⇒ 正确；但 `RV3D_CAM` / `RV3D_NPC_CAM` 把相机移开时**玩家仍在原点** ⇒
  **相机眼前的人被判为"从玩家位置看不到" ⇒ 剔除**（实测 `npc` 288→4590）。
  **这正是"看不到士兵"的真正原因**，不是模型/遮挡/取景。
- 🔴 **`RV3D_CAM` 的朝向与 pitch 必须先标定再用来做对照实验**：
  `forward = (-sin(yaw), 0, -cos(yaw))` ⇒ **yaw=90° 面向 −X，不是 +X**；
  **负 pitch = 抬头**（`-84` 拍到天空），俯看用**正**值（`:0,84`），平视 `:0,0`。
  **判据：用任何角度参数前，先花一次 run 拿一个已知会在/不在视野里的物体验一次朝向**，
  别信"yaw=90 应该是往右"这种直觉 —— **规则写了不执行等于没写**。

### 分辨率
`RESOLUTIONS` 5 档含 2560x1600；显式配置非预设分辨率会回退首项（旧坑 2560x1600→1280x720）；
默认按主显示器宽高比：16:10 → 1280x800，16:9 及其它 → 1280x720（**仅首次运行生效**）。

---

## 铁律 D — 建模与资产

- 建模一律 **headless** `--background --python`，**不要自动化 Blender GUI**（抢焦点/鼠标，违反鼠标安全协议）。
  Blender = `D:\3D_Work\blender\blender-5.2.1-windows-x64\blender.exe`（5.2.1 LTS，自带 Python 3.13；
  系统另有 python 3.11.9）。探针 `tools/glb_survey.py` / `tools/glb_probe.py`。
- **单位约定（勿改）**：1 单位 = 1 米；原点在**底面中心**（z=0，其余 z≥0）；Blender +Z 上；
  导出 `export_yup=True`；单 mesh / 单 primitive / 节点不带变换。
- Blender 5.2 导出器：`export_vertex_color` 是 **ENUM**，用 `"NAME"` + `export_vertex_color_name="Col"`；
  `COLOR_0` 是 VEC4+u16，接引擎必须解决格式不匹配。
- **枪械资产朝向已规范化并烘进顶点**：muzzle **+Z** / up **+Y** / right **+X**、bbox 居中、
  最长边 = 1.0。`main.rs` 的 `scale = 1.35/longest` 因此恒等于 1.35。最长轴朝向启发式对全部
  14 件落 `IDENTITY` —— **绝不能反过来依赖它"修正"朝向**。
- 枪械色彩空间：`Image.pixels` 返回**原始 sRGB 编码值**（脚本自己做 sRGB→linear）；
  `baseColorFactor` 已是线性直接用；贴图 socket 的 0.8 默认值**刻意忽略**。
- 枪模 buffer **只增不减**：容量 = `max(32768, next_pow2(顶点))` / `max(262144, next_pow2(索引))`。
  🔴 **2026-09-15 实测更新**：AK-12M 的 GLB 有 **63283 顶点（容量 65536 = 96.6%）**、70479 索引 ——
  旧文档写的"峰值 11949/32768 = 36%"是**过期数字**。换更大的枪会触发一次扩容重建 ⇒
  **扩容前必须 `device_wait_idle()`**；而**换成更小的枪绝不允许重建**（destroy 在飞 buffer =
  device lost：2026-09-15 按一下 "2" 就把整台设备打掉）。回归测试 `gun_glb_indices_all_in_range`
  逐把校验 GLB 索引范围（越界索引 = GPU 顶点抓取越界 = 设备消失，同样不报 VUID）。
- 程序化贴图（勿回退）：写 `R8G8B8A8_SRGB` 前必须 linear→sRGB 编码；材质分域用**世界尺度**
  value_noise（biome ~120m、detail ~10m），旧 `fbm(x*0.025)`（2560m）会致整图单色；
  `RV3D_PROC_TEX=0` 回退做 A/B；`MARKER_INSTANCE_BASE=65537`；片元用 world-space UV。
- `SteelFront.bat` 的 touch 列表**必须含 `build.rs` 与 `build_spv_rt.rs`**
  （只改着色器/构建脚本时 cargo 会静默不重编 = 启动旧版本）。

### ⭐ 设计化建模链路（2026-09-12 建立，取代 `gen_props.py::asset_building`）

> 旧路线把建筑写成"参数拼箱子"，生不出品味。现有 6 个建筑模块由
> `tools/blender/build_city_kit.py` 生成，比例是**照着真实板楼/抹灰楼写死**的，不是尺寸区间。

```powershell
# 1) 尺寸普查：24 件资产的占地/高度/底面 —— **尺寸契约的唯一来源**
blender.exe --background --python tools/blender/survey_props.py -- assets/props "*.glb"
# 2) 生成（可只给名字）：输出到临时目录，确认后才覆盖 assets/props/
blender.exe --background --python tools/blender/build_city_kit.py -- <out_dir> [name...]
# 3) 预览渲图：4 视图 → PNG，**我必须用眼睛看过再入库**
blender.exe --background --python tools/blender/preview_glb.py -- <in.glb> <out_prefix> [n]
```

- **引擎只给了三条视觉杠杆，别指望第四条**：没有法线槽位 ⇒ **纯平着色**，**细节只能是真几何**；
  AO/明暗**必须烘进顶点色**；道具 `export_materials="NONE"`，外观**全部**来自顶点色。
- **尺寸契约是硬的**：`city.rs` 按**占地**摆放（`building_tall` 有 **52 处**）。资产越出契约轮廓
  就会插进邻居 ⇒ **外挑一律改内凹**（凹阳台 loggia 既是东欧板楼真做法，又不占轮廓）；
  允许的小外挑上限 = **0.06m/侧**（勒脚/窗台）。
- **层高反解**：总高固定，`上层 = 3.15`（**等于引擎 `FLOOR_H`**），**底层吸收余量**（现值 3.56），
  **别再写死**。
- **绕序交给叉积判定**：`add_quad_n()` 收"意图法线"，算叉积、反了就翻。引擎侧反的面
  **直接黑掉且不报错**，靠手推绕序是一类静默事故。
- ⚠ **预览图只对"几何与比例"可信，对"最终颜色"不可信**：预览走 Blender 自己的光照 +
  AgX 视图变换。**颜色判断必须在引擎里做**（`RV3D_CAM` 固定机位取证）。
- **顶点预算**：`props: 缓冲扩容 顶点 N/2097152` ⇒ **硬容量 2^21 = 2,097,152**，加细节前先看这个数。
- 🔴🔴 **道具/GLB 的两条通则（改动前先读）**：
  1. **`box_project_uv` 的逐面 UV 岛 + `export_normals=True` 的逐面法线都会阻止顶点共享**
     （`tree_oak` 838 三角形占 **2264 顶点**；全城 372 棵 = 整个道具缓冲的 54%）。
  2. **引擎根本不需要这两样**（无正常线槽位；`Shape::Authored` 在片元里**跳过**四条会采样 UV 的
     效果；`assets.rs` 只把 `POSITION` 当硬要求）。
  ⇒ **改法**：`tools/blender/weld_props.py`（按 pos+color 去重、UV 设常量、**`export_normals=False`**）。
  **`export_normals` 是总开关**：开着它时焊到 458 顶点、落盘又变回 2264（导出器按面拆分），只降 6.6%。
  **收益（同机位实测）**：顶点 1563020→509616、显存 72→24 MB、每帧提交 476100→153363、
  **中位帧率 +18.6%**，**三角形一个没少、画面无退化**。
  **本仓是顶点瓶颈**（像素少 4 倍只 +12%；一次画完反而 −38%）⇒ 顶点数就是帧率，这是最高杠杆的一处。
- **确定性**：`hash(str)` 每个进程都变（PYTHONHASHSEED），生成器里用 `zlib.crc32`。
- 同型号建筑的"克隆军团"由 **`props.rs::placement_tint`** 治（逐摆放确定性色调 ±12%），
  不是靠堆更多型号。

---

## 铁律 E — 平台 / 指令集 / 线程

- **不做原生 Metal 后端**（macOS/iOS 走 MoltenVK，零改动）。
- AArch64 NEON `cull_spheres_neon`（4 实例/批、非 FMA、与标量**逐位一致**）；
  ash 字符串指针统一 `RawCString`；`sched_setaffinity` 仅 `target_os="linux"` 编译；
  交叉验证 `cargo check --target aarch64-unknown-linux-gnu`。
- **AVX-512**：AMD Zen4/Zen5 启用；Intel 11 代默认关（能效/降频负收益）；
  Intel 12 代起**防御性关闭**（出厂熔丝禁用，防虚拟化透传）；`RV3D_DISABLE_AVX512=1` 强制关。
- **线程红线**：攻击态/接火 NPC **必须每帧步进**，降频仅限无感知的非攻击 NPC
  （`AI_FAR_DECIMATE=4`，`RV3D_AI_DECIMATE=off` 关闭）。渲染**不拆线程**（仍=主线程）。
  **不可写死 SMT 奇偶**：运行时读 sysfs 每对取最小 vCPU，不可读时回退旧行为。
- 硬件门槛：最低 3300X + RX 6500 XT（RDNA2，mesh shader 起点）4C8T、内存最低 8GB / 推荐 12GB+；
  推荐 8C16T 中端独显；最高 16C32T + RTX 40/50。详见 `docs/hardware-requirements-2026-08-11.md`。

---

## 铁律 F — 协作 / 上下文 / 工具

- **并行分身**：文件集两两不相交；**分身禁止 cargo**（12GB 只允许一个）**与 git**；
  `renderer.rs` 上万行**禁止整文件重写**，只许精确 edit；跨文件接口由主 Agent 定义；
  开工前先把在飞改动 commit 成干净基线。
- **上下文节约**：非必要不读编译产物（`target/`、`Cargo.lock`、`*.spv`、`*.rlib`）；
  非必要不反汇编（要看 .spv 时 `spirv-dis` 输出到临时文件再 grep）；
  非必要不反复读同一文件；**大文件先 rg 定位再限定行号读**；`git diff` 一律 `--stat` 或限定文件。
  ⚠ 本 shell 里 `Get-Content` 数行数不准（实测 2826 vs 实际 3229），**行号以 `read` 工具为准**。
- 🔴 **`cargo check` 不能替代 0 警告闸门**：`check` 与 `build` 的 fingerprint 不同，
  **`check` 会重放它自己缓存下来的旧诊断** —— 实测 `cargo check --release` 报 28 条 `never used`，
  而同一次 `git checkout` 之后 `cargo build --release` 是 0 警告。
  **判据：`0 警告` 只能用 `cargo build --release`（或 `cargo test --release`）验。**
  ⚠️ 实验占着 exe 时 `build` 会卡在**链接**（`failed to remove …exe`），
  但**编译与警告在此之前就已产出** ⇒ 看警告仍然有效，别把那个 error 当成编译失败。
- 🔴 **`> file` 重定向会写成 UTF-16**（实测 102552 B 的真实文件写成 184852 B）：
  要取 HEAD 版本做字节比对，用 `git checkout-index` / `git cat-file` 写二进制，或用
  `git diff` / `git status` 判断，**不要用 PowerShell 的 `>`**（教训 7 的另一面）。
- 🔴 **脚本里取备份必须取"未改动的"那一份**：按 `Copy-Item $path $backup` 取备份而脚本跑了两次 ⇒
  **第二次取的备份已是剥过的版本**，失败回滚时把坏文件写回去。
  取备份要么从 `git show HEAD:`，要么在脚本开头判断"若备份已存在则复用它"，
  并且**回滚用 `git checkout --`**（教训 30）。
- **看到"规划中"的 dead code，必须回答"那它为什么没被接线"，不许加 `#[allow]` 了事。**
  现状：**全仓 `#[allow(dead_code)]` 共 109 处**（`audio.rs`(38) / `weapons.rs`(15) /
  `lighting.rs`(11) / `build.rs`(9) / `renderer.rs`(8)），**绝大多数是有注释的"预留接口"**，
  属诚实预留不是藏问题。
  🔴 **要清陈旧压制，判据只能是编译器，不能是文本匹配**：删掉 `#[allow]` 后若仍 0 警告即为陈旧。
  按"符号名出现次数"判定的脚本会把 `new`/`get`/`update` 全报成陈旧（**名字匹配分不清
  "这个符号被用了"与"同名的东西到处都是"**），已弃用。
- 🔴 **未结案条目会过期，而"过期的待办"和"错的结论"一样有害**：
  **⇒ 判据：动某条未结案之前，先用 `rg` 确认它引用的符号/文件**还在**。**
  **⇒ 反过来：结案一条时必须把条目本身改掉或删掉** —— 只不再提的话，下一个人还会照着旧条目去找。
- 🔴 **核对"文档里的常量值 vs 源码"时，别写正则**（真实写法 `pub const PT_MAX_BOXES: usize = 1024;`
  中间夹着类型标注和一个额外的 `=`，字符类正则到不了那个数字）。
  **⇒ 判据：核对定义就用 `rg --fixed-strings "const $NAME"` 把整行打出来用眼睛看。**
  这一类"我的匹配模式比数据复杂"的坑一晚踩了四次（符号名频次 / 字面路径 vs 基名 / 上述正则 /
  行号跨版本比对），**同一形态：先有结论，再写一个刚好能"证明"它的工具。**
- **阈值纪律**：冒烟 `fps_min` 越线先判是不是**首帧窗口**（判据 = 仅首样本越线 +
  `npc` 计数远低于稳态 + `wait_fence ≈ frame`，SPIR-V 重生成后驱动 JIT 冷缓存），
  重跑确认 —— **别改测试、别调阈值**。
- **验收口径**：`run_smoke_pm.ps1` → `gameplay_smoke_pm.py`，判据 = **`vuid==0 and panics==0 and killed>=1`**，
  **无 fps 门槛**（`fps=` 只用于打印）。旧版误把 `fps>=120` 写进这里 —— 那是**旧 SendInput 版
  `gameplay_smoke.py`** 的规则，而本文件又写着"别用旧脚本"。**两个脚本的口径别混。**
  `playtest_perf.py` 是**时长制**：跑满 `PT_SECS`（默认 600s）即完成，击杀是附带指标、不设门槛、不判 FAIL。
- 微基准：`cargo test --release <名> -- --nocapture --test-threads=1`
  （`shockwave_path_microbench` / `simd_cull_microbench`）；
  `RV3D_FORCE_SIMD=avx512|avx2|avx|sse4.2|scalar`（硬件不支持时告警回退）。
- `cargo clippy --fix` 在本仓**不可用**（build.rs 代码生成缓存行为，反复提示却不改字节），别再试。

### 常用命令

```powershell
cargo build --release
cargo test --release
# 游戏冒烟（**用这个**；PostMessage 注入，实测 ALL-OK：命中 + 击杀 + VUID=0）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_smoke_pm.ps1
# 旧的 run_gameplay_smoke.ps1 走 SendInput，在本机结构性跑不通（见铁律 C），别用它判断回归
# LLM 战术指挥通道会战（红蓝 128v128，由服务端下命令；实测 14 条命令全被采纳）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_llm_battle.ps1 -Secs 150 -Interval 20
# 截图取证（finally 里 taskkill + 硬超时）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\cap_safe.ps1 -Tag orbit -WarmupSec 8 -HoldSec 2 -Keys 9 -AfterKeysSec 3
# 多键必须走 -Command，-File 会把 9,9 合并成一个 "9,9"
powershell -NoProfile -Command "& 'scripts\cap_safe.ps1' -Keys 9,9"
# 无焦点接管一局（PostMessage 注入：不抢前台、不抓光标、不锁指针）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\pm_play.ps1 -Tag demo1 -TurnPx 1200 -WalkMs 1500
# 输入路由诊断（报游戏线程焦点，并把同一按键用 SendInput / PostMessage 各投一次）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\input_probe.ps1
# 输入归还校验（杀进程 + 解除 ClipCursor + 复核后给 OK/FAIL）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release_input.ps1
# 心跳看门狗：**常驻**后台即可，不要每次运行临时 arm 一个（见教训 19）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\play_watchdog.ps1 -StaleSec 30
# 性能尺子（Windows 原生；压力模式跑 N 秒，读 logs/perf_*.log 出统计，退出时交还机器）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\perf_run.ps1 -Secs 30
# 画面差分（第一道筛子）：同机位 A/B 的差异像素占比 + **差异包围盒**
# —— 没有包围盒，几百个差异像素既可能是"引擎坏了"也可能是"HUD 上的 FPS 数字变了"
python scripts\png_diff.py screenshots\a.png screenshots\b.png
```

⚠ `cap_safe` / 截图脚本的游戏日志是 **`logs/<tag>.log.err`**（stdout 的 `.log` 常为空文件）。
⚠ `scripts/play_cap.ps1` 已于 2026-09-12 删除（它用 SetCursorPos + mouse_event 拿到焦点后不做真正的
释放，正是用户 2026-09-03 报告的鼠标死锁那一类行为；已由 cap_safe.ps1 + pm_play.ps1 +
release_input.ps1 取代）。
---

## 🔴 未结案清单

> 按"值不值得下一轮动手"排序。每条给出当前最优线索（lead），**没有 lead 的不要瞎猜。**

### 红蓝阵营不对称：**20 轮/臂后判定为噪声主导，效应未获确证**（2026-09-14 结案）

`scripts/ab_reps.ps1 -Reps 20`（40 局 × 120 s），`blue margin = 红损 − 蓝损`（正 = 蓝优）：
baseline（红 +X / 蓝 −X）均值 **+12.8**、极差 29；swapped（红 −X / 蓝 +X）均值 **−10.3**、极差 **54**。
**三层读法**：① 两臂均值分居零两侧且与 8 轮那次（+16.5 / −6.3）同向 ⇒ 可能真有个 ~20 点的位置倾向；
② 但臂内极差均值 41.5 **大于臂间差 23.1**；③ ⇒ **任何"单次对撞谁赢"都不构成证据**（教训 24/27）。
**"红方恒胜"与"−X 半场占优"两个说法都撤回。**
四条静态测量**全部对称**（掩体 0.999 / 出生环被挡 39.1% vs 39.4% / 角度差 0.6° / 半径两侧同为 174.00 m）
⇒ 可划掉掩体分布、出生点可站立性、出生几何配对、角色分配。**要继续先解决"测不准"**（每局方差 ~±30 点）。

- 相关：军情 JSON 的 `击杀` 字段实际是**该营自身阵亡数**（`round_kills_*` 按阵亡者阵营计数）。
  若 `llm_commander.py` 当"我方战果"读则**信号是反的** —— 调参前先核对。

### ⚠️ 障碍 marker 的**可见尺寸 = 碰撞盒的 2 倍**（2026-09-15 实测，**未修**）

`WorldMarker::for_obstacle` 的实例缩放写的是 `2*half`，而模板几何是 **±1** 的单位立方体
（`renderer.rs::VERTICES` / `build.rs::CUBE_POS`）与**单位圆柱**（r=1、y 已是 ±0.5）
⇒ 画出来的障碍是它的碰撞 AABB 的 **2 倍**（圆柱的竖直方向恰好 1 倍）。
两条渲染路径都是同一结果（CPU 顶点几何与 mesh 模板都是 ±1），所以**不是某条路径的 bug，
而是"缩放约定"与"模板单位"不一致**，且已存在很久。

- **实测判据**（不是读代码推的）：近处那根 `Block @(0,1.5,−11.8) 尺寸=0.22×0.22×0.70 shape=Cylinder`
  在画面上的**宽/高比 = 0.61**；半径 0.22（2×）预测 0.44/0.70 = 0.63 ✓，半径 0.11（1×）预测 0.31 ✗。
- **影响面**：城市里可见的 marker 多是栏杆柱/树干/长凳这类小件 ⇒ 只是"比碰撞粗一倍"，
  玩家仍被挡在可见面之外（碰撞半宽 + 玩家半径 > 可见半宽），**所以一直没人当成 bug**。
  ⚠️ 但 **TOML 关卡**（`RV3D_MAP=...`）的墙/掩体走 `Shape::Legacy` 立方体 ⇒ 会被画成 2 倍大，
  玩家能站进"看得见的那半个盒子"里 —— 那几张图上是**真会穿帮**的。
- **为什么没顺手修**：改成一致（模板 ±0.5 或缩放改 `half`）会让**全城所有 marker 缩小一半**，
  属全局观感改动，得先给用户看过；而且 NPC 四肢与 marker **共用同一套单位圆柱模板**
  （NPC 的实例矩阵是按 r=1 建的）⇒ 改模板会连带改士兵体型。
- **临时对策**：任何需要"可见面"的地方（如弹孔）一律走 `geom::Shape::visual_half_gain` 取真实倍率，
  **不要各自再写一遍 2.0**。

1. ~~**`PrintWindow` 对非前台窗口返回冻结帧**~~ **已结案（2026-09-14）：症状不复现**。`cap_safe.ps1` 用 `PrintWindow(h, dc, 2)` = **`PW_RENDERFULLCONTENT`**（Vulkan 窗口靠它才抓得到活画面，**别把 2 改成 0**）；复测判据 = 同刻两张图差异像素 **1.36%**（冻结会是 0.00%）。
2. ~~**PT 崩溃 `0xC0000005`**~~ **已结案（2026-09-15）：四个真 bug 全修，PT 通路的验证层问题当日清零**（今只剩 #23 层侧误报）：①交换链缺 `TRANSFER_DST`；②`hud_framebuffers` 只在启动建一次、而启动有 **5 次**交换链重建（症状"光栅正常、一开 PT 就崩"）；③`pt_img` 格式与 GLSL `rgba8` 不等；④overlay pass 复用主 pass 的 MSAA+深度管线。**判据见铁律 B 的 PT 段**。
3. ~~**`config.rs` 不读 `pt_enable` / `rt_enable`**~~ **原结案是错的，2026-09-15 重开并真修**：`load_from` 无 arm、`save_to` 不写 ⇒ 两字段只能是源码默认值，配置文件/设置面板根本开不了 PT。现补读写 + `parse_bool`（`1/0`/`true/false`，非法值保持默认）。🔴 **结案要看完整条链路（写→读→用）；"字段存在"≠"接线完成"**（见铁律 F 未结案条目段）。
4. ~~**玩家可能站在 GLB 楼体内部**~~ **已结案（2026-09-13）**：`pick_building` 的 `max` → **`min`**（判据见铁律 B「建筑摆放」）。
5. ~~**`FLOOR_H` 常量分叉**~~ **已结案（2026-09-12）**：6 个模块「上层 3.15 + 底层反解 + 女儿墙/压顶」，实测 6/6 命中（见铁律 D 层高反解）。
6. **`svd_63` 未入库** — 源文件是含两把相差 90° 重叠枪身 + 独立瞄具的产品宣传图，
   `install_guns.py` 仍 SKIP。需人工删掉重叠枪身后装为 `svd12`。
7. ~~**D12 士兵近距观感**~~ **已结案（2026-09-13）：士兵由 `assets/soldier/soldier.glb`（1082 顶点/540 三角形）经 `cmd_draw_indexed(…, SOLDIER_INSTANCE_BASE)` + `self.pipeline` 实例化绘制（不需新管线）**；🔴 **阵营色 = 队色 × `tint.w = 6.0`**（不接会整身涂成一队色）。**仍缺**骨骼动画（现为整体起伏）与两套队色顶点变体，详见 `docs/HANDOFF-soldier.md`。
8. **D4 墙缝天空亮条 / 悬浮亮条** — **lead**：疑似楼间缝隙的正常天空，需定点复现再定。
9. ~~**mesh 着色器过不了严格 `spirv-val`**~~ **已结案（2026-09-15）**：根因 = naga-30 `decorate_struct_member` 无条件写 `Offset`，而 SPIR-V ≥1.4 禁止对非 Block 类型写它、mesh 又必须用 1.4；现由 **`build.rs::strip_workgroup_explicit_layout`** 去掉，🔴 **只去掉 Workgroup 可达类型**（带 `Block` 的 `_struct_300/303/308` 动一个字节即缓冲错位）。**7 个 `.spv` 全 exit 0**；`mesh_spirv_has_no_workgroup_explicit_layout` / `block_types_keep_their_offsets` 两条测试锁住两个方向（都会红）。
10. ~~**PT 512 盒上限静默截断**~~ **已结案（2026-09-14）：512 → 1024 一次分配 + 一次性告警**（`Renderer::pt_box_cap_warned` 有闩）；代价仅 **0.92 → 1.84 MB**（余量 87%）。
11. **PT 与光栅同屏叠加未做**（现为整体替换）；移动相机每次全量重开累积。
    **lead**：按像素重投影复用，或运动自适应 spp。相关：`signature()` 量化已改分层
    （位置 ~0.5m / 朝向 ~3° / 光照 ~0.01），**勿回退到 1mm**。
12. ~~**`MAX_RIGID_BODIES` vs `MAX_AI`** 溢出静默丢弃~~ **已结案（2026-09-14）——两个常量都已不存在**（刚体表已是 `pub bodies: Vec<Body>`）；但「静默丢弃」这个**模式**仍值得防，`set_npc_visuals` / `set_dead_bodies` 超容时已补一次性告警 `Renderer::warn_npc_cap_once`。
13. **联网 NAT / 断线重连 / 远端实体渲染为 TODO**（UDP 客户端/服务端已有 Input/Snapshot + 插值 + 超时；
    快照的**位置修正应用**与**实体插值渲染消费**均未接线）。
14. ~~**道具是否进阴影 pass 未确认**~~ **已结案（2026-09-14）：确实没进，道具 + 士兵两处都已补**；🔴 **剔除必须用光源视锥**（照抄主 pass 的相机视锥 ⇒ 影子随视角缺块），**代价（只切 `RV3D_NO_SHADOW`）0.5%**。
15. ~~**阴影 `normal_bias` 未使用**~~ **已结案（2026-09-14）：一直在用**（`build.rs:420` 消费它），顺手清了三处**陈旧**的 `#[allow(dead_code)]`；⚠️ 其余 `#[allow]` **必须保留**（只有 `cfg(test)` 用处）。
16. ~~**`tests/rayquery_probe.rs` 被改成 `.bak` 隔离**~~ **已结案（2026-09-14）——文件已不存在**。
17. **`survive` 完整 5 波真机未验**；手榴弹弹道落点测试受玩家出生点影响。
    ~~手榴弹 AoE 不结算障碍~~ / ~~切枪无动画~~ **均已结案（2026-09-15）**：`obstacle_blocks_blast` 只挡"爆心→目标之间"的障碍（含爆心/目标的障碍跳过，否则贴脸炸会把自己堵死）；`WeaponRack::switch_progress()` 给出 0→1 归一化进度，枪模用 `sin(π·t)` 包络做下坠 0.18 m + 前倾 12° + 侧转 6°。两条都**验证过测试会红**。
18. **CoverSeek 战术占比偏低**（压力模式实测 4%，另一次 0；由掩体密度决定）。
    **lead**：加 TOML 关卡掩体。
19. **呈现层欠账**：毛玻璃菜单非真模糊（半透明暗色遮罩近似，需 shader 后处理采样主 pass）；~~kill feed 仅英文~~ **已中文化**；~~不分击杀者名字~~ **已结案（2026-09-15）**：现加 `DamageSource`（只留 `Player`/`Blast`）+ 三处调用点共用 `kill_line`；~~弹孔贴花~~ **已结案（2026-09-15，见铁律 B 弹孔段）**；第一人称枪模动画仍欠。
20. **DLSS 立项评估未做**（**open**，唯一剩下的子项）。~~`playtest_perf.py` 未做 Windows 移植~~ **已结案（2026-09-15）——它不是没移植，是搬不过来**（X11/XImage/`pgrep`/`/proc` 全是 Linux 的），改用 **`scripts/perf_run.ps1`**（只"启动 → 等待 → 读 `logs/perf_*.log` → 统计"，不注入输入、不抓屏）；🔴 **实测噪声底 2.8%（69.7 / 71.8）**，见教训 35。
21. ~~**GLB 加载器忽略 `bufferViews[].byteStride`**~~ **已结案（2026-09-14）：已支持交错布局**（修法 = `byteStride.unwrap_or(...)`）。⚠️ 交错缓冲读错时每个数**都是合法浮点数**（不崩不报、几何静静变乱麻）⇒ **凡"支持"都要补一条会红的测试**。
22. ~~**`data/` 里的历史残留**~~ **已清理（2026-09-13）**：62 文件 → **只留 3 个被代码引用的**（`llm_decisions.jsonl` / `llm_server.jsonl` / `llm_doctrine.json`），同批 `screenshots/` 300→25、`logs/` 646→20、`dist/` 删除，**共回收约 600 MB**。
23. **`VUID-VkSwapchainCreateInfoKHR-flags-parameter` 与输入矛盾，按「层侧误报」挂着**（2026-09-15）。
    报文说 `flags` 带 `MUTABLE_FORMAT` 却没启用 `VK_KHR_swapchain_mutable_format`。**三条否证**：
    ① 全仓只有一个 `create_swapchain` 调用点、从不设 `flags`；② 实测把 flags 打进日志是**空的**；
    ③ ash 的 `SwapchainCreateInfoKHR` 是 `#[repr(C)]` 且字段顺序与 C 头一致。5 次创建 = 5 条报文（1:1）。
    **下一步**：更像 1.4.357 层与 1.3.281 头文件的版本错位。要证伪就把 `flags` 临时设成未定义位看报文是否改口；
    **在上述三条被推翻前不要再改代码"修"它。**

---

## 教训清单（跨迭代去重合并）

> 37 条，每条都真的付过代价。**只留可执行的判据**，案例细节见 `docs/PROGRESS.md`。

1. **新结论与旧约束冲突时，必须删掉旧的那条。** 本文件曾同时存在同一铁律的"错误版 + 更正版"，每条矛盾都导致过一轮错误工作。
2. **先读文档，再动手**（曾花大半天重新发现用户三天前就写下的结论）。
3. **判"某个面没画/形状不对"先拍正侧对或正俯视**，别在 45° 斜透视裁剪上反复判读。
4. **判结构先量尺寸对表设计值。** 反算前 FOV 与距离必须从代码/坐标查，不能估。
5. **取证图必须带 provenance**（文件名 + 对应哪个 commit 的 exe + 相机高度）。
6. **别用"看着像什么"代替验证**，也别用没验证过的输入（hFOV、距离、函数归属）。
7. **改源码一律用编辑器工具，不要过 PowerShell 字符串。** 新写的 `.ps1` 尽量纯 ASCII —— PS 5.1 读无 BOM 的 `.ps1` 按 ANSI 解，非 ASCII 在**字符串字面量**里会破坏引号配对。
8. **`Get-Content` 数行数不准**（实测 2826 vs 实际 3229）—— 行号以 `read` 工具为准（同见铁律 F）。
9. **先看日志再看图**（"键没生效"曾来自只瞄 HUD 小字，而日志里早有记录）。
10. **脚本 kill 的"拆机噪声"不是崩溃**：与在飞帧竞争会刷一叠 `device has been lost`。
11. **`.log` 常是空文件**，游戏日志看 **`.log.err`**。
12. **筛选阈值要从"现象"反推**，别从"我以为它该长什么样"来定 —— 曾把真凶整条过滤掉。
13. **"没有豁口" ≠ "闭合"**（围墙测试一路绿灯，而四角是开口的、能直接走出城市）。
14. **测试写的断言如果恒真，等于没写**（`npcs.len() >= 0`）；回归测试把**错误结论**固化下来的情况也发生过。
15. **越界读是静默的**：实例 buffer 三处副本不同步 → 不崩不报 VUID、返回全零 → 几何全消失。宁可加编译期 `assert!` 收口单一定义。
16. **改共享代码必须双模式验证**（第一人称 + `RV3D_INSPECT=1`）；改 `build.rs`/着色器后 cargo 可能静默不重编。
17. **同一个现象别用没量纲区分度的量去判**（"投影跨度""截图观感"都能被误读）。**"整类地改"只能否证、不能定位**：定位画面里某片几何要靠 `RV3D_DEBUG_KIND=1`（按类染色）+ `RV3D_DUMP_NEAR=<米>`（逐件打印），**⚠️ 半径要给够**（太小给出误导性的空结果）。
18. **"键没生效"这类结论要先排除自己**：`cmd.exe` 传数组会被并成一个数字。
19. **看门狗用「心跳式」，不要按"启动后睡 N 秒"来 arm**（遗留看门狗曾把新会话杀掉，伪装成"窗口没出现"）。
20. **一件事卡住两轮以上，就该去改代码加埋点，而不是继续推理**（曾连推四轮全落空，加一行日志后四个真实原因一次全暴露）。**临时埋点验完就删。** ⚠️ 但**调试 build.rs 不要靠打印 cargo warning**（会被归并/缓存，见未结案 #9）：往文件里写日志。
21. **跨进程读窗口尺寸前必须 `SetProcessDPIAware()`。** 本机 DPI 1.5x，未声明时 `GetClientRect` 报 1706x1066 而真实 2560x1600 ⇒ 注入坐标整体偏 1.5 倍且不报错。
22. **几何/坐标换算的前提假设，要么在注释里写明，要么加断言**（曾因开场多按了 1.5 秒 W 打破"玩家在原点"的假设 ⇒ 38 发点射命中零）。**回路收敛不等于打中了正确的东西。**
23. **怀疑配置没生效时，先用能正确解码的工具复核，再动手"修"** —— `Get-Content` 的输出不能当作文件内容的证据（教训 8 的另一面）。
24. **单次 A/B 说明不了任何事：必须做「互换对照 + 无处理对照」。** **判据：先确认对照组本身没有一边倒，再去看处理组之间的差。** ⚠️ 臂内方差 40 点、臂间差 8 点时就写过结论 ⇒ **< ~5% 的帧率差必须多轮重复才能开口**（见教训 35）。
25. **"做完了"要有可判定的数字标准**，否则会停在"看起来好多了"（眼睛在预览图上完全看不出"这栋楼会插进邻居"）。**动手前先把契约量出来，每次产出都比一遍。**
26. **安全网的假警报和漏报一样有害。** 判定要允许收敛窗口（重试），不能只查一次 —— 一个会喊狼来了的安全脚本，会训练人以后不再当回事。
27. **🔴 先确认你的测量工具测的是你以为的东西。** 一个会话为此栽了六次（键码空间 / 待测区域含小地图 / 均值只留 1 位小数把 1.2% 四舍五入掉 / 全图 diff 含 NPC / 读错文件 / 探针里写了 `break` ⇒"没测到"与"测到 0"分不清）。**判据：任何量化结论之前，先用一个"必然能测出差异"的已知变化验一次工具。**
28. **视觉改动的验收必须给两个数，且两次运行场景必须一致。** 流程：同场采基线 → 改动 → 重采 → 整幅 diff（第一道筛子）→ 在差异集中区取指标；整幅 diff 的数值**不能当改善幅度**用。
29. **"看着不对劲"的东西，先换视角看清它是什么，再去读代码找它**（曾连读五轮代码猜类别、五次全错）。**读代码是"知道名字之后"做的事。**
30. **改回源码要用编辑器工具或 `git checkout --`**，不要过 PowerShell 字符串（会留残差，教训 7 的延伸）。⚠️ **本 shell 的 `ReadAllText` 按 GBK 解码**（65918 B 的文件只读出 38703 字符）⇒ 针对中文的替换**全部静默打不中**，按"读到的行号"删除会**删掉别处的行**。**⇒ 中文文档的编辑一律走编辑工具**；非要用 shell，先验 `(ReadAllText).Length -eq (Get-Item).Length`。
31. **🔴 遇到视觉缺陷，`rg` 代码注释应当是第一动作。** 本仓的处理方式是「改掉 + 在注释里留一段事后分析」，用**现象的词**搜注释常直接命中同一缺陷的历史（曾为"薄壁"连猜五个假设，而它就在注释里写着「白色交叉薄壁」）。
32. **"整类地改"只能否证，不能定位。** 定位画面里某片几何是什么，要靠 `RV3D_DEBUG_KIND=1`（按类染色）+ `RV3D_DUMP_NEAR=<米>`（逐件打印）。**⚠️ 半径要给够** —— 太小会给出误导性的空结果。
33. **🔴 论及资产是否"合理"之前，先走完证据链：名字 → `glb_probe.py` 尺寸 → 生成器规格表**（曾在同一对象上连错三轮，每轮只补一个证据源）。
34. **🔴 参数语义要读注释，不要靠"同一套网格"外推**（答案一直写在表头注释里、只读了一半）。**另一面：那三轮的实机截图看起来都变好了 —— 但改善来自别的因素 ⇒ 观感改善只能证明"改动有效果"，不能证明"数值变对了"。**
35. **🔴 同一份代码跑两次也有 ~3% 的差异**（`perf_run.ps1` 连测两次同一二进制，中位 fps **69.7 / 71.8**）⇒ **小于 ~5% 的帧率差必须多轮重复才能开口**；单次 A/B 只能证伪"巨大回归"，不能证明"变快了"（教训 24 的量化版）。
36. **🔴 「工具跑不起来」本身就是一条要修的缺陷，不是环境噪声。** 验证层因 mesh.spv 布局被拒而**灰屏**，于是被写进文档当"已知限制"、此后再没人开过 —— **期间所有渲染改动都没有验证层兜底**；修掉根因当天第一次开起来就**立刻**报出两条一直存在的 VUID。**⇒ 判据：任何"这个工具在我们这儿用不了"的结论，都要当场问"根因是什么、值不值得修"，修好后的第一个动作就是把它重跑一遍。**
37. **🔴 截图是「崩溃前的最后一帧」——"改动毫无效果"之前先 grep `has been lost`。** 2026-09-15 查弹孔时依次否掉了追加实例、模型矩阵、颜色、尺寸、遮挡…… **而每一次 A/B 都在比对两张"设备已经 lost、画面不再更新"的旧图**（`pm_play` 按 "2" 切枪触发 device lost，见铁律 D）：游戏照常"在跑"、截图照常能存、fps 照常有，**唯独画面是死的**。**⇒ 判据：任何"视觉改动毫无效果"的结论，先在 `logs/<tag>.log.err` 里查 `has been lost` / `panicked`，再去看图。** 图不会告诉你它是第几帧的。
