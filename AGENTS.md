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

**21 世纪架空世界观的大战场 FPS**（⚠ 本文件与 README 旧版曾长期误写为「二战题材」——
据此建模/选材会全错。装备是现代系，HUD 默认武器 **AK-12 风暴 7.62×39mm**，2018 年列装）。
美术基调按 **2020s 当代东欧/中东战乱城镇**走（用户 2026-09-12 定：混凝土板楼 + 抹灰老城 +
破损，冷灰色调），不是战壕与 1940 年代道具，也**不是**本文件旧版写的"近代复古城市"。

Rust + Vulkan，纯 bin crate。**依赖只有 10 个**（`Cargo.toml`）：
`ash` 0.38 / `ash-window` 0.13 / `winit` 0.30(rwh_06) / `glam` 0.29 / `raw-window-handle` 0.6 /
`log` 0.4 / `env_logger` 0.11 / `image` 0.25 / `naga` 30(wgsl-in,spv-out,spv-in) / `rspirv` 0.11。
**不新增第三方依赖**是硬约束。工具链：rustc 1.96.1（2026-06-26）。

### 模块地图（`src/`，按体量）

| 文件 | 行数 | 职责 |
|---|---|---|
| `engine/cjk_glyphs.rs` | 21490 | 生成的中文点阵字模，**勿手改** |
| `engine/renderer.rs` | 10767 | 地形 LOD + 65536 实例场 + HUD 覆盖层。**改 pipeline/shader/swapchain 风险最高，须先跑冒烟验 VUID** |
| `engine/game.rs` | 7377 | 运行时中枢：每帧 `update(dt, camera)` 编排物理/武器/AI/UI/音频/网络 |
| `main.rs` | 3313 | GameApp + winit 事件循环 + 输入/光标捕获 |
| `audio.rs` | 2748 | 合成音效与音乐（`audio_out.rs` 是 waveOut 输出层） |
| `ui.rs` | 2592 | HUD / 菜单 / 设置 / 键位表 |
| `engine/city.rs` | 1802 | 程序化城市生成 |
| `net.rs` | 1733 | UDP 联机（协议魔数 'S'） |
| `engine/ai.rs` / `weapons.rs` / `map.rs` / `procedural.rs` / `cpu.rs` / `physics.rs` | 1436 / 1432 / 1125 / 1082 / 1074 / 951 | AI 分层与战术 / 武器系统 / TOML 关卡 / **程序化贴图 + 烘焙 AO/静态天光** / CPU 拓扑与亲和 / 物理 |
| `llm_cmd.rs` | 550 | RV3D_LLM 战术指挥通道（HTTP 出站，见下） |

其余：`config.rs`（`$HOME/.steel_front.cfg`，原子写 + 容错加载，测试不写盘）、
`engine/objective.rs`（据点/胜负）、`engine/ai_command.rs`、`engine/ray_tracer.rs`（PT）、
**`engine/lighting.rs`（纯运行时：方向光 / 点光源 / 阴影矩阵 / 镜面参数 —— **不含烘焙**）**、
`engine/assets.rs`、`engine/props.rs`（GLB）、`engine/meshgen.rs`、`engine/gpu_caps.rs`、`perf_log.rs`。

- 地形高度纯函数在 `renderer.rs`（`terrain_height` / `terrain_height_at`），**中央 60×60 压平 y=0**。
- `GameState`：`StartMenu` / `LoadingMap` / `Playing` / `GameOver` / `Victory(Team)` / `Defeat`。
- 关卡资产 `assets/maps/*.toml` 6 张：`index` / `street_fight` / `open_field` / `factory_ambush` /
  `bridgehead` / `defense_line`。手写零依赖 TOML 解析在 `map.rs`。

### 验收约束（硬红线）

`cargo test --release` 全绿、**0 警告**（dead-code=0）、不新增第三方依赖、
commit 规范 `feat/fix/docs/chore` + 范围前缀（如 `fix(input)`、`docs(AGENTS.md)`）、
**一个功能一个 commit，禁止 mega-commit**。
**内存 12GB：一次只跑一个 cargo，禁止并行构建。**

---

## 开发环境

> **环境铁律（勿回退）**：开发/验证 = **Windows 原生**。2026-08-15 起从 WSL2 迁出，
> WSL2 相关材料**全部作废、已从本文件删除**（详见文末存档指针）。

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
- **传统 VERTEX + FRAGMENT 管线冻结维护**：只作缺扩展时的兼容回退，**不再新增任何功能**，
  只做必要维护；冒烟基线仍要求双路径 VUID=0。
- **mesh 路径约定（勿回退）**：
  - naga 30 网格写入器对 `@builtin(vertices)` 数组内 position 的 `ADJUST_COORDINATE_SPACE`
    翻转**失效** → mesh 着色器必须在 WGSL 内显式 `v.position.y = -v.position.y`（`build.rs`，删掉会垂直镜像）。
  - `maxMeshWorkGroupCount[0]` 最低保证 65535，地面场 65536 workgroup 必须按查询上限分块下发
    （字段 `mesh_max_wg_x`）。
- **`assets/*.spv` 是运行时从磁盘读的着色器**，不是可随便换的产物。改 WGSL 后必须重新构建，
  **勿手改 .spv**。看到它们变 dirty：先怀疑库里的是不是过期。

---

## 铁律 B — 渲染约定（勿回退）

**坐标与深度**
- Vulkan Y 翻转**只由 shader 负责**（`triangle.vert.spv` / `hud.vert.spv` 各翻 1 次）；
  `main.rs render()` **禁止**再翻投影（`proj.y_axis = -proj.y_axis` 曾致画面上下颠倒）。
  投影用 `glam::Mat4::perspective_rh`（y-up NDC、深度 [0,1]）。
- **阴影深度映射 = `frag_depth = sp.z`**（glam `ortho_rh` 已是 Vulkan [0,1] 深度），
  `lighting.rs::world_to_shadow_uv` 同步取 `(uv, p.z)`。
  🔴 **旧写法 `clip.z*0.5+0.5` 是错的，已于 2026-08-31 推翻**；当时那条"铁律"和把错误映射
  固化的回归测试都已删除。别再从旧记录里把它捡回来。
- 阴影其它约定：UV 必须 V 镜像 `uv.y = 1-(p.y*0.5+0.5)`；光方向语义 = **表面→光源**
  （`sun.direction` 直接传，勿加负号）；采样器 `.compare_enable(false)` 手动 PCF
  （comparison sampler 非 Dref 采样报 VUID）；**地形 identity 矩阵必须写到槽位
  `INSTANCE_COUNT`(65536)**，槽位 0 每帧被 `cull_and_upload` 覆盖；
  参数 2048² D32、半宽 250m、near=1/far=500、3×3 PCF、bias 0.005/0.02；`RV3D_NO_SHADOW=1` 做 A/B。
  排阴影问题先用 **`RV3D_DEBUG_SHADOW=1`**（R=frag_depth/G=阴影图深度均值），别再静态推矩阵。

**顶点格式与着色**
- `stride=32`，`pos@0 color@12 uv@24`，**没有法线槽位**：法线全部由屏幕空间导数重建
  → 只能纯平着色；AO / 烘焙光照**必须进顶点色**；**绕序反了的面直接黑掉且不报错**。
- 实例场与障碍立方体顶点色**全部白化**，颜色只走 tint。地形实例 tint=0.7 灰、
  marker tint=`WorldMarker.tint`（勿混）。
- `flat_flag`：槽位 ≥ 65601（`NPC_SLOT_BASE`）顶点着色器置 1，片元走纯色路径跳过贴图 50% 混合。
  改槽位常量须同步 `build.rs::NPC_INSTANCE_BASE` 与 `renderer.rs::NPC_SLOT_BASE`。
- `Shape::Authored`（`tint.w = 6.0`）→ `flat_flag = 1.25`，跳过四条程序化表面效果
  （`window_dark` / `glass_shade`+菲涅尔 / `is_canopy` 值噪声 / marker 混凝土皮肤）。
  **不接这条，GLB 立面会被再画一层错位窗带（D11 重演）**。
- 皮肤贴图 `RV3D_SKIN_TEX=1` 启用，缺省 0 纯色回退（冒烟基线不变）；
  `flat_flag` 材质编码 0=地面 / 1=marker / 2=NPC，binding 7/8。

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
  本身不改绕序，索引交换是刻意的事。
- **实例 buffer 元素数只允许 `INSTANCE_BUFFER_ELEMS = PROP_INSTANCE_INDEX + 1` 单一定义
  + 编译期 `const _: () = assert!(...)`**。三处（建 buffer、主管线 `descriptor.range()`、
  阴影 pass `descriptor.range()`）必须同源 —— 不同步 = **静默越界读**（驱动不崩不报 VUID、
  返回全零 → 几何全消失）。这是本项目最贵的一类 bug。
- 道具 buffer **固定容量一次分配**，绝不照抄枪模 `next_power_of_two` 按需重建
  （destroy 在飞 buffer → NVIDIA device-lost）。`PROP_INSTANCE_INDEX = GUN_INSTANCE_INDEX + 1`（83010）单槽。
- 道具要剔除 → 在**合并阶段按街区分桶**、每桶一次 draw call，不动实例系统
  （已落地 `merge_binned(cell=40m)`，实测 fps 112→152）。
- 验证层只在 **`RV3D_VALIDATION=1`** 时启用（默认关；否则严格 spirv-val 拒 mesh 布局 → 灰屏）。
- 改共享计算（如 `fp_gun_pre` 顶点/矩阵管线）必须**双模式**截图验证：第一人称 + `RV3D_INSPECT=1` 检视模式；
  检视模式实例矩阵用 `Mat4::IDENTITY`。
- 性能日志里的 `marker` / `npc` 字段 = 每帧 `upload_markers` / `upload_npcs` 的 (near+far) 计数。
- 🔴 **⚠️ 有两个同名的 `npc`，别混**（2026-09-13 我把它们混了，据此写下一个错误结论）：
  - **HUD 左上那行的 `npc: I{} P{} C{} A{}`**（`game.rs:2640`）—— 是 NPC 的**状态人数**
    （Idle / Patrol / Chase / Attack），**与渲染、与箱体实例数毫无关系**。
    ⚠️ 那个 `I` 前缀很容易看漏：`npc: I1255 P0 C0 A0` 是"**1255 个 Idle**"，
    不是"npc=1255"。`RV3D_NPC_CAM` 下 AI 不步进 ⇒ 全员 Idle ⇒ 这个数会很大。
  - **perf 日志里的 `npc=`**（`renderer.rs:8604` = `last_npc_box_near + last_npc_box_far`）——
    这才是**箱体实例数**，用于判断"活体/尸体走的是 GLB 还是回退箱体"。
  **判据要挑对字段**；拿 HUD 那个数当实例数会得出完全错误的结论。
  **排查"某物到底有没有被画"先看这两个计数**，再看图。

**玩家碰撞契约（2026-09-13 变更，勿退回旧写法）**
- 🔴 `PlayerBody::push_out_of_aabb` **现在带 Y 判据**：只有玩家的垂直区间
  `[pos.y, pos.y + eye_height]` 与 `Aabb` 的 `[min.y, max.y]` **相交**时才推挤。
  - 旧契约（**已废弃**）："墙的 y 范围不包含玩家 y，**但水平碰撞仍应生效**" ——
    即不管多高只要 XZ 落进盒子就被水平推开。**那条契约的直接后果是永远站不到任何
    东西上面**（楼顶/集装箱/掩体顶），而且与"垂直方向只贴地形高度、没有顶面支撑"
    合起来，正是用户报的"嵌进地板 / 穿进墙里"。
  - **曾经有 5 条测试把旧契约写死**（`player_y_untouched_by_collision` 的注释甚至
    明写"但水平碰撞仍应生效"）—— 改动时它们会红，那是**预期的**，要改测试而不是改回代码。
- 🔴 **站立面 = `max(terrain_height_at(x,z), PlayerBody::support_height(world, PLAYER_STEP_UP))`**。
  `support_height` 取所有**水平范围内（按 radius 外扩）且顶面不高于 `pos.y + step_up`**
  的盒子的顶面最大值；没有任何盒子时返回 `f32::NEG_INFINITY`。
  **这两条必须成对存在**：只加 Y 判据而不取支撑，玩家会直接穿进盒子里掉下去。
- `PLAYER_STEP_UP = 0.45` m：路缘/台阶能迈上去，护栏(1.5m)/集装箱(2.6m)必须跳。

**呈现模式（2026-09-13）**
- `RV3D_PRESENT_MODE` 支持 `immediate` / `fifo` / **`mailbox`**；引擎默认 **IMMEDIATE**。
- ⚠️ **IMMEDIATE 在真实显示器上是持续撕裂**，快速转视角时正好读成"残影/鬼影"
  （用户 2026-09-13 报告枪有"非常明显的残影"）。**`PrintWindow` 抓不到它** ——
  它抓的是已合成的完整帧，撕裂只发生在显示器上。**别再用静态截图去证伪"残影"。**
- **玩家路径由 `SteelFront.bat` 设 `RV3D_PRESENT_MODE=mailbox`**（不撕裂、也不像 FIFO
  那样在独显直连下等不到 vblank 而死锁）。引擎默认保持 IMMEDIATE 是为了基准最稳。

**建筑摆放（2026-09-13）**
- `city.rs::pick_building` 的缩放是 **`min(w/gw, d/gd)`**（**不是 `max`**）。
  用 `max` 会按较大方向贴合、另一个方向**必然溢出 footprint** ⇒ 相邻楼互相穿插，
  玩家看到"一堆窗格以不同角度叠在一起"（用户截图的"乱窗/透视错误"）。
  **建筑模型本身没问题** —— `preview_glb.py` 渲出来是规整窗格；问题一直在摆放。
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
  光照 ~0.01；旧的 ~1mm 已作废 —— 1mm 正是"PT 永不收敛"的根因）。
- RT 命中判据：`rayQueryGetIntersectionTypeEXT(q, 1)` 必须是 **committed**；
  **声称"RT 已验证"必须给命中着色的图像证据，不能只给 rays/s**。
- PT 着色器改 `assets/rt/pt_panorama.glsl` → glslangValidator → `.spv`，
  `spirv-val --target-env vulkan1.3` 严格通过；用 `scripts/compile_pt.ps1`（勿手工拼装 SPIR-V）。
- PT 盒面法线用不变量 `(primitive % 12) / 2` 查表（"来射方向主轴"近似会把地面法线判成 ±Z → 地面全黑）。
- PT 资产：`PT_MAX_BOXES=512` 一次分配；**BLAS 尺寸必须按容量上限而非当前盒数**
  （按 4 盒算 5376B 塞 512 盒 → 越界写 device lost）；scratch 归 `PtAssets` 所有；两次构建之间加 barrier。

---

## 铁律 C — 输入 / 键位 / 分辨率 / 鼠标安全

### ⭐ 鼠标安全协议（用户 2026-09-03 明确要求，2026-09-12 实测**证实其正确**）

> 引擎自己抓光标，抓取期间鼠标被锁 → 用户的机器会"像死机"。
> **按键用 `PostMessage` 投给游戏窗口句柄，不用 `SendInput`、不用 `SetForegroundWindow`。**
> 截图用 `PrintWindow`，不前置窗口。每次启动游戏必须用 `cap_safe.ps1`（`finally` 里 taskkill + 硬超时）。
> 手动流程 = 启动后点一下游戏窗口 → 按 R 进入操控；死亡后再按一次 R 复活。
> `RV3D_AUTOSTART=1` **不跳过菜单**（2026-09-12 实测，旧版此处写它能跳过，已更正）；
> 要进游戏态请用 `RV3D_NPC_CAM`（它自己会调 `on_any_key()`）。

🔴 **2026-09-12 更正**：本文件中间有一版写着"SendInput 视角注入实测正常"，**那是错的，已删除**。
当天实测（`scripts/input_probe.ps1`）：

- 游戏线程报 `hwndActive == hwndFocus ==` 自己的窗口，**但 `GetForegroundWindow()` 是另一个进程**（浏览器）。
- `SendInput` 6/6 全部被系统接受（返回 1），游戏**一个都没收到**。
- 同一个按键改用 `PostMessage` 投递 → 游戏立刻记录 `weapons: 切枪 0 -> 1`。

原因：`SendInput` 喂的是**前台窗口**的输入队列，`PostMessage` 直接进**目标窗口**的队列。
只要前台不是游戏，SendInput 就是在往别的程序打字（实测打进了浏览器）。
**这与用户的原始指示完全一致，是我没有先读文档。**

### 光标捕获与视角（`main.rs::sync_cursor` / `device_event` / `window_event`）

- `want = self.focused && state==Playing && !settings_open && !esc_menu_open`。
- **`focused` 初值必须是 `false`**（2026-09-12 修，commit `49d994f`）。写成 `true` 会让 `want`
  从第 1 帧就成立：窗口被别的程序占着前台时（启动瞬间极常见），winit 只在收到 `WM_SETFOCUS`
  时才发 `Focused(true)`，本进程既收不到 true 也收不到 false，会一直"自认为有焦点"，
  于是 ClipCursor 把指针钉成 1×1 —— 用户看到的就是整台机器像死机。
  **2026-09-03 用户报告的"鼠标死锁"就是这个，与插件/残留状态无关。**
- **捕获态的视角来源只有两条且互为唯一出口**：
  `device_event` 的 `DeviceEvent::MouseMotion`（仅 `cursor_locked` 时生效）与
  `window_event` 的 `CursorMoved`（`cursor_locked` 时直接 `return`）。
- 🔴 **`DeviceEvent::MouseMotion` 在 Windows 上不存在**：winit 0.30 的 Windows 后端只构造
  `DeviceEvent::Added` / `Removed`（该事件目前只由 X11 / Wayland / macOS / web 后端发出）。
  所以 **Windows 上绝不能用 `Locked`** —— 锁定即等于视角失效，且编译期/运行期都不报错。
  现由平台常量 `RAW_MOUSE_MOTION` + 纯函数 `cursor_grab_plan` 决定（commit `2bfd767`），
  Windows 走 `Confined` + 绝对位置路径。实测日志由 `grab=locked, look=relative`
  变为 **`grab=confined, look=absolute`**。
- **非捕获态有拖拽转视角路径**（`dragging` 由左键按下置位），源码注释称之为"冒烟在无焦点环境下的
  瞄准路径"。该路径每个事件后把真实光标 warp 回窗口中心并把 `last_cursor` 设为该中心。
- `MAX_LOOK_DELTA_PX = 512`（绝对位置单次位移上限，超过视为光标传送，跳过并重基准）；
  `MAX_RAW_LOOK_DELTA = 1024`（raw 单事件上限）。

### ⭐ 无焦点视角注入配方（2026-09-12 实测标定，误差 0.3%）

`scripts/pm_play.ps1` 已实现下面四条，**照抄即可，别再重推**。缺任何一条都会静默失效：

1. **必须用 `PostMessage`，不能用 `SendInput`**（理由见上）。
2. **每一步前重新按下左键**（`WM_LBUTTONDOWN`），然后**紧接着**投移动，两条消息背靠背。
   原因：post 的第一次移动会让 winit 判定指针"进入窗口"并调 `TrackMouseEvent`；而真实光标
   并不在窗口上，于是 Windows 立刻投递 `WM_MOUSELEAVE` → winit 发 `CursorLeft` →
   `main.rs` 把 `dragging` 置回 false。**移动照样到达、照样被日志记录，但视角被跳过。**
3. **每一步投的偏移都必须是"窗口中心 + step"，不能累加**。拖拽路径每个事件后把 `last_cursor`
   重设回中心，所以游戏看到的增量恒为 `posted - centre`；累加会让第 2 步变成 800px，
   **超过 `MAX_LOOK_DELTA_PX=512` 被当传送丢弃**（这正是"请求 1200px 与 400px 都只转一步"的原因）。
   步长取 400，**连续两次坐标要差 1px**，否则被 winit 的位置去重丢掉
   （`cursor_moved = last_position != Some(position)`）。
4. **步间隔必须 > 150ms（用 300ms）**。捕获路径会给 `recenter_pending_until` 设一个 150ms 窗口，
   落在窗口内的 `CursorMoved` 会被吞掉（只更新基准、不转视角）。90ms 间隔时每隔一步就被吞一次。

- **实测标定**：1200px（3×400）→ 实测 **-170.30°**，模型预测 **-170.79°**，误差 0.3%。
  模型 = `yaw -= dx * (0.0005 + sensitivity*0.002)`（`game.rs::sensitivity_rads`），
  且 **`cam:` 日志的 yaw/pitch 单位是度**（内部弧度），换算别忘 `*180/PI`。
- **`SetCursorPos` 驱动真实光标走不通**（2026-09-12 实测）：那样做游戏收到 **0 个**
  `CursorMoved`。posting `WM_LBUTTONDOWN` 并不会让 winit 调 `SetCapture`，真实移动都给了前面那个窗口。

### 键位与方向
- **键码一律用 winit 0.30 `KeyCode` 枚举序号**（KeyW=41 / KeyS=37 / KeyA=19 / KeyD=22 /
  KeyR=36 / Space=62 / ContextMenu=54 / Escape=114），**不是 USB HID 码**。
  `ui.rs::winit_keycode_indices_match_table` 锁死这张表，**升级 winit 先跑它**。
- `config.rs bindings_version=1`：旧版 HID 键码配置整体忽略回退默认（**勿删迁移逻辑**）。
- **鼠标水平**：`look()` → `yaw -= dx*sens`（右移 = 右转）。
- **鼠标垂直**：`pitch += dy*sens`（winit Y 向下，dy>0 = 鼠标下移 = 低头）；
  后坐力 `pitch -= recoil_pitch*dt`。旧 `pitch -= dy*sens` 是反的，且正是"低头剔除 bug"的根因。
  **勿再翻 pitch。**
- **灵敏度**：`game.rs::sensitivity_rads() = 0.0005 + hud.sensitivity * 0.002`
  （默认 0.5 → 0.0015 rad/px），`main.rs` 每帧 `set_mouse_sens` 同步。
  `camera.rs::set_mouse_sens` 夹在 `[0.0005, 0.02]`。**勿改回 0.003 起步。**
- 注入脚本必须**从 `~/.steel_front.cfg` 读真实灵敏度**，不要写死。
- `cam: yaw=.. pitch=..` 日志的单位是**度**（内部弧度）。
- 死亡重开：R 或 Enter。保留键 ESC / TAB / ENTER / F12 / Q / E / N 不可重绑。
- `Tab`（VK 9）在玩法态**不切视角**，别依赖它做环绕取证。
- 🔴 **调试相机取证必须带 `RV3D_NO_NPC_CULL=1`**（2026-09-12 定案，本会话最贵的一条）：
  `npc_occluded()` 是**以「玩家眼位」为中心**的剔除。正常玩法里玩家就是相机 ⇒ 正确；
  但 `RV3D_CAM` / `RV3D_NPC_CAM` 把相机移到别处时**玩家仍在原点** ⇒
  **相机眼前的人被判为"从玩家位置看不到" ⇒ 剔除**。
  实测：关剔除前 `npc=288`（≈16 人），开后 `npc=4590`（255 人 × 18 段）。
  **⇒ 第 55~79 轮"看不到士兵"的真正原因就是它**，而不是模型、不是遮挡、不是取景。
  （第 19 轮曾测出"误剔除 93.7%"并被我判为误报撤回 —— **那次撤回是错的**，数字是真的。）
- 🔴 **`RV3D_CAM` 的朝向必须先标定再用来做对照实验**（2026-09-12 付过代价）：
  `forward = (-sin(yaw), 0, -cos(yaw))`，即 **yaw=90° 面向 −X，不是 +X**。
  当天我拿两个朝向做 A/B，把方向搞反，据此写出"主因是地面/地形"的错结论并已入库，
  靠后续"关掉道具帧率涨 2.7 倍"才发现方向不对。
  **判据：做方向类对照前，先用一个已知会在视野里/不在视野里的物体验一次朝向**，
  别相信"yaw=90 应该是往右"这种直觉。
  🔴 **pitch 同样必须先标定 —— 2026-09-12 我为此白烧了四轮 9 次取景**：
  **`RV3D_CAM` 里负 pitch = 抬头**（`-84` 拍到的是天空，不是地面）。俯看要用**正**值（`:0,84`），
  平视 `:0,0`，抬头用负值。我给 yaw 写了标定规则却**没有对 pitch 执行同一条规则** ——
  **规则写了不执行等于没写**；下次用任何角度参数前，先花一次 run 验一个已知朝向。
- 🔴 **注入脚本里的键码有两套，混用会「发出去了但游戏零响应」**（2026-09-12 付过代价，
  查了三轮才找到）：
  - **winit `KeyCode` 枚举序号**（含 `KeyW=41`）—— 只在**游戏源码/配置**里用。
  - **Windows 虚拟键码 VK** —— 只有它才该进 `PostMessage(WM_KEYDOWN)` 的 `wParam`。
    常用：`R=82(0x52) C=67 Z=90 W=87 A=65 S=83 D=68 Space=32 Tab=9 Esc=27 Shift=16`。
  - 反例：把 winit 的 `KeyR=36` 当 VK 发出去，游戏收到的是 **`VK_HOME`**；`44` 是 `VK_SNAPSHOT`
    （PrintScreen）。`-Keys 9` 之所以"看起来能用"，纯粹因为 **9 在两套里恰好都是 Tab**。
  - `lParam` 的 **bit16-23 必须是扫描码**（`MapVirtualKey(vk, 0)`），winit 靠它解析键位。
- 🔴 **`cap_safe.ps1` 的两个历史 bug 已修（2026-09-12）**，别再退回：
  ① 窗口句柄不能用 `Process.MainWindowHandle`（对 winit 程序拿到的不是接收输入的那个窗口）
  → 用 `FindWindowW(None, "Steel Front - Vulkan")` + 40×250ms 轮询；
  ② `-Keys` 必须收 **VK 码** + `lParam` 带扫描码。
  **`pm_play.ps1` / `gameplay_smoke_pm.py` 一直是正确的参照实现**（实测无前台也能送达）。

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
  `baseColorFactor` 已是线性直接用；贴图 socket 的 0.8 默认值**刻意忽略**
  （乘上去每把贴图枪暗 20%）。
- 枪模 buffer 上限很松（顶点峰值 11949/32768 = 36%、索引峰值 33585/262144 = 13%）
  → **不要动它、不要加扩容逻辑**（device-lost 老雷）。
- 程序化贴图（勿回退）：写 `R8G8B8A8_SRGB` 前必须 linear→sRGB 编码；材质分域用**世界尺度**
  value_noise（biome ~120m、detail ~10m），旧 `fbm(x*0.025)`（2560m）会致整图单色；
  `RV3D_PROC_TEX=0` 回退做 A/B；`MARKER_INSTANCE_BASE=65537`；片元用 world-space UV。
- `SteelFront.bat` 的 touch 列表**必须含 `build.rs` 与 `build_spv_rt.rs`**
  （只改着色器/构建脚本时 cargo 会静默不重编 = 启动旧版本）。

### ⭐ 设计化建模链路（2026-09-12 建立，取代 `gen_props.py::asset_building`）

> 旧路线把建筑写成"参数拼箱子"（开间数 / 窗间墙宽 / 窗台高），结果是一栋 14m 宽的面
> 只开 **4 个 2.75×1.6m 的洞** —— 那是店面橱窗的比例；而且没有勒脚、没有外挑窗台、
> 没有女儿墙压顶、没有入口、没有阳台、没有屋顶杂物，每个面一个平色。
> **参数生不出品味，设计过的模块可以。** 现有 6 个建筑模块由
> `tools/blender/build_city_kit.py` 生成，是"照着真实板楼/抹灰楼的比例写死"的，不是尺寸区间。

```powershell
# 1) 尺寸普查：24 件资产的占地/高度/底面 —— **尺寸契约的唯一来源**
blender.exe --background --python tools/blender/survey_props.py -- assets/props "*.glb"
# 2) 生成（可只给名字）：输出到临时目录，确认后才覆盖 assets/props/
blender.exe --background --python tools/blender/build_city_kit.py -- <out_dir> [name...]
# 3) 预览渲图：4 视图 → PNG，**我必须用眼睛看过再入库**
blender.exe --background --python tools/blender/preview_glb.py -- <in.glb> <out_prefix> [n]
```

- **引擎只给了三条视觉杠杆，别指望第四条**：没有法线槽位（法线由屏幕空间导数重建
  ⇒ **纯平着色**），所以**细节只能是真几何**；AO/明暗**必须烘进顶点色**；道具
  `export_materials="NONE"`，外观**全部**来自顶点色。
- **尺寸契约是硬的**：`city.rs` 按**占地**摆放，`building_block` 只有 4 处但
  `building_tall` 有 **52 处**。资产越出契约轮廓就会插进邻居 —— 第一版模块的**外挑阳台
  让进深从 10.9 涨到 13.6**，入口雨篷再 +1.0、台阶再 +0.62。**外挑一律改内凹**（凹阳台
  loggia 既是东欧板楼的真做法，又完全不占轮廓）。允许的小外挑上限 = **0.06m/侧**（勒脚/窗台）。
- **层高反解**：总高固定，`上层 = 3.15`（**等于引擎 `FLOOR_H`**），**底层吸收余量**
  （现值 3.56）。硬编 3.4 就是 `FLOOR_H` 分叉的来源，**别再写死**。
- **绕序交给叉积判定**：`add_quad_n()` 收"意图法线"，算叉积、反了就翻。引擎侧反的面
  **直接黑掉且不报错**，靠手推绕序是一类静默事故。
- ⚠ **预览图只对"几何与比例"可信，对"最终颜色"不可信**：预览走 Blender 自己的光照 +
  AgX 视图变换，同一份顶点色在引擎里明显更暗。**颜色判断必须在引擎里做**（`RV3D_CAM` 固定机位取证）。
- **顶点预算**：`props: 缓冲扩容 顶点 N/2097152` ⇒ **硬容量 2^21 = 2,097,152**。
  🔴 **2026-09-13：已从 156 万降到 51 万（−67%），靠的是"焊接 + 去掉法线"** ——
  见下面那条通则。加细节前先看这个数。
- 🔴🔴 **道具/GLB 的两条通则（2026-09-13 实测，改动前先读）**：
  1. **`box_project_uv` 的逐面 UV 岛 + `export_normals=True` 的逐面法线，
     两者都会阻止顶点共享。** 实测 `tree_oak` 838 三角形却占 **2264 顶点**
     （全城 372 棵 = 84 万顶点 = 整个道具缓冲的 54%）。
  2. **引擎根本不需要这两样**：顶点格式是 `stride=32, pos/color/uv`，**没有法线槽位**
     （法线由屏幕空间导数重建，见铁律 B）；而 `Shape::Authored`（flat_flag=1.25）
     在片元里显式**跳过**四条会采样 UV 的效果（`build.rs:611-616`）。
     `assets.rs` 也只把 `POSITION` 当硬要求，缺 `NORMAL`/`TEXCOORD_0` 有显式回退。
  ⇒ **改法**：`tools/blender/weld_props.py`（按 pos+color 去重、UV 设常量、
  **`export_normals=False`**）。**`export_normals` 是总开关** ——
  开着它时焊到 458 顶点、落盘又变回 2264（导出器按面拆分），实测只降 6.6%。
  **收益（同机位实测）**：顶点 1563020→509616、显存 72→24 MB、
  每帧提交 476100→153363、**中位帧率 132.1→156.6（+18.6%）、最差帧 +21.2%**，
  **三角形一个没少（199981）、画面无退化**。
  **本仓是顶点瓶颈**（判据：像素少 4 倍只 +12%；一次画完反而 −38%）——
  所以顶点数就是帧率，这条通则是最高杠杆的一处。
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
- **看到"规划中"的 dead code，必须回答"那它为什么没被接线"，不许加 `#[allow]` 了事。**
  → 待办：删 `scripts/allowall.py` / `allowv3.py` / `allowv4.py` 的 `dead_code` 压制规则。
- **阈值纪律**：冒烟 `fps_min` 越线先判是不是**首帧窗口**（判据 = 仅首样本越线 +
  `npc` 计数远低于稳态 + `wait_fence ≈ frame`，SPIR-V 重生成后驱动 JIT 冷缓存），
  重跑确认 —— **别改测试、别调阈值**。
- **验收口径**：`run_smoke_pm.ps1` → `gameplay_smoke_pm.py`，判据 = **`vuid==0 and panics==0 and killed>=1`**，
  **无 fps 门槛**（`fps=` 只用于打印）。旧版误把 `fps>=120` 写进这里 —— 那是**旧 SendInput 版
  `gameplay_smoke.py`** 的规则，而本文件又写着"别用旧脚本"。**两个脚本的口径别混。**
  `playtest_perf.py` 是**时长制**：跑满 `PT_SECS`（默认 600s）即完成，击杀是附带指标、不设门槛、不判 FAIL。
- 微基准：`cargo test --release <名> -- --nocapture --test-threads=1`
  （`shockwave_path_microbench` / `simd_cull_microbench`）；
  `RV3D_FORCE_SIMD=avx512|avx2|avx|sse4.2|scalar`（仍要求硬件支持，非法值告警回退）。
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
```

⚠ `cap_safe` / 截图脚本的游戏日志是 **`logs/<tag>.log.err`**（stdout 的 `.log` 常为空文件）。
⚠ `scripts/play_cap.ps1` 已于 2026-09-12 删除（它用 SetCursorPos + mouse_event 拿到焦点后
不做真正的释放，正是用户 2026-09-03 报告的鼠标死锁那一类行为；已由 cap_safe.ps1 + pm_play.ps1
+ release_input.ps1 取代）。
---

## 🔴 未结案清单

> 按"值不值得下一轮动手"排序。每条给出当前最优线索（lead），**没有 lead 的不要瞎猜。**

### ⭐ 最高优先级：红蓝阵营不对称（**未定案；先解决"测不准"**，2026-09-13）

压力模式下**红方恒胜**（四次 190s 对撞，蓝方净耗 101/102/102/103）。
**2026-09-13 的互换对照没能定案，而且暴露了更根本的问题：这个测量本身的方差极大。**

| 臂 | 红损 | 蓝损 | 蓝方差 |
|---|---|---|---|
| A 原样（红 +X / 蓝 −X） | 40 | 62 | −22 |
| B 对调（红 −X / 蓝 +X） | 38 | 68 | −30 |
| C 朝向敌人（见下）第 1 次 | 75 | 24 | **+51** |
| C 朝向敌人 **第 2 次（同一条命令）** | 57 | 46 | **+11** |

**⇒ 🔴 C 臂两次相同运行给出 +51 与 +11 —— **臂内方差 ≈40 点**，比 A/B 之间的差（8 点）
大一个量级。所以「赢家跟着半场走还是跟着队伍走」**这个问题用单次 run 根本回答不了**，
我一度据此写下的结论**已作废**（这正是教训 24 的形态，我又踩了一次）。

**⇒ 下一步不是继续换参数，而是先把测量做可靠**（任选其一）：
① **每臂重复 ≥5 次**，比较分布而不是单点；
② **把 run 拉长到 190s 以上**（原报告的 101/102/102/103 就是 190s 的，比本次的 125s 稳定）；
③ 找**确定性种子**（出生半径抖动、`role_for` 都已是确定性的，但 AI 的时序/索敌依赖浮点累积，
   `RV3D_AI_DECIMATE`、线程调度都可能引入不可复现性 —— 先确认它到底确不确定）。

**现成的仪器**（不用写代码）：
* `RV3D_SWAP_SIDES=1` —— 只交换两个出生半场（`game.rs::spawn_stress_battle`，默认关）
* `ai:` 日志每秒打一次 `red=<存活> blue=<存活>`
* 跑法：`cap_safe.ps1 -Keys @(82) -AfterKeysSec 125`，再读 `logs/<tag>.log.err` 最后一行

**⚠️ 仓里那条 `RV3D_FACE_ENEMY` 开关是这次失败实验的遗留物**：它把出生朝向从"朝向玩家"
改成"朝向对面半场"。**改完发现两者在几何上本来就是同一个方向**（红在 +X 朝 −X、蓝在 −X 朝 +X），
所以它**不该有效果** —— 而实测的差异全部来自噪声。**开关保留（无害、默认关），
但不要拿它的任何一次结果当证据。**

- 相关：军情 JSON 的 `击杀` 字段实际是**该营自身阵亡数**（`round_kills_*` 按阵亡者阵营计数）。
  若 `llm_commander.py` 当"我方战果"读则**信号是反的** —— 调参前先核对。

1. **`PrintWindow` 对非前台窗口返回冻结帧**（2026-09-12 定案）：注入 170.3° 转向（游戏日志为证）
   之后前后两张截图**残差 0.03、x-shift=0** ⇒ **截图的"没变化"不能用来判断输入没生效，以游戏日志为准。**
   **lead**：要截图取证就把窗口置前（`cap_safe.ps1` 那条路）。
2. **PT 崩溃 `0xC0000005`** — `pt_enable=false` 现状；设 true 一启动即崩，无法截图验收。
   **lead**：崩点在 `pt_set_scene_markers` 返回之后（每帧 PT 派发 / 主命令缓冲 / blit 到 swapchain）；
   候选 = AS 显存与尺寸、dispatch 与 scene rebuild 读写竞争、push constant 布局。
   判据：用 Windows 事件日志的出错模块区分驱动侧（`nvoglv64.dll`）与应用侧。
3. **`config.rs` 不读 `pt_enable` / `rt_enable`** → 配置文件与 `RV3D_PT_LIVE=1` 都开不了 PT。
   **lead**：`main.rs` 的 `if config.pt_enable { init_pt_resident() }` 分支，resident 从未建。
4. ~~**玩家可能站在 GLB 楼体内部**~~ **已结案（2026-09-13）**：根因是 `pick_building`
   用 `scale = max(w/gw, d/gd)` ⇒ 另一个方向**必然溢出 footprint**（52 处 `building_tall`
   互相穿插），以及碰撞盒用 footprint `(w,d)` 而视觉体是缩放后的 GLB。现改为
   **`min`**（楼放得进格子）+ **碰撞盒取真实视觉尺寸**（`half_footprint × scale`，
   ±90° 时 x/z 互换）⇒ 既不穿插、也没有"无形的墙"。详见"建筑摆放（2026-09-13）"。
5. ~~**`FLOOR_H` 常量分叉**~~ **已结案（2026-09-12）**：6 个建筑模块「上层 3.15（= `FLOOR_H`）+ 底层反解 + 女儿墙 + 压顶 = 精确总高」，实测 6/6 命中。旧硬编 3.4 的 `asset_building` 已弃用。
6. **`svd_63` 未入库** — 源文件是含两把相差 90° 重叠枪身 + 独立瞄具的产品宣传图，
   `install_guns.py` 仍 SKIP。需人工删掉重叠枪身后装为 `svd12`。
7. ~~**D12 士兵近距观感（用户说"神人样子"）**~~ **已结案（2026-09-13）**。
   18 段箱体已由 `assets/soldier/soldier.glb`（1082 顶点 / 540 三角形、真人比例、
   烘天光遮蔽、头/盔用六棱柱）取代，**走实例化**：网格上传一次 +
   `cmd_draw_indexed(索引数, 实例数 N, 0, 0, SOLDIER_INSTANCE_BASE)`，用 `self.pipeline`
   （传统 VERTEX、depth 开），**不需要新建管线**。**每 NPC 实例数 18 → 1**
   （压力模式 fps 111 → 155）。步态用实例矩阵做（起伏 ±4cm + 前后倾 ±3° + 开火后坐），
   尸体用同一条路（绕原点转 −90° 平躺）。阵营色 = 取向白靠拢 35% 的队色 ×
   `tint.w = 6.0`（Authored 标记，不接这条会整身涂成一队色）。
   **`soldier_part_matrices` / `dead_part_matrices` 保留**为 GLB 缺失时的回退与远距 LOD 备选。
   全过程与三个踩过的坑见 `docs/HANDOFF-soldier.md`。
   **仍缺**：真正的骨骼动画（现在是整体起伏）、两套队色顶点变体（现在是 tint 乘色）。
8. **D4 墙缝天空亮条 / 悬浮亮条** — **lead**：疑似楼间缝隙的正常天空，需定点复现再定。
9. **mesh 着色器布局未过严格 `spirv-val`**（Workgroup Offset 布局）。
    **lead**：开 `RV3D_VALIDATION=1` 做 RT 调试前应先修。
10. **PT 512 盒上限静默截断**（实测 `marker=547 > PT_MAX_BOXES=512`）。
    **lead**：提容量或按视锥裁剪。相关：`PT_SUN_AMBIENT` 无消费者、天空/环境项硬编在 GLSL；
    曝光 0.2 硬编在 `main.rs`，曝光/弹跳/spp 都未进 `config.rs` 与设置面板。
11. **PT 与光栅同屏叠加未做**（现为整体替换）；移动相机每次全量重开累积。
    **lead**：按像素重投影复用，或运动自适应 spp。相关：`signature()` 量化已改分层
    （位置 ~0.5m / 朝向 ~3° / 光照 ~0.01），**勿回退到 1mm**。
12. **`MAX_RIGID_BODIES=640` vs `MAX_AI=768`** 溢出静默丢弃（release 下 `debug_assert` 被优化掉）。
13. **联网 NAT / 断线重连 / 远端实体渲染为 TODO**（UDP 客户端/服务端已有 Input/Snapshot + 插值 + 超时；
    快照的**位置修正应用**与**实体插值渲染消费**均未接线）。
14. **道具是否进阴影 pass 未确认**（不画则道具没有投影）—— 提出后未见结案，也未见再提。
15. **阴影 `normal_bias` 已在 uniform 但未使用** —— 需要更干净的阴影边界时做坡度 bias。
16. **`tests/rayquery_probe.rs` 被改成 `.bak` 隔离**（引用 naga 导致 test 目标编译失败）——
    待清理或正式入库。
17. **`survive` 完整 5 波真机未验**；手榴弹弹道落点测试受玩家出生点影响；
    手榴弹 AoE 不结算障碍；切枪无动画（纯计时器）。
18. **CoverSeek 战术占比偏低**（压力模式实测 4%，另一次 0；由掩体密度决定）。
    **lead**：加 TOML 关卡掩体。
19. **呈现层欠账**：毛玻璃菜单非真模糊（半透明暗色遮罩近似，需 shader 后处理采样主 pass）；
    kill feed 仅英文（5×7 位图字体无中文）、不分击杀者名字；第一人称枪模动画 / 弹孔贴花未做。
20. **`playtest_perf.py` 未做 Windows 移植**；**DLSS 立项评估未做**。
21. **GLB 加载器忽略 `bufferViews[].byteStride`**（交错布局会读错）。
    **lead**：现导出器是一 accessor 一 bufferView（密集），暂不受影响。
22. ~~**`data/` 里的历史残留**~~ **已清理（2026-09-13）**：62 个文件 → **只留 3 个被代码引用的**（`llm_decisions.jsonl` / `llm_server.jsonl` / `llm_doctrine.json`），回收 19.5 MB。同批清理：`screenshots/` 300→25 个（456 MB，只留文档引用的基线）、`logs/` 646→20 个（43 MB）、`dist/`（79 MB，可再生产物）。**共回收约 600 MB。**

---

## 教训清单（跨迭代去重合并）

> 34 条，每条都真的付过代价。**只留可执行的判据**，案例细节见 `docs/PROGRESS.md`。

1. **新结论与旧约束冲突时，必须删掉旧的那条。** 本文件曾同时存在"错误版 + 更正版"的同一铁律，每条矛盾都直接导致过一轮错误工作。
2. **先读文档，再动手。** 曾花大半天重新发现用户三天前就写在文档里的结论。
3. **判"某个面没画/形状不对"先拍正侧对或正俯视**，别在 45° 斜透视裁剪上反复判读。
4. **判结构先量尺寸对表设计值。** 反算前 FOV 与距离必须从代码/坐标查，不能估。
5. **取证图必须带 provenance**（文件名 + 对应哪个 commit 的 exe + 相机高度）。
6. **别用"看着像什么"代替验证**，也别用没验证过的输入（hFOV、距离、函数归属）。
7. **改源码一律用编辑器工具，不要过 PowerShell 字符串。** 新写的 `.ps1` 尽量纯 ASCII —— PS 5.1 读无 BOM 的 `.ps1` 按 ANSI 解，非 ASCII 在**字符串字面量**里会破坏引号配对。
8. **`Get-Content` 数行数不准**（实测 2826 vs 实际 3229）——行号以 `read` 工具为准。
9. **先看日志再看图。** "键没生效"曾来自只瞄 HUD 小字，而日志里早有记录。
10. **拆机噪声不是崩溃**：脚本 kill 与在飞帧竞争会刷一叠 `device has been lost`。
11. **`.log` 常是空文件**，游戏日志看 **`.log.err`**。
12. **筛选阈值要从"现象"反推**，别从"我以为它该长什么样"来定 —— 曾把真凶整条过滤掉。
13. **"没有豁口" ≠ "闭合"。** 围墙测试一路绿灯，而四角是开口的、能直接走出城市。
14. **测试写的断言如果恒真，等于没写**（`npcs.len() >= 0`）。回归测试把**错误结论**固化下来的情况也发生过。
15. **越界读是静默的**：实例 buffer 三处副本不同步 → 不崩不报 VUID、返回全零 → 几何全消失。宁可加编译期 `assert!` 收口单一定义。
16. **改共享代码必须双模式验证**（第一人称 + `RV3D_INSPECT=1`）；改 `build.rs`/着色器后 cargo 可能静默不重编。
17. **同一个现象别用没量纲区分度的量去判**："投影跨度""截图观感"都能被误读。
18. **"键没生效"这类结论要先排除自己**：`cmd.exe` 传数组会被并成一个数字。
19. **看门狗不要按"启动后睡 N 秒"来 arm** —— 早先遗留的看门狗曾把新会话杀掉，伪装成"窗口没出现"。用**心跳式**。
20. **一件事卡住两轮以上，就该去改代码加埋点，而不是继续推理。** 曾连推四轮全落空，加一行日志后四个真实原因一次全暴露。**临时埋点验完就删。**
21. **跨进程读窗口尺寸前必须 `SetProcessDPIAware()`。** 本机 DPI 1.5x，未声明时 `GetClientRect` 报 1706x1066 而真实 2560x1600 ⇒ 注入坐标整体偏 1.5 倍且不报错。
22. **几何/坐标换算的前提假设，要么在注释里写明，要么加断言。** 曾因开场多按了 1.5 秒 W 打破"玩家在原点"的隐含假设 ⇒ 38 发点射命中零。**回路收敛不等于打中了正确的东西。**
23. **怀疑配置没生效时，先用能正确解码的工具复核，再动手"修"** —— `Get-Content` 的输出不能当作文件内容的证据（教训 8 的另一面）。
24. **单次 A/B 说明不了任何事：必须做「互换对照 + 无处理对照」。** 曾据此得出会反转的结论。**判据：先确认对照组本身没有一边倒，再去看处理组之间的差。** ⚠️ **2026-09-13 又踩一次**：见"红蓝阵营不对称"那条 —— 臂内方差 40 点、臂间差 8 点，我却先写了结论。
25. **"做完了"要有可判定的数字标准**，否则会停在"看起来好多了"。眼睛在预览图上完全看不出"这栋楼会插进邻居"。**动手前先把契约量出来，每次产出都比一遍。**
26. **安全网的假警报和漏报一样有害。** 判定要允许收敛窗口（重试），不能只查一次 —— 一个会喊狼来了的安全脚本，会训练人以后不再当回事。
27. **🔴 先确认你的测量工具测的是你以为的东西。** 一个会话为此栽了六次（键码空间 / 待测区域含小地图 / 均值只留 1 位小数把 1.2% 变化四舍五入掉 / 全图 diff 含 NPC / 读错文件 / 探针里写了 `break` ⇒"没测到"与"测到 0"无法区分）。**判据：任何量化结论之前，先用一个"必然能测出差异"的已知变化验一次工具。**
28. **视觉改动的验收必须给两个数，且两次运行场景必须一致。** 流程：同场采基线 → 改动 → 重采 → 整幅 diff（第一道筛子）→ 在差异集中区取指标。整幅 diff 的数值**不能当改善幅度**用。
29. **"看着不对劲"的东西，先换视角看清它是什么，再去读代码找它。** 曾连读五轮代码猜类别、五次全错，把相机飞上去俯视一次就定案。**读代码是"知道名字之后"做的事。**
30. **改回源码要用编辑器工具或 `git checkout --`**，不要过 PowerShell 字符串（会留残差）。这是教训 7 的延伸。
31. **🔴 遇到视觉缺陷，`rg` 代码注释应当是第一动作。** 本仓的处理方式是「改掉 + 在注释里留一段事后分析」，所以用**现象的词**搜注释常常直接命中同一个缺陷的历史。曾为"薄壁"连猜五个假设、烧十几轮，而它就在注释里写着「白色交叉薄壁」。
32. **"整类地改"只能否证，不能定位。** 定位画面里某片几何是什么，要靠 `RV3D_DEBUG_KIND=1`（按类染色）+ `RV3D_DUMP_NEAR=<米>`（逐件打印）。**⚠️ 半径要给够** —— 太小会给出误导性的空结果。
33. **🔴 论及资产是否"合理"之前，先走完证据链：名字 → `glb_probe.py` 尺寸 → 生成器规格表。** 曾在同一对象上连错三轮，每轮只补一个证据源、结论就翻一次。
34. **🔴 参数语义要读注释，不要靠"同一套网格"外推。** 曾在一张尺寸表上栽三轮，而答案一直写在表头注释里、我只读了一半。**另一面：那三轮的实机截图看起来都变好了 —— 但改善来自别的因素。⇒ 观感改善只能证明"改动有效果"，不能证明"数值变对了"。**
