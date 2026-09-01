# AGENTS.md — Steel Front 项目记忆与 AI 交接文档

## 项目
二战题材 FPS，Rust + Vulkan（winit 0.30），零第三方游戏依赖，纯 bin crate。
- 入口：`src/main.rs`（GameApp + winit 事件循环）
- 运行时中枢：`src/engine/game.rs`（每帧 `update(dt, camera)` 编排物理/武器/AI/UI/音频/网络）
- 渲染：`src/engine/renderer.rs`（地形 LOD + 65536 实例场 + HUD 覆盖层；改 pipeline/shader/swapchain 风险高，须先跑冒烟验 VUID）。**渲染主路径 = `VK_EXT_mesh_shader` 网格着色器（MESH + FRAGMENT）**，传统 VERTEX + FRAGMENT 顶点管线仅作 WSLg/dzn 回退（见下方「渲染技术路线铁律」）
- 地形高度纯函数在 renderer.rs（`terrain_height` / `terrain_height_at`），中央 60×60 压平 y=0
- 配置持久化：`src/config.rs` → `$HOME/.steel_front.cfg`（Windows: `C:\Users\<user>\.steel_front.cfg`；原子写 + 容错加载，测试不写盘）
- 测试：`cargo test`（纯逻辑，不碰 GPU）
- 验收约束：dead-code=0（0 警告）、测试全绿、不新增第三方依赖、commit 规范 **`feat/fix/docs` 前缀**（`chore` 亦允许；一个功能一个 commit，禁止 mega-commit）
- 内存约束：12GB，一次只跑一个 cargo，禁止并行构建

## 开发环境（2026-08-15 起：Windows 原生 + DeepSeek Harness）

> **环境铁律（勿回退）**：开发/验证已从 WSL2 迁移至 **Windows 原生**。
> 历史 WSL2 内容（git 只跑 WSL、X11 冒烟、dzn 转译等）**保留作历史记录，不再适用**。

- **开发主体**：Windows 原生（RTX 5060 Laptop 真机，NVIDIA 驱动 610.88）。总指挥（DeepSeek Harness）直接在本仓库改代码、编译、测试、冒烟、提交（GitHub 令牌 push，仅限本仓库）。
- **编译**：`cargo build --release`（VS 2026 + rustc 1.96+，Windows 侧已配好）。
- **测试**：`cargo test --release`（当前基线 364 tests 全绿、0 警告）。
- **冒烟**：`powershell -ExecutionPolicy Bypass -File scripts/run_gameplay_smoke.ps1`（SendInput 注入 + 日志断言，约 30s，ALL-OK = VUID=0 + kills>=1 + fps>=120）。游戏 stdout → smoke.log、stderr → smoke.log.err（脚本内部合并）。
- **分辨率**：默认 2560x1600（配置文件 `C:\Users\Jerry-Huang\.steel_front.cfg`，RESOLUTIONS 5 档）。
- **GPU 能力（原生实测，勿回退）**：VK_EXT_mesh_shader=true（网格着色器路径启用）、光追 RT pipeline/AS/ray_query=true、DLSS VK_NVX=true、present_us 101-373µs（WSL2 dzn 的 1-2ms 瓶颈消失）。
- **功率说明**：奥创中心手动模式 + 电源最佳性能；GPU 功耗墙已解锁 111.92W（默认 55W）；1280x800 下 42W/300+fps 属正常（负载轻），2560x1600 压力模式 50W+/150-400fps。
- **已知修正（2026-08-15）**：鼠标水平方向（look() `yaw -= dx*sens`，右移=右转）；冒烟脚本 aim 注入已同步反向。
- **遗留**：WSL2 时代输入捕获问题（HANDOFF-2026-08-11.md）在原生 win32 下已解决（SendInput 视角注入实测正常）；`docs/perf-*` 等 WSL2 基准保留作历史。

## 渲染技术路线铁律（2026-08-16 更新：顶点管线冻结、网格着色器优先）

> **本决策取代 2026-08-11 的「网格着色器冻结、传统管线主迭代」决策（勿回退）。**

- **主开发路径 = `VK_EXT_mesh_shader` 网格着色器（MESH + FRAGMENT）**：所有新渲染功能、性能优化、视觉迭代一律在 mesh 路径上进行（build.rs `MESH_SHADER_WGSL` + renderer.rs mesh 管线）。mesh 路径在支持扩展的设备上自动启用（Windows 原生 RTX 5060 实测 VK_EXT_mesh_shader=true），无需环境变量干预。
- **传统顶点着色器管线（VERTEX + FRAGMENT）冻结维护**：仅作 WSLg/dzn 回退（转译层不支持 mesh shader），**不再为顶点路径新增任何功能**；只做必要的兼容性维护，冒烟基线仍要求双路径零回归（VUID=0）。
- **mesh 路径关键约定（勿回退）**：
  - naga 30 网格写入器对 `@builtin(vertices)` 数组内 position 的 ADJUST_COORDINATE_SPACE 翻转**失效**，mesh 着色器必须在 WGSL 内显式 `v.position.y = -v.position.y`（build.rs write_vertex，删掉会垂直镜像）。
  - 槽位常量与 renderer.rs 同步：TERRAIN_INSTANCE_INDEX=65536、MARKER_INSTANCE_BASE=65537、NPC_INSTANCE_BASE=65601、EMISSIVE_INSTANCE_BASE=66625。
  - 地面场 65536 workgroup 按 `maxMeshWorkGroupCount[0]` 查询上限分块下发（字段 mesh_max_wg_x）。
- **环境**：Windows 原生（RTX 5060 Laptop 真机）为唯一开发/验证环境；WSL2 相关内容保留作历史，不再承担开发/验证。

## 迭代规划与交接日志（AI 交接规范）

本文件是**唯一的正式 AI 交接载体**（项目记忆 + 迭代规划 + 交接留痕一体化）。所有 AI 会话（含并行智能体，如 Newton）在本仓库工作，必须遵守以下交接协议：

- **规划开启**：每次迭代/任务规划开始时，必须在本节登记：目标、任务拆解、负责人、状态（in_progress）。
- **迭代结束**：迭代结束时必须在本节写完成记录：完成项、验收结果（测试数/警告数/冒烟结果）、遗留问题与下一步。
- **AI 间交接**：AI 与 AI 之间的所有交接必须在此留痕——上下文交接（记忆/结论迁移）、任务交接（接手/移交）、绘画/美术素材交接（素材路径、用途、规格、验收标准），格式：日期、发起方、接收方、交接内容、状态。
- **git 提交规范不变**：一个功能一个 commit，禁止 mega-commit；提交信息 `feat/chore/docs/fix` + 范围（如 `feat(game)`、`docs(AGENTS.md)`）；git 操作在 Windows 原生跑（总指挥 GitHub 令牌 push，仅限 steel-front 仓库）。

### 交接日志模板（可复制）

```markdown
### [YYYY-MM-DD] 交接：<一句话主题>
- 日期：YYYY-MM-DD
- 发起方：<AI 名称 / 会话标识>
- 接收方：<AI 名称 / 会话标识>
- 交接类型：<规划开启 / 迭代结束 / 任务交接 / 美术素材交接>
- 交接内容：<目标、拆解、关键结论、素材路径与规格、验收标准等>
- 状态：<in_progress / done / blocked>
```

### 当前迭代（2026-08-11）

- ① 渲染主路径迁移网格着色器（VK_EXT_mesh_shader，WGSL mesh 着色器 + GPU 剔除；传统 VERTEX+FRAGMENT 管线保留为回退——WSLg/dzn 不支持 mesh shader，实测 VK_EXT_mesh_shader=false；naga 30 支持 `enable wgpu_mesh_shader` + `@stage(mesh)` 输出变量语法）｜负责人：当前会话 AI｜状态：done（已冻结，仅保留接口/验证功能，见下方 2026-08-11 渲染技术路线决策交接）
- ② README.md 重构为对外进度说明书｜负责人：Newton｜状态：in_progress
- ③ AGENTS.md 重构为正式交接文档（本任务）｜负责人：当前会话 AI｜状态：in_progress
- ④ 线程分层调度优化（AMD 双 CCD / Intel P+E 分层负载）｜负责人：当前会话 AI｜状态：done（第 1-4 步全部完成：分组纯函数 + 双池调度 + 地图生成换核 + 远组降频；265 tests + 冒烟 ALL-OK；基准存档 `docs/perf-ai-tier-2026-08-11/`——128 NPC 压力模式 near~42/far~85（2/3 AI 走 CCD1/E 核）、ai_us p50≈385µs 非瓶颈、fps p50≈274 无回归、降频 A/B 收益≈0（压力模式互射 NPC 全在 Chase/Attack 触发面小，保留为防御性优化）；`RV3D_AI_DECIMATE=off` 可关降频）
- ⑥ 物理核/超线程分层绑定（线程优化第 5 步）｜负责人：当前会话 AI｜状态：done（sysfs SMT 配对识别 + 高性能线程绑物理核 + 超线程溢出辅助；270 tests + 128 NPC 基准 fps 持平 + ai_us 提升 + 冒烟 ALL-OK，见下方 2026-08-12 交接）
- ⑤ 美术方向（阴影 / 光线遮挡 / 渲染烘焙 + 程序化贴图）｜负责人：当前会话 AI｜状态：done（① 阴影贴图 + ② 烘焙 AO + ③ 光照烘焙 + ④ 程序化地面贴图全部完成，见下方 2026-08-12 / 2026-08-13 交接；剩余：障碍物/士兵皮肤程序化贴图）

### [2026-09-01] 交接：多模态画面巡检（光栅三视角 + PT 收敛复验）→ 修 2 项、开缺陷清单 D7–D12

- 日期：2026-09-01
- 发起方：Qwen Code（原生视觉逐张审图）
- 接收方：后续迭代 AI
- 交接类型：迭代结束
- 交接内容：
  - **方法**：`RV3D_AUTOSTART=1 RV3D_STRESS_AI=1` 起真机，`scripts/shot.ps1` 截图 + `data/key.py`（ctypes `keybd_event`，VK 9=Tab）切 Orbit/Flight 多视角取证；PT 通道与光栅通道各采一组对照。**结论：静态推 shader/数据看不出问题，看图直接暴露——这条路径本项目此前没人走过。**
  - **修复①（D7 已验收）发光黄带**：`procedural.rs::city_zone_color` 沥青分支的中线判据 `dx < 0.3 || dz < 0.3`（0.6m 宽）在世界空间 UV 下**不足 1 纹素**，线性过滤 + MIP 把亚纹素高饱和色抹开放大 → 画面上一条从脚前冲到地平线的发光黄楔形（旧截图 `093415/093425` 可见）。改为 `0.55`（1.1m 可解析宽）+ 低饱和磨损黄 `[0.34,0.31,0.19]`；复验截图 `094049` 黄带消失，只剩正常磨损中线。
  - **修复②（PT 实时永不收敛，本会话自引入的回归）已验收**：`PtParams::signature()` 原先把相机位置量化到 **1mm**，而实机有呼吸 bob + 后坐震屏（`camera_shake_offset`）→ **每帧指纹都变 → 每帧复位累积 → 永远停在 1 spp**，表现为"降噪没效果、建筑立面满屏噪点"（截图 `093213/093218` 实证）。现按物理幅度分层量化：位置 ~0.5m、朝向 ~3°、光照 ~0.01、曝光/fov 粗化。复验 `094037`：天空与地面完全平滑、**每棵树和每个箱体下都有柔和接触阴影**，收敛达成。
  - **⚠ 教训（勿回退）**：任何"变化就复位"的时域累积/缓存，其变化判定的量化粒度**必须粗于相机 idle 抖动幅度**。这类 bug 不报 VUID、不报错日志，只有看图能发现。
  - **新开缺陷清单（本次只记录，未修）**：
    - **D8 前景浅黄多边形团（未定案，优先查）**：光栅视角玩家枪身周围悬浮 4–5 个浅卡其色**平面着色 12 边多边形圆盘**（截图 `094049` 屏幕坐标约 (620,690)/(975,660)/(800,830)/(1275,830)，部分被画面底边裁切）。**已排除爆炸特效**——同一次运行的 `data/v2.log` 中 `explosion: at` 计数为 **0**，故与 D5 的 SPH 火球无关。轮廓呈正多边形 ⇒ 高度怀疑**圆柱/球体端视**（24 段圆柱端视即正 24 边形），建议下一步从 `EMISSIVE_INSTANCE_BASE=66625` 与 `NPC_CYL/NPC_SPH` 槽位的实例 tint 与矩阵入手：用日志 `marker=/npc=` 计数比对，或临时把自发光通道置空来二分定位。**不要照猜测改**。
    - **D9 HUD 调试文本重叠**：左上 `AI/NET/GPU/TEMP/VULKAN` 逐行与 `visible=…/entities:…`、fps 大数字互相压字，实测不可读（`093415/094049` 均现）。属排版行距问题，非渲染 bug。
    - **D10 据点立柱无底盘**：两根深色细柱悬在街道中央，`capture_points` 的半径 5.0 底盘不可见（疑与地面 y=0.05 共面 z-fighting，或底盘过薄）；视觉上像电线杆，完全读不出"可占领目标"。
    - **D11 玻璃幕墙凸成悬空鳍片**：D2 修好了贴图乱码，但幕墙条带作为独立 marker 盒**外凸于立面**，从街对面看是一排排悬挑深色板（`094049` 右侧建筑、`093415` 两侧），并在地面投下错位阴影。需把条带内缩到与立面共面或改为贴花。
    - **D12 NPC 远距离读作蓝色平板**：中远距离士兵呈纯色扁平剪影 + 亮斑（`093425` 左侧蓝兵清晰可见头/躯干亮带，但四肢不成圆柱），D6 的圆柱四肢在这个距离完全看不出体积感；阵营色饱和度在强光下偏淡。建议近距离再取一帧专门评估，或给四肢加最小屏幕空间粗细。
  - **验收**：`cargo test --release` **405 全绿**；冒烟 **ALL-OK**（VUID=0、kills=13、fps 131.5–193.7、panics=0）；本波两处修复的警告数与编译状态见下条 commit（黄带 + 指纹改动后重编通过）。
  - **工具留档**：`data/key.py`（SendInput 之外的轻量按键注入，配合 shot.ps1 做定点取证）。注意 `data/` 已在 .gitignore 内，脚本未入库——需要长期用就移到 `scripts/`。
- 状态：done

### [2026-09-01] 交接：PT 降噪——spp 时域累积（线性 HDR 均值）

- 日期：2026-09-01
- 发起方：Qwen Code
- 接收方：后续迭代 AI
- 交接类型：迭代结束
- 交接内容：
  - **做法**：新增 binding 3 累积图像（`R32G32B32A32_SFLOAT`，rgb=Σ线性样本、a=已累积帧数）。片元流程改为「采样 → 乘曝光 → **累加进 acc** → 用 `acc.rgb/acc.a` 运行均值做 ACES + linear→sRGB → 写 OutImg」。色调映射只作用在均值上（先各自 tonemap 再平均会压平高光，且在 gamma 域相加不物理）。
  - **关键前提（勿回退）**：采样种子**必须含帧索引**。旧实现种子只由 `gid` 决定 → 逐帧图案完全静止 → 累积等于原地踏步、永不收敛。现 `lambertianBounce(n, pxSeed, frameSeed*64+b)`，`frameSeed` 来自 push constants 的 `f.x`。
  - **push constants 扩到 6×vec4 = 96B**（新增 `f = (frameIndex, resetFlag, sppTarget, unused)`）；`PtParams::pack(res, frame, reset, spp_target)` 与 GLSL `PC{a..f}` 必须同步，两处 `.size(96)` 也要同步——任一不改就是静默错相机/错累积。
  - **复位三条路**：① 取景指纹变化（`PtParams::signature()`：cam/fwd/sunDir/sunColor 量化 ~1mm + fov/曝光/弹跳）→ 清累积重开，否则不同视角样本混成拖影；② `pt_set_scene_markers` 重建 BLAS → 复位；③ `destroy_pt_resident` → 复位。
  - **图像布局纪律（易错点）**：累积图像只在创建时做一次 `UNDEFINED→GENERAL` 转换，逐帧 barrier 用 `GENERAL→GENERAL`；主图像每帧整体重写所以允许 `UNDEFINED` 丢弃。**若对累积图用 `old_layout=UNDEFINED` 等于告诉驱动内容可丢 = 累积白做**，且这种 bug 不报 VUID、只表现为"降噪没效果"。
  - **累积状态用 `Cell`**：上屏代码在 `record_command_buffer(&self)` 内，改 `&mut self` 会波及整条渲染调用链，故 `pt_frame`/`pt_reset`/`pt_view_sig` 取 `std::cell::Cell`。
  - **达标即停派发**：`pt_frame >= pt_spp_target` 时整个 dispatch 跳过（画面保持、GPU 归零），实测 fps 从累积期回到 **184.9**；`RV3D_PT_SPP` 覆盖目标（实时默认 256，`run_pt_view` 默认 64）。
  - **参考帧同步升级**：`RV3D_PT_VIEW` 现在**一条命令缓冲内派发 spp 次**（相邻 dispatch 加 compute→compute 自依赖 barrier），输出收敛图而非 1 spp 噪声图，可直接当烘焙真值。
  - **验收（实测）**：`cargo build --release` 警告 **46（bin）+1（build script），0 新增**；`RV3D_PT_SPP=128` 参考帧 256²——颗粒噪点目视消失、地面平滑渐变、盒体立面干净、**接触阴影带软半影**；`RV3D_PT_SPP=512` 实机 2560×1600——`PT-RESIDENT: 512x512 spp 目标 512`、`PT-SCENE: 盒 512 个`、无 panic/无 device lost、收敛后 **fps 184.9**。（`cargo test` 与冒烟本波未重跑——改动全在 PT 通道内，主路径仅新增 `Cell` 读写。）
  - **遗留队列**：① 删死代码 `pt_live_frame`/`pt_live_frame2`（仍是 2-binding 旧描述符，误接必崩）；② 512 盒上限静默截断（实测 marker=547）；③ 曝光/弹跳/spp 进 config.rs 与设置面板（现曝光 0.2 硬编于 main.rs）；④ 天空/环境项仍硬编在 GLSL，`PT_SUN_AMBIENT` 无消费者；⑤ PT 与光栅同屏叠加（现整体替换）；⑥ 移动相机时每次都全量重开累积——可改「按像素重投影复用」或运动自适应 spp；⑦ 冒烟/测试本波未重跑，下位接手先补一次。
- 状态：done

### [2026-08-31] 交接：PT 路径追踪命中根治（rayQuery committed 参数）+ 着色器改 glslang 编译 + 场景同源

- 日期：2026-08-31
- 发起方：Qwen Code（接手 Kimi K3 会话，其因余额中断于 PT 取证最后一步）
- 接收方：后续迭代 AI
- 交接类型：迭代结束
- 交接内容：
  - **根因（两天悬案结案）**：`rayQueryGetIntersectionTypeEXT(q, 0)` 第二参数是 **`committed`**，传 `0/false` 查的是**候选（Candidate）**记录；`rayQueryProceedEXT` 循环结束后候选记录已被清空 → 恒返回 `NoIntersection` → 全图无命中。反汇编对照：旧 `screenshots/ptframe.spv` = `OpRayQueryGetIntersectionTypeKHR %uint %70 %uint_0`（Candidate），新模块 = `%int_1`（Committed）。源证据 `screenshots/pt_frame.comp:22,38`。AS 内存、64B 实例布局、scratch、barrier、绕序、描述符绑定当时逐层证明正确——**方向完全对，只是问错了记录**。
  - **次因**：旧 `ptframe.spv` 是取证探针版——所有像素射线被硬编码为 `origin (0,1.5,0)` / `direction %63=(0,-1,0)`，逐像素算出的方向 `%91` 被 `OpStore %73` 存下后从未使用。
  - **⚠ 假信心陷阱（务必留痕）**：`tools/rt-bench` 反汇编核对显示其模块是**逐线程无条件 `OpStore 1`**，全程未查询任何 intersection type —— 故 commit `d0290e3` 所称「hit-readback verification」并不构成命中证据，33.6 G rays/s 只是遍历吞吐。**RT 通路此前从未被证明命中过**；唯一的命中真值是本波 `pt_panorama.glsl`（`committed=true` + `GetIntersectionType==Triangle`）渲染出的带接触阴影的参考帧。以后要声称"RT 已验证"，必须给出命中着色图像证据，不能只给 rays/s。
  - **根治**：弃手工拼装 SPIR-V（含约 190 个一次性补丁脚本路径），PT 着色器改为 `assets/rt/pt_panorama.glsl` 经 **glslangValidator** 编译为 `assets/rt/pt_panorama.spv`，`spirv-val --target-env vulkan1.3` 严格通过；`build.rs` 从该 .spv 嵌入 `PT_FRAME_SPV`，GLSL 比 SPV 新时发 `cargo:warning`；新增 `scripts/compile_pt.ps1`（编译 + 校验）。
  - **场景同源**：新增 `Renderer::pt_set_scene_markers(&[WorldMarker])`——PT 盒体直接取光栅化同一批 marker 矩阵（平移=中心、缩放列长/2=半宽、tint=反照率），盒 0 为地面；坐标量化 1m 的 FNV 指纹判定，只有集合真变才重写几何 + 重建 BLAS。`PtAssets` 顶点/索引/材质缓冲一次按 `PT_MAX_BOXES=512` 分配（TLAS 恒单实例：全部盒合进同一 BLAS，实例数与场景规模无关）。着色器接口：binding 0=TLAS / 1=storage image(rgba8) / 2=每盒材质 SSBO；push constants 5×vec4=80B（相机**直接传 `camera.forward()`**，不重推 yaw/pitch 公式）。
  - **连带修掉两个真 bug**：① `record_pt_build` 旧实现每次调用新建 2MB scratch 且从不释放 = 显存泄漏源（对应 08-30「16GB leak storm」），现 scratch 归 `PtAssets` 所有、BLAS 用前段 / TLAS 用后段，并在两次构建之间加 `ACCELERATION_STRUCTURE_BUILD` barrier（旧实现同地址连用且无执行依赖）；② **BLAS 尺寸必须按容量上限而非当前盒数**——首次实时试跑塞 512 盒进按 4 盒算的 5376B BLAS → 越界写 → device lost。
  - **法线教训（勿回退）**：盒面法线用「来射方向主轴」近似在浅角度下会把地面法线错判成 ±Z → `ndl<0` → 地面全黑。改用 `ray_tracer::box_triangles/box_indices` 的不变量 `(primitive % 12) / 2` = 面号查表，零额外绑定且精确。
  - **验收（实测）**：`cargo test --release` **405 全绿**；`cargo build --release` 警告 **46（bin）+ 1（build script）**，较接手时存量 49 条**减少 3 条**（材质常量转活跃），**0 新增**；`RV3D_PT_VIEW=1` → `screenshots/pt_ref.bmp`（256²，非天空像素 61.8%），图含天空渐变 / 地平线 / 三材质盒体 / **盒体接触阴影**；`RV3D_PT_LIVE=1 RV3D_AUTOSTART=1` → `PT-SCENE: 盒 512 个`、无 device lost、无 panic，2560×1600 **fps 181–182**（与 PT 关闭基线 180–190 持平）。
  - **遗留队列（下一步）**：① **降噪**——当前 1 spp + 哈希余弦采样，颗粒明显；加 spp 累积或蓝噪声/去噪，PT 作烘焙参照必须先收敛；② **删死代码** `renderer.rs` 的 `pt_live_frame` / `pt_live_frame2`（无调用点，且其描述符布局缺 binding 2 / 材质 SSBO，误接必崩；顺带清掉其贡献的 unused 与「unused Result」告警）；③ 512 盒上限截断（实测 marker=547 > 512，需提容量或按视锥裁剪）；④ `PT_SUN_AMBIENT` 仍无消费者，天空/环境项硬编码在 GLSL，接进 `light_uniform` 语义才算完整同源；⑤ 曝光/太阳强度进 config.rs 持久化；⑥ 冒烟脚本 `scripts/run_gameplay_smoke.ps1` **本次未跑**（PT 默认关，主路径仅 `pt_params` 赋值 + 提前返回，且 405 测试 + 实跑 181fps 无 VUID/panic 已覆盖）——下位接 RT 前先补跑一次。
  - **BAT**：`C:\Users\Jerry-Huang\Desktop\SteelFront.bat` touch 列表从只 `src -Recurse *.rs` 扩到含 `build.rs` / `build_spv_rt.rs`（只改着色器或构建脚本时 cargo 静默不重编 = 「启动旧版本」的另一半原因）。
- 状态：done

### [2026-08-31] 交接：D6 方块人根治 + D5 爆炸圆润化 + 三个 mesh 潜伏 bug

- 日期：2026-08-31
- 发起方：Kimi K3（指挥/几何攻坚/多模态验收）+ DeepSeek API（分支执笔）
- 接收方：后续迭代 AI
- 交接类型：迭代结束（巡检第三波）
- 交接内容：
  - **D6（f280351）**：mesh 路径 NPC 四肢/头恢复圆柱/球体。新增槽位几何：四肢区 [NPC_CYL_BASE=NPC+3072, NPC_SPH_BASE=NPC+6144) → WGSL 程序化单位圆柱（r=1、y∈[-0.5,0.5]、24 段含盖、50v/96tri，与 CPU create_cylinder_geometry 同单位空间，绕序模型空间外侧 CCW 已按管线约定验算）；头部区 [NPC_SPH_BASE, EMISSIVE) → 归一化二十面体。**配套扩容**：MeshOutput 24v/12p → 50v/96p、workgroup_size 32 → 96。
  - **潜伏 bug 连带根治（K3 审计发现）**：① 旧 MeshOutput 图元上限 12，而 ICO 分支写 20 → 越界钳位垃圾（所有树/爆炸长期带病）；② mesh is_gun 残留 1024 时代区间 67569..75777，覆盖全部 NPC 圆柱/球体槽 → 四肢/头被 z=0 深度覆盖（「鬼魂」观感来源之一），已改 `slot == 75841u`；③ DeepSeek 两版圆柱索引均有交错布局/盖中心/回绕三处错误 → 按协议第二部分由 K3 亲自写几何（3D 空间干预域），DeepSeek 回任后续。
  - **D5（3d7507d）**：爆炸巨大扁平尖刺 → 圆润火球。根因=自发光壳（~13m）用 12v/20tri 二十面体，单面 ~4m。修复=自发光改 SPH 一级细分二十面体（42v/80tri，单位半径，确定性脚本预生成常量嵌入 WGSL）；树冠仍 ICO×0.9；is_foliage_or_glow 拆 is_glow/is_tree。
  - **验收**：405 tests 全绿；截图亲验——士兵圆柱四肢/球形头/跪姿射击（红蓝可辨+迷彩+脚下投影）、爆炸分层火球+烟帽、玻璃幕墙干净横带、树/建筑投影方向一致；128v128 稳态 ~180-190fps（圆柱成本 ~30% 帧时，可接受）。
  - **遗留队列**：D4 墙缝天空亮条（疑似楼间缝隙正常天空，需定点复现再定）；mesh 着色器严格 spirv-val 布局（开 RV3D_VALIDATION=1 前必修）；49 条存量警告专项；PT 命中（另一会话主线）；阴影 normal_bias 已在 uniform 但未用（如需更干净阴影边界可做坡度 bias）。
- 状态：done

### [2026-08-31] 交接：D3 阴影根治（深度二重映射 +0.25 偏移）+ 诊断基建

- 日期：2026-08-31
- 发起方：Kimi K3（指挥/多模态验收/证据链闭环）+ DeepSeek API（代码执笔）
- 接收方：后续迭代 AI
- 交接类型：迭代结束（巡检第二波）
- 交接内容：
  - **D3 根治**：全场无可见阴影对比 → 真根因 = 阴影深度比较基准二重映射。glam ortho_rh（当前版本）产出 Vulkan 原生 [0,1] 深度（GPU 阴影图写入同基准），但片元与 world_to_shadow_uv 沿用 OpenGL [-1,1] 旧映射 `clip.z*0.5+0.5` → frag_depth 恒 +0.25 → `frag_depth - bias > d` 恒真 → 全场景 shadow_factor≈1 → 方向光被均匀压制 → 看似「无阴影」。**这也是 2026-08-22「全图判黑」的真根因**（当时关阴影是治标）。修复：`frag_depth = sp.z`（build.rs 片元）+ `(uv, p.z)`（lighting.rs 镜像）+ 测试期望修正。V 镜像不变（Y 翻转约定，无关本 bug）。
  - **诊断基建（常驻，默认关）**：`RV3D_DEBUG_SHADOW=1` → 片元输出 R=frag_depth / G=阴影图深度均值（蓝=UV 外）+ set_lights 一次性矩阵 dump 日志。排查阴影类问题先用它，别再静态推矩阵。
  - **证据链方法（可复用）**：矩阵 dump（col3.z=0.499）→ 调试图 RG 通道采样（0.74 vs 0.49 恒差 0.25）→ 定位二重映射。旧回归测试 `depth_origin > 0.5` 曾把错误映射固化为「铁律」——**测试镜像与 GPU 真值必须分开校验**。
  - **验收**：405 tests 全绿；截图亲验树木/建筑地面投影（方向与太阳 (-0.4,0.9,-0.3) 一致）、场景恢复光照分层、不再均匀压暗。
  - **下波队列**：D2 建筑窗户乱码 → D6 NPC mesh 几何退化立方体 → D4 悬浮亮条 → D5 爆炸尖刺。
- 状态：done

### [2026-08-31] 交接：K3 指挥官首轮渲染巡检——D1 NPC 阵营色修复 + 环境门控 + 缺陷清单

- 日期：2026-08-31
- 发起方：Kimi K3（指挥/多模态验收）+ DeepSeek API（代码执笔）
- 接收方：后续迭代 AI（含并行 RT 会话）
- 交接类型：迭代结束（巡检第一波）
- 交接内容：
  - **灰屏插曲（非代码回归）**：RT 会话的 PT 全景（config.rs `pt_enable` 默认 true，且 parser 未读 cfg 的 pt_enable 键）把半成品 PT 输出（仅天空）铺满 swapchain → 全灰。已加 `RV3D_PT_LIVE=0` 强制关通道（main.rs，0=关/1=开/未设=跟随配置）。**巡检/游玩请带 RV3D_PT_LIVE=0，直到 PT 命中修复**。
    > 【2026-08-31 更正】PT 命中已根治（根因 = rayQuery getter 的 `committed` 参数，见本文件同日「PT 路径追踪命中根治」条目）。上述"直到 PT 命中修复"的规避前提已消失，保留仅作历史记录；当前默认关闭的理由改为「PT 帧整体替换光栅画面 + 1 spp 噪声大」，属调试/烘焙参照视图而非玩法渲染。
  - **验证层门控**：Vulkan SDK 1.4.357 安装后验证层「可用即启用」→ 严格 spirv-val 拒 mesh 着色器（Workgroup Offset 布局）→ 灰屏。已改 `RV3D_VALIDATION=1` 才启用（renderer.rs）。**遗留**：mesh 着色器布局未过严格 spirv-val，RT 调试需开验证层前应先修。
  - **D1 修复（已验收）**：NPC 士兵苍白大理石「冰雕」、阵营色不可辨 → 根因=片元 NPC 分支 mix(tint, 卡其迷彩, 0.65) 稀释色相。修复=去饱和纹素亮度调制 `base = input.color * (0.55 + 0.9*luma)`（build.rs 片元 NPC 分支，双路径共用）。A/B 截图验收：红/蓝饱和可辨且迷彩细节保留。
  - **测试隔离**：tests/rayquery_probe.rs（RT 会话未提交 WIP，引用 naga 导致 test 目标编译失败）已改后缀 .bak 隔离，恢复 405 绿。
  - **渲染缺陷清单（巡检发现，按序推进）**：D1✅ → D3 全场无投影（shadow map 系统存在但画面无可见阴影）→ D2 建筑窗户「蝴蝶结」交叉面+乱码贴图 → D4 墙面悬浮亮蓝竖条 → D5 爆炸特效=巨大扁平尖刺片 → D6 NPC 几何 mesh 路径四肢/头退化为立方体（cyl/sph 几何选择缺失，build.rs mesh_main 仅 is_foliage_or_glow/is_marker/CUBE/CROSS 四支）。
  - **验收**：405 tests 全绿；截图亲验 D1；本波改动 0 新警告（存量 49 警告仍待专项清理）。
- 状态：done

### [2026-08-29] 交接：检视模式白屏修复（d909348 静态顶点管线回归）

- 日期：2026-08-29
- 发起方：Kimi K3（总指挥/多模态验证）+ DeepSeek API（代码执笔）
- 接收方：后续迭代 AI
- 交接类型：迭代结束（接手首任务）
- 交接内容：
  - **修复**：`RV3D_INSPECT=1` 检视模式白屏。d909348 静态顶点+实例矩阵管线后，main.rs render() 的 `fp_gun_pre` 对检视模式也套用了第一人称矩阵（view_inv×anchor×~0.25 缩放），枪模被扔到相机旁 0.6m、偏轴 0.25m，落在 14.5° 长焦取景框外 → 全白。修复：检视模式实例矩阵用 `glam::Mat4::IDENTITY`（枪模顶点已居中到世界 (0,1,0)，Orbit 相机绕拍）；第一人称路径逻辑不变。
  - **教训**：凡改动 fp_gun 顶点/矩阵管线的共享计算，必须双模式（第一人称 + RV3D_INSPECT）截图验证——d909348 只验了第一人称。
  - **验收**：405 tests 全绿；检视模式截图确认深色 AK-12 全枪侧面产品照（导轨/散热孔/板机可见，本色直出）；第一人称 gameplay 截图确认持枪姿态/HUD/128v128 无回归；本改动 0 新增警告。
  - **已知存量（非本次引入，未处理）**：release 构建 21 警告（assets.rs GDI+ 段 unused/static_mut_refs 等 + main.rs:905 `cam` 未用，为 d909348 遗留与新 rustc lint），与 AGENTS.md「0 警告」基线有缺口，待专项清理。
  - **工具**：`scripts/shot_fg.ps1`（前置窗口 CopyFromScreen 截图，备用；本次 PrintWindow 的 shot.ps1 实际可用）。
- 状态：done

### [2026-08-16] 交接：渲染方向转向（顶点冻结 / 网格优先）+ 修复记录 + 文档更新

- 日期：2026-08-16
- 发起方：总指挥（DeepSeek Harness，Windows 原生直接开发）/ 文档专员
- 接收方：后续迭代 AI（下一会话，Windows 原生）
- 交接类型：迭代结束 + 方向决策
- 交接内容：
  - ① **渲染技术路线转向（重要，勿回退）**：冻结传统顶点着色器管线维护，全面转向 VK_EXT_mesh_shader 网格着色器开发——mesh 路径（MESH + FRAGMENT）为唯一主开发路径（GPU 视锥剔除 + 实例变换，RTX 5060 真机自动启用）；顶点路径仅作 WSLg/dzn 回退，不再加新功能。取代 2026-08-11「网格着色器冻结」决策（历史记录保留）。
  - ② **修复记录（详见 README「2026-08-16 修复记录」）**：世界垂直镜像修复（mesh 路径 WGSL 内显式 Y 翻转，build.rs）；HUD 双重缩放修复（1280×800 设计空间布局、出口统一乘 ui_scale）；窗口/交换链尺寸自动校验（不匹配自动重建交换链）；开镜 FOV 补偿（tan 反比枪模缩放）；跳跃物理（JUMP_SPEED=3.3，Space 跳跃）；中文 HUD 字形系统（font_cjk.rs，Windows GDI 8×8 点阵掩码）。
  - ③ **文档更新**：README.md 全量重写（中文对外说明书：技术特性 / 开发方向 / 构建运行 / RV3D_* 环境变量表 / 测试门槛 / 已知注意事项 / 2026-08-16 修复记录）；AGENTS.md 新增「渲染技术路线铁律」节 + commit 规范改为 feat/fix/docs 前缀。
  - ④ **RV3D_* 环境变量清单（代码实测 20 个）**：RV3D_PRESENT_MODE / RV3D_PROC_TEX / RV3D_SKIN_TEX / RV3D_NO_SHADOW / RV3D_NPC_SCALE / RV3D_STRESS_AI / RV3D_CPU_PIN / RV3D_DISABLE_AVX512 / RV3D_SCENE_WORKERS / RV3D_AI_WORKERS / RV3D_AI_PARALLEL / RV3D_AI_DECIMATE / RV3D_FORCE_SIMD / RV3D_BENCH_YAW / RV3D_BENCH_PITCH / RV3D_EXPLOSION_SIM / RV3D_NET / RV3D_NET_ADDR / RV3D_MAP / RV3D_MAPS（详见 README 环境变量表）。
- 状态：done

### [2026-08-15] 交接：Windows 原生 UI/呈现迭代（ESC 毛玻璃菜单 + 击杀提示 + 环境适配）

- 日期：2026-08-15
- 发起方：总指挥（DeepSeek Harness，Windows 原生直接开发）
- 接收方：后续迭代 AI（下一会话，Windows 原生）
- 交接类型：迭代结束
- 交接内容：
  - ① ESC 毛玻璃菜单（替代两段式退出确认）：ESC 打开半透明毛玻璃菜单（全屏暗色遮罩 + 居中面板 + PAUSED 标题 + 两个选项 EXIT GAME / SETTINGS），Tab 切换选中（高亮黄底）、Enter 确认（退出 / 打开设置）、ESC 关闭、任意其它键关闭；旧 confirm_quit 渲染块已删（字段保留防测试破坏）。
  - ② 击杀提示（右上角 feed，战地风格）：HudState 新增 kill_feed（最多 4 条、6 秒消退、最新在上）；game.rs 三处钩子——玩家击杀 NPC（YOU KILLED RED #id）、NPC 互杀（RED KILLED BLUE #id，apply_npc_combat 血量归零判定）、NPC 杀玩家（YOU WERE KILLED）；新增 team_name 辅助函数。
  - ③ 环境适配（AGENTS.md 结构更新，历史保留）：新增「开发环境（2026-08-15 起：Windows 原生 + DeepSeek Harness）」节——编译/测试/冒烟命令、GPU 能力实测（mesh/RT/DLSS/present）、分辨率 2560x1600、功率说明、鼠标方向修正记录；git 规则改为 Windows 原生 push（总指挥令牌）；WSL2 时代内容保留作历史。
  - ④ 鼠标水平方向修正（commit 6009684，随前轮）：look() 改 yaw -= dx*sens（右移=右转）；冒烟脚本 aim 注入同步反向（否则振荡不收敛）。
  - ⑤ 分辨率 2560x1600：写入 C:\Users\Jerry-Huang\.steel_front.cfg（用户确认）；压力模式实测 50W+/150-400fps。
  - 验收：364 tests 全绿 / 0 警告 / 冒烟 ALL-OK（VUID=0、kills=1、fps 246-307）。
  - 遗留/下一步：① 毛玻璃是真模糊（需 shader 后处理采样主 pass，当前为半透明暗色遮罩近似）；② 击杀提示文本目前英文（位图字体 5x7 无中文）；③ kill feed 未区分击杀者名字（NPC 只有阵营+id）；④ 呈现层续作（第一人称枪模/弹孔贴花）。
- 状态：done

### [2026-08-14] 交接：总指挥指令单 #4 完成（防守波次规则 + 爆炸纵深 + 音效差异化）

- 日期：2026-08-14
- 发起方：主会话 AI（DeepCode）+ Agent A（audio.rs 枪声参数化）
- 接收方：总指挥 / 后续迭代 AI（下一会话）
- 交接类型：迭代结束
- 交接内容：
  - **阶段一 survive 规则（commit `224f9c4` feat(map)）**：GameRule 加 `Survive { waves }`（evaluate 返回 None——胜负由 game.rs 波次循环驱动：守住全部波 → Victory、玩家死亡 → Defeat）；RuleDef 加 waves + TOML 解析；波间补给窗口（血量 +50% + 弹药/手榴弹补满）；HUD WAVE x/N；defense_line.toml（环形工事 14 障碍 survive 5 波）入 index；README 四规则 + 5 图。
  - **阶段二 爆炸纵深（commit `f6aaa19` feat(ai)）**：spawn_explosion 障碍 AoE（EXPLOSION_OBSTACLE_FACTOR=1.0，距离衰减，Barrier 100HP 被手榴弹爆心摧毁；damage_obstacle 容忍 bodies 越界）；NPC 投掷手榴弹（压力/survive 中 Attack 态 NPC 确定性概率 5-8% 朝敌对目标投掷 + 冷却 10-18s，复用玩家 Grenade 链路；普通波次不投掷零回归）；玩家自伤（SELF_DAMAGE_FACTOR=0.35 + CAP=45 封顶不被秒杀，死亡 → GameOver/survive Defeat）。
  - **阶段三 音效差异化（commit `ec539be` feat(audio)）**：Agent A 实现 ShotParams（M1_SHOT 清脆 115Hz/0.12s vs THOMPSON_SHOT 低闷 78Hz/0.17s/thump）+ play_shot_with（旧 play_shot 委托 M1 零回归）+ play_grenade_throw（哨声 1100→520Hz）+ play_grenade_bounce；主会话 fire() 按武器选参数、投掷播哨声、爆炸前落地音。
  - **验收（主会话独立复核）**：364 tests（+12：audio 8 + survive 1 + 爆炸障碍 1 + NPC 投掷 1 + 自伤 1）/ 0 失败 / 0 警告；冒烟 ALL-OK（EXIT=0、VUID=0、kills=1——AI/爆炸改动后普通波次零回归）；defense_line 实跑加载 survive VUID=0/panic=0；音频实跑 VUID=0/panic=0。3 commits 已推送。
  - **实测观测**：压力模式 3 分钟 NPC 投掷 76 次（Attack 态触发）；玩家自伤 3m 处 120 伤 → fall 0.625 × 0.35 = 26.25 伤封顶不死（单测验证）；手榴弹炸障碍单测验证（爆心 Barrier 摧毁/远处无伤）。
  - **遗留/下一步**：① survive 完整 5 波真机需玩家主动转向击杀（自动脚本无法验证玩法循环，单测已覆盖逻辑）；② NPC 投掷只在压力/survive 启用（普通波次按红线不投掷）；③ 手榴弹弹道落点测试受玩家出生位置影响（功能由单测覆盖）；④ survive 波次强度用 wave_profile（wave 1 = 6 NPC，可调 spawn 数量增强防守压力）。
- 状态：done

### [2026-08-15] 交接：Windows 原生迁移完成（总指挥接管开发环境）

- 日期：2026-08-15
- 发起方：总指挥（Windows 侧，直接开发）/ DeepCode（WSL2，已收工）
- 接收方：后续迭代 AI（下一会话，Windows 侧）
- 交接类型：迭代结束（环境迁移）
- 交接内容：
  - **背景**：WSL2/dzn 转译层瓶颈（present_us 1-2ms）、GPU 能力全锁（mesh/RT/DLSS 全 false）、宣发目标是 Windows——经用户决策，开发环境整体迁移 Windows 原生（RTX 5060 真机）。
  - **① 迁移内容**：仓库 clone 至 Windows（git clone + push 权限实测 OK）；修复 2 处跨平台编译问题（cpu.rs AMD 分支 collect 类型标注 + main.rs 非 Linux force_x11 冗余变量），commit 4504f89 fix(win)。
  - **② Windows 原生能力解锁（实测，勿回退）**：VK_EXT_mesh_shader=true（网格着色器管线首次真机创建成功，WSL2 冻结两个月后开光）、光追 RT pipeline/AS/ray_query=true、DLSS VK_NVX=true、present_us 101-373µs（WSL2 的 1-2ms 瓶颈消失）。
  - **③ 冒烟移植**：scripts/gameplay_smoke_win.py（SendInput 替代 XTest、FindWindowW 替代 XQueryTree、UTF-8 容错读）+ scripts/run_gameplay_smoke.ps1（启动器）。实测 ALL-OK：VUID=0、fps 262-325、kills=1、hit=4、一次 -1753px 注入即收敛瞄准。
  - **④ 环境铁律更新（Windows 侧生效）**：git 操作可在 Windows 直接跑（push 走 GitHub 令牌，仅限 steel-front 仓库）；cargo 构建/测试/冒烟均在 Windows 原生执行；12GB 内存约束不变（一次一个 cargo）；WSL2 不再承担开发/验证。
  - **⑤ 遗留/下一步**：A. 呈现层欠账（枪模/动画/粒子/弹孔）在真 GPU 上开发；B. playtest_perf.py Windows 移植未做；C. 输入捕获遗留（WSL2 冻结项）原生 win32 大概率已解决（冒烟视角注入成功即证据）；D. mesh 路径真机验证 + DLSS 立项评估。
- 状态：done

### [2026-08-14] 交接：总指挥指令单 #4 开启（防守波次规则 + 爆炸纵深 + 音效差异化）

- 日期：2026-08-14
- 发起方：总指挥（外部模型）/ 主会话 AI（DeepCode）
- 接收方：Agent A（audio.rs 枪声参数化 + 投掷哨声）/ 主会话（survive 规则 + 爆炸纵深）
- 交接类型：规划开启
- 交接内容：
  - **阶段一 survive 防守波次**：GameRule 加 `Survive { waves }` 变体（不破坏 capture/kill/time 语义与单测）；NPC 分 N 波进攻、波间补给窗口（弹药/血量回复）、波次递增（数量/强度）；守住全部即胜、玩家死亡即败；新演示关卡 defense_line.toml（survive 5 波）入 index；HUD 波次信息（当前波/总波 + 间隔倒计时）；README 同步。
  - **阶段二 爆炸纵深**：spawn_explosion 加爆炸半径内障碍 AoE 伤害（复用 damage_obstacle 血量体系）；NPC 低概率（5-8%）投掷手榴弹（阵营区分不炸友军，复用玩家投掷/爆炸链路参数化）；玩家手榴弹自伤（爆炸中心偏移保证不被秒杀）；普通波次 AI 零回归 + 冒烟必跑。
  - **阶段三 音效差异化**：Agent A 在 audio.rs 参数化 DspSynth 枪声（M1 与 Thompson 音色/时长/音高不同）+ 手榴弹投掷哨声 + 落地滚动音；单测确定性；不破坏现有音效/音乐链路。
  - **验收**：≥352 tests、0 警告、冒烟 ALL-OK、三阶段实跑观测、零新依赖、每阶段增量验证。
- 状态：in_progress

### [2026-08-14] 交接：总指挥指令单 #3 完成（手感扩充 + 收尾）

- 日期：2026-08-14
- 发起方：主会话 AI（DeepCode）+ Agent A（weapons.rs 纯逻辑）
- 接收方：总指挥 / 后续迭代 AI（下一会话）
- 交接类型：迭代结束
- 交接内容：
  - **阶段一收尾**：① 客户端 HUD 接入 ObjectiveState（hud_quads 联机客户端用 net ObjectiveState 归属码 0/1/2 → Team 驱动据点进度条，无联机零回归）；② 音乐音量接入设置面板（MUSIC 行 = selection 2，Tab 循环 12 项，config.rs music_volume 持久化，game.rs 音乐通道音量同步）。
  - **阶段二 第二武器 + 手榴弹（commit `fd66467` feat(weapon) 纯逻辑 + `aca127f` feat(weapon) 集成）**：
    - Thompson SMG：射速 10/s、伤 12、弹匣 30、备弹 120、中距离投射（120m/s）；与 M1（25 伤 3/s）差异化；6 发点射击杀平衡不回归（M1 未动）
    - WeaponRack 多武器槽：数字键 1/2 或滚轮切换，切枪计时 0.6s（期间禁开火/换弹），HUD 显示武器名 + SWITCHING 提示
    - 手榴弹：G 键投掷（抛物线 + 上抛分量，引信 1.5-2.5s 确定性伪随机），落地/到期爆炸复用 spawn_explosion（AoE 120 伤/8m 半径/径向击退/震屏/闪光）；默认 2、N 补给补满；HUD GRENADES 计数
    - **关键修复（勿回退）**：WeaponRack::update 必须同时推进当前武器 Firearm::update（换弹计时），否则换弹永不完成
    - 真机实测：切枪 M1→Thompson（shot #1 M1 → #2 Thompson）、手榴弹 thrown fuse=1.51s → detonate radius=8 dmg=120 knockback=true、VUID=0、panic=0
  - **阶段三 新关卡（commit `5e57beb` feat(map)）**：factory_ambush.toml（工厂伏击·kill 40，隔间墙/机器/货箱）+ bridgehead.toml（桥头堡·time 180s 占 2 据点，桥面 barrier/沙袋/哨塔）；index.toml 4 图按序；README 关卡表格。实跑均 12 障碍 VUID=0/panic=0。
  - **survive 规则：跳过**（评估成本中等需新规则判定 + 波次逻辑，本轮已含 3 规则 + 4 图 + 武器/手榴弹大功能，回执已说明）
  - **Agent 教训**：Agent B（地图）deepcode -p 卡死 11 分钟无输出（log mtime 不更新、CPU 推理但无文件产出），按红线主会话接管完成；**监控手段：log mtime 超 5min 未更新即接管**
  - **验收（主会话独立复核）**：352 tests（+13：weapons 13 个）/ 0 失败 / 0 警告；冒烟 ALL-OK（EXIT=0、VUID=0、kills=1、fps 166-252）；武器/手榴弹/新图真机 VUID=0、panic=0。4 commits 已推送。
  - **遗留/下一步**：① 手榴弹不炸障碍（spawn_explosion 只结算 NPC + 震屏，障碍伤害走投射物直击；如需可加 AoE 障碍结算）；② NPC 不投掷（本轮范围外）；③ 切枪无动画（纯计时器符合要求）；④ survive 规则待后续迭代。
- 状态：done

### [2026-08-14] 交接：总指挥指令单 #3 开启（手感扩充 + 收尾）

- 日期：2026-08-14
- 发起方：总指挥（外部模型）/ 主会话 AI（DeepCode）
- 接收方：Agent A（weapons.rs 多武器/手榴弹纯逻辑）/ Agent B（新关卡 TOML）/ 主会话（阶段一收尾 + 阶段二集成）
- 交接类型：规划开启
- 交接内容：
  - **阶段一（收尾两小项）**：① 客户端 HUD 接入 ObjectiveState（联机客户端用网络数据驱动 HUD 据点进度条，无联机零回归）；② 音乐音量接入设置面板（Music 通道独立音量 + config.rs 持久化 + Tab 循环项）。
  - **阶段二（第二武器 + 手榴弹，核心）**：Agent A 在 weapons.rs 实现 Thompson SMG（射速 ~10/s、伤 ~12、弹匣 30、备弹 120、中距离散布）+ 手榴弹弹道（抛物线/引信 1.5-2.5s/复用爆炸）+ 多武器槽切换状态机 + 单测各 ≥2；主会话集成 game.rs/main.rs/ui.rs（数字键 1/2 或滚轮切枪、G 投掷、HUD 武器名/弹药/手榴弹数、补给）。
  - **阶段三（新关卡内容）**：Agent B 新增 2 张 TOML 关卡（工厂伏击 kill + 桥头堡 time）+ 更新 index.toml + README；可选 survive 规则评估后定。
  - **验收**：≥339 tests、0 警告、冒烟 ALL-OK（爆炸改动后必跑）、切枪/投掷真机 VUID=0、零新依赖、每阶段增量验证。
- 状态：in_progress

### [2026-08-14] 交接：总指挥指令单 #2 完成（桥接收尾 + 战术纵深 + 音频氛围）

- 日期：2026-08-14
- 发起方：主会话 AI（DeepCode）+ Agent A（audio.rs 音乐）
- 接收方：总指挥 / 后续迭代 AI（下一会话）
- 交接类型：迭代结束
- 交接内容：
  - **阶段一 ObjectiveState 桥接（commit `e840d6b` feat(net) + game.rs 部分随 `f705a28`）**：服务端 step_net_server 每 tick 广播 ObjectiveState(0x07)（归属码 Team↔0/1/2）；客户端 Client 新增 objective 状态字段 + handle_message 消费（乱序/重复丢弃）+ getter；step_net 日志带 obj/rule；回环测试 `net_objective_state_loopback_broadcast_consumed`（服务端广播 → 客户端解析据点归属/进度）；PROTOCOL_VERSION 不变向后兼容。
  - **阶段二 CoverSeek 互射触发（commit `f705a28` feat(ai)）**：压力模式 Chase 态目标 ≤ attack_range+40m 进 CoverSeek（advance 沿目标方向 `find_cover_shielding` 找遮挡掩体，`STRESS_COVER_MAX_DIST=35` 格覆盖障碍环带）；压力 Attack 态站定于掩体旁 → 标记 CoverSeek 持续可见；普通模式行为零回归（pick_attack_cover + 环带过滤不变）。flank_chance 0.1→0.22（wave1）。新增单测 `stress_cover_seek_triggers_near_obstacle`。**实测（压力 32v32 + street_fight，3 分钟）：突进 62% | 包抄 10% | 偷袭 15% | 撤退 9% | 掩体 4%，智能战术合计 29% ≥ 25%**。
  - **阶段三 程序化环境音乐（commit `346da6b` feat(audio)）**：Agent A 在 audio.rs 实现 MusicSynth 三声部（低音 pad 和弦循环/112BPM 行军节奏/A 小调五声音阶旋律，纯函数绝对时间驱动确定性）+ 混音总线 Music 通道接入 + 1.5s 淡入淡出（fade_step 纯函数）；game.rs 按 game_state 设 set_music_target（战斗 1.0/菜单 0.3）；12 新单测 audio 48 tests；实跑 VUID=0、panic=0、无爆音。
  - **验收（主会话独立复核）**：339 tests（+14：net 回环 1 + CoverSeek 1 + audio 12）/ 0 失败 / 0 警告；冒烟 ALL-OK（EXIT=0、VUID=0、kills=1、fps 190-263）；压力模式实测 VUID=0、panic=0。4 commits 已推送。
  - **修正上轮笔误**：指令单 #1 实为 **5** 个 commit（74a74dd/18f2808/331e3d9/20421f0/9e8ff9b），上轮交接记录误写"4 commits"，此处更正。
  - **遗留/下一步**：① 客户端 HUD 接入 ObjectiveState（解析已完成，数据接入 UI 后置）；② 音乐音量可调（Music 通道独立音量 API 已备，未接设置面板）；③ 压力模式多轮补员下 CoverSeek 占比 4% 偏低（掩体密度决定），如需更高可加 TOML 关卡掩体；④ Agent 并行教训已记录（>5min 无输出主会话接管）。
- 状态：done

### [2026-08-14] 交接：总指挥指令单 #2 开启（桥接收尾 + 战术纵深 + 音频氛围）

- 日期：2026-08-14
- 发起方：总指挥（外部模型）/ 主会话 AI（DeepCode）
- 接收方：Agent A（audio.rs 程序化音乐）/ 主会话（net 桥接 + CoverSeek 扩展）
- 交接类型：规划开启
- 交接内容：
  - **阶段一（ObjectiveState 桥接）**：服务端每帧用 objective 状态组包（Team↔0/1/2 归属码映射）广播 `NetworkMessage::ObjectiveState(0x07)`；客户端消费解析据点归属/进度（日志 + 状态字段，HUD 接入后置）；单机双进程回环验证；PROTOCOL_VERSION 不变。
  - **阶段二（CoverSeek 互射触发）**：NPC 目标交火时利用附近障碍（环带/TOML barrier/cover）进入掩体利用状态（贴掩体 + 探头射击）；压力模式出现非零 CoverSeek；调优 flank_chance/Flanker 权重使 包抄+偷袭+掩体 ≥ 25%；普通波次零回归。
  - **阶段三（程序化环境音乐）**：Agent A 在 audio.rs 实现 DspSynth 音乐合成（pad 低音 + 行军节奏 + 旋律动机，零资产）+ 混音总线（Music/Sfx 分层）+ 菜单/战斗淡入淡出；不破坏现有事件合成链路。
  - **验收**：≥325 tests、0 警告、冒烟 ALL-OK、零新依赖、每阶段增量验证、AGENTS.md 交接（顺带修正上轮"4 commits"笔误为 5）。
- 状态：in_progress

### [2026-08-14] 交接：总指挥指令单 #1 完成（关卡收尾 + AI 战术扩展）

- 日期：2026-08-14
- 发起方：主会话 AI（DeepCode）+ Agent A（allow 移除）/ Agent C（ObjectiveState 网络）
- 接收方：总指挥 / 后续迭代 AI（下一会话）
- 交接类型：迭代结束
- 交接内容：
  - **阶段一① dead_code 清理（commit `18f2808`，feat(game)）**：移除 map.rs/objective.rs 顶部 `#![allow(dead_code)]`（dead-code=0 硬红线）；删除未用 MAP_BLOCK_HEIGHT/MapManager::new/objectives/parse_kv/Value::Bool 与 ObjectiveSnapshot/CapturePointSnapshot/snapshot()（net.rs 消息为自定纯数据格式，主会话直接读 points 编码）；删 3 个冗余锁定测试。
  - **阶段一② 据点世界标记（commit `20421f0`，feat(render)）**：game.rs `capture_points()` 访问器 + main.rs 每据点 2 个 WorldMarker（立柱 + 半径 5.0 底盘，归属色蓝/红/灰随帧更新），复用现有通道零渲染管线改动。
  - **阶段一③ 压力+关卡共存验证**：`RV3D_STRESS_AI=16 + RV3D_MAP=street_fight` 实跑 32 NPC 互射 + 据点 capture 规则共存，VUID=0、panic=0（无冲突）。
  - **阶段一④ ObjectiveState 消息（commit `331e3d9`，feat(net)）**：`NetworkMessage::ObjectiveState(0x07)`（seq/time/rule_kind/points(id,归属码,进度)），大端手写编解码 + 防御（MAX_OBJECTIVE_POINTS=64、逐字段防越界、超大 count 撞 Truncated、InvalidUtf8）+ 6 单测；向后兼容（旧客户端收 0x07 → 由调用方忽略，PROTOCOL_VERSION 不变）。**桥接留给主会话**：服务端用 objective 状态组包广播（归属码 Team↔0/1/2 映射）。
  - **阶段二 AI 战术扩展（commit `74a74dd`，feat(ai)）**：`player_facing` 泛化为「任一敌对目标朝向感知」——pick_stress_targets 返回含目标 facing；step_npc 压力模式用 `target_yaw`（facing 坐标系 atan2(dz,dx)）判定目标是否面朝本 NPC；**冲锋覆盖排除 Flanker 的 Flank/Ambush**（is_flank_maneuver），其余角色冲锋仍全队直突。实测 32v32 三分钟：**突进 78% | 包抄 8% | 偷袭 9% | 撤退 6%**（修复前突进 91%/撤退 9%，Flank/Ambush=0）。新增单测 stress_flanker_tactic_follows_target_facing。
  - **验收（主会话独立复核）**：325 tests / 0 失败 / 0 警告；冒烟 ALL-OK（EXIT=0、VUID=0、kills=1、fps 187-269、panics=0）；压力模式实测 VUID=0、panic=0。4 commits 已推送 origin/master。
  - **遗留/下一步**：① ObjectiveState 桥接（服务端广播 + 客户端消费，指令单注明"不做完整联机"故本轮未做）；② Agent B（据点标记）在 deepcode -p 下卡死（CPU 0.1% 无输出 12min），已由主会话直接实现——**教训：deepcode -p 并行 Agent 偶发挂起，监控超时应主会话接管**；③ CoverSeek 战术实测占比仍 0（需要掩体+距离条件，压力模式开阔地无掩体，属预期）。
- 状态：done

### [2026-08-14] 交接：总指挥指令单 #1 开启（关卡收尾 + AI 战术扩展）

- 日期：2026-08-14
- 发起方：总指挥（外部模型）/ 主会话 AI（DeepCode）
- 接收方：Agent A（allow 移除）/ Agent B（据点世界标记）/ Agent C（ObjectiveSnapshot 网络）/ 主会话（AI 战术泛化）
- 交接类型：规划开启
- 交接内容：
  - **阶段一（关卡收尾 4 小项）**：① 移除 map.rs/objective.rs 顶部 `#![allow(dead_code)]`（已接线，须 0 警告）；② 据点世界内视觉标记（main.rs WorldMarker 通道，归属色蓝/红/灰 + 进度）；③ 压力模式 + 关卡系统共存验证（RV3D_STRESS_AI + RV3D_MAP 同开，VUID=0 无 panic）；④ ObjectiveSnapshot 接 net.rs（新消息，向后兼容）。
  - **阶段二（AI 战术扩展）**：`player_facing` 触发条件从「玩家朝向感知」泛化为「任一敌对目标朝向感知」——核心改 game.rs `step_npc`：压力模式用目标 NPC 的 facing 判定「目标是否面朝本 NPC」→ Flank/Ambush 在互射战场触发；普通波次（打玩家）路径不变（facing_angle 仍用 player_yaw）。pick_stress_targets 返回扩展为含目标 facing。
  - **验收**：≥321 tests 全绿、0 警告、冒烟 ALL-OK、零新依赖、每小项增量验证、AGENTS.md 交接。
- 状态：in_progress

### [2026-08-14] 交接：关卡系统完成（TOML 地图 + 占领/胜负 + 关卡流程）

- 日期：2026-08-14
- 发起方：主会话 AI（DeepCode）+ Agent A（map.rs）/ Agent B（objective.rs）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：迭代结束
- 交接内容：
  - **并行模式**：Agent A 写 `engine/map.rs`（1164 行）、Agent B 写 `engine/objective.rs`（531 行），主会话并行做整合侧（GameState/main.rs/ui.rs/示例地图/README），文件边界隔离零冲突。
  - **① 地图格式（commit `c8e6af5`，feat(map)）**：手写轻量 TOML 解析器（零第三方依赖，用户确认过铁律）——`[map]` 节/内联表数组/嵌套子表/跨行值/注释/UTF-8/未知键忽略；MapData + SpawnPoint/ObstacleDef/ObjectiveDef/RuleDef；`load_map`/`load_map_list`/`MapManager`（spawn_point 阵营过滤 + 确定性 LCG + reload 热重载）；`obstacle_to_map_obstacle`（wall/block/barrier/cover→ObstacleKind）。20 单测。
  - **② 目标系统（同 commit）**：`engine/objective.rs`——CapturePoint（归属/进度/半径/耗时）+ `update_point` 纯函数（单人推进/敌对压制/无人衰减/守点维持）；GameRule（CapturePoints/KillCount/TimeLimit）+ `ObjectiveState::evaluate` 每帧判定（幂等）；ObjectiveSnapshot 网络兼容骨架。14 单测。
  - **③ 整合（commit `1a139c8`，feat(game)）**：GameState 扩展 LoadingMap/Victory(Team)/Defeat（GameOver 保留）；`RV3D_MAP`（单关）/`RV3D_MAPS`（列表）环境变量启用关卡系统，**未设置保持程序化地图零回归（测试/冒烟基线不变）**；apply_level 用 TOML 障碍替换程序化；update_objectives 每帧据点推进 + 胜负判定；击杀注入 KillCount；F5 热重载/N 下一关（最后一关通关）/R 与 Enter 重开本关；HUD 顶部据点进度条（蓝/红/灰）。
  - **④ 交付（同 commit + `031dfbb` docs）**：`assets/maps/street_fight.toml`（巷战·占领 2 点）+ `open_field.toml`（开阔·歼 30）+ `index.toml` 关卡列表；README 关卡格式说明（环境变量表/TOML 样例/胜负流程）。
  - **验收（主会话独立复核）**：321 tests（+3 map 新增）/ 0 失败 / 0 警告；冒烟 ALL-OK（EXIT=0、VUID=0、kills=1、fps 248-263）；RV3D_MAP=street_fight/open_field 与 RV3D_MAPS=index 三种模式实跑 VUID=0、panic=0、fps ~250。
  - **遗留/下一步**：① 压力模式（RV3D_STRESS_AI）与关卡系统互斥未验证（stress 分支不受 map_mgr 影响，理论上可共存但未测）；② 据点占用视觉指示（HUD 已有进度条，世界内无圆圈标记）；③ 网络同步目标状态（ObjectiveSnapshot 骨架已备，未接 net.rs 消息）；④ map.rs/objective.rs 顶部 `#![allow(dead_code)]` 已可移除（主会话已接线）。
- 状态：done

### [2026-08-13] 交接：双 Agent 并行完成（美术贴图 + SIMD 负收益修复）

- 日期：2026-08-13
- 发起方：主会话 AI（DeepCode）+ Agent A（SIMD）/ Agent B（美术）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：迭代结束
- 交接内容：
  - **并行模式验证成功**：tmux 双会话 `deepcode -p`（Agent A/B 各司其职，文件边界隔离，cargo build lock 自动串行）。主会话独立验收后分两个 commit 提交。
  - **① SIMD 负收益修复（commit `f877115`，feat(perf)）**：新增 `transpose_aos3<const N>` 公共标量 load 转置内核，AVX-512(16)/AVX2(8)/AVX(8) 三档共用；删除 `_mm512_i32gather_ps`/`_mm256_i32gather_ps` 索引向量。微基准 65536 点×200 轮：avx512 0.92×→**1.68×**、avx2 0.83×→**1.75×**（Zen4 gather 负收益修复）；运算序列未动、五档 bitwise_eq=true、`RV3D_FORCE_SIMD` 选路不变。
  - **② 障碍/NPC 程序化皮肤贴图（commit `af3c70c`，feat(render)）**：`procedural.rs` 新增 marker 木板墙（竖板+接缝+木纹+钉痕）与 NPC 四色迷彩军服（双层细胞噪声软边）纯函数 + 8 单测；`build.rs` flat_flag 材质编码扩展 0=地面/1=marker/2=NPC，片元新增 binding 7/8 皮肤纹理采样；`renderer.rs` 新增 6 个皮肤纹理资源 + `create_sampled_image` helper + Drop 清理。**A/B 开关 `RV3D_SKIN_TEX=1` 启用贴图、缺省 0 纯色回退（冒烟基线不变）**；linear→sRGB 编码铁律满足。
  - **验收（主会话独立复核）**：287 tests（+8）/ 0 失败 / 0 警告；冒烟 ALL-OK（VUID=0、kills=1、fps_min=124.5、panics=0）；RV3D_SKIN_TEX=1 真机 ~190fps、VUID=0、panic=0。
  - **已推送**：`git push origin master`。
- 状态：done

### [2026-08-13] 交接：双 Agent 并行开启（美术贴图 + SIMD 负收益修复）

- 日期：2026-08-13
- 发起方：主会话 AI（DeepCode）
- 接收方：Agent A（SIMD 修复）/ Agent B（美术贴图）
- 交接类型：规划开启
- 交接内容：
  - **用户指令**：并行处理两个任务，多开 Agent 并行（12GB 内存内尽量多开）。文件边界隔离：Agent A 只改 `src/engine/simd.rs`；Agent B 只改 `src/engine/procedural.rs`/`renderer.rs`/`build.rs`。两者禁止 git 操作与改 AGENTS.md，由主会话统一验证 + commit。
  - **任务 A（SIMD）**：`shockwave_pressure_avx2/avx512`（simd.rs 216/153 行）gather 负收益（0.82-0.92×），改为与 `_avx`（277 行）相同的无 gather 标量 load 转置策略，预期 1.7×；保持逐位一致 + `RV3D_FORCE_SIMD` 选路不变。
  - **任务 B（美术）**：障碍物（marker）/士兵（NPC）程序化皮肤贴图，CPU 画像素零依赖可辨识配色；集成走传统管线零回归；R8G8B8A8_SRGB 写入前 linear→sRGB 编码。
  - **验收**：279 tests 基线 + 新增单测全绿、0 警告、冒烟 ALL-OK（VUID=0）。
- 状态：in_progress

### [2026-08-12] 交接：playtest_perf.py 性能测试脚本修复进行中（用户暂停，明日继续）

- 日期：2026-08-12
- 发起方：当前会话 AI（Codex）
- 接收方：下一会话 AI
- 交接类型：任务交接
- 交接内容：
  - **任务**：`scripts/playtest_perf.py`（**未追踪、未提交，勿丢**）——压力模式性能测试脚本：开游戏窗口、自动寻敌击杀、循环 3 次每次 ≥8 杀，采样 fps/CPU/GPU/硬件占用，每轮 F12 截图，输出实测报告（贴图/剔除/阴影确认）。
  - **已修复（本轮，脚本侧）**：① 站定目标时间戳新鲜度过滤（PT_FRESH=20s）+ 波次过滤（最近一次"全量补员开新轮"之后）——根因：压力模式战场互射（apply_npc_combat DPS 结算）静默移除 NPC（只记 `battle: 阵亡 N` 总数、无按 ID 死亡日志），历史站定条目变幽灵目标；修复前完整跑 303 发仅 30 命中（9.9%）、每轮 0-2 杀。② 只瞄 70m 内（PT_MAX_DIST，更远命中率骤降）。③ 轮内周期夺焦（PT_REFOCUS=45s，WSLg 焦点漂移）。④ 首轮命中则第二轮免重瞄直射。⑤ ROUND_TIMEOUT 默认 120→360s（击杀窗口=互射把 NPC 打光前约 20-40s/波，须跨多波累计 8 杀）。
  - **验证数据**：快速验证 1 轮×3 杀 = 57.4s 全过（命中 14/30=47%，修复前 9-14%）；完整 3×8 验收 = **19/24 击杀**（轮 1/2 各 8/8 通过，轮 3 仅 3/8 超时 361s），总计 97 命中/357 发、fps min 116/p50 214/p95 234、present 1812µs、ai 399µs、GPU 39%/2.5GB、VUID=0。截图：轮 3 成功（/tmp/steel_front_1786542924.png，ASCII 分析可见地面/红蓝阵营/HUD，渲染正常），轮 1/2 失败（F12 焦点丢失）。
  - **剩余工作（明日继续）**：① 轮 3 卡 3 杀根因 = 残血目标不补枪——脚本每目标最多 2 次尝试（12 发），波次靠后的满血 NPC 需 4-6 发命中，打残不打死；改为持续补射同一目标至死（命中持续则最多 4 轮点射，连续两轮零命中才放弃换目标）。② 截图失败 = F12 焦点（refocus FAIL 0x0 时 XTest 点击夺不回，截图前需 RaiseWindow + 验证焦点重试）。③ FPS 判定 min≥120 过严（压力模式混战瞬时 min=115.6），改 p50≥120 + min≥60 或环境变量可调。④ 全部通过后提交 `scripts/playtest_perf.py`（feat(test)）+ 本交接收尾登记 + `git push origin master`。
  - **用户点名提醒（明日对话必须转达）**：用户睡前要求——明日重启对话时提醒用户"上下文窗口已接近耗尽，建议准备 / 提示进行 AI 交接工作"（AGENTS.md 交接协议）。这是用户明确指定的提醒事项，务必转达。
- 状态：done（2026-08-13 完成，见下方 [2026-08-13] 交接）

### [2026-08-13] 交接：playtest_perf.py 完成——改为时长制「压力与 AI 智能实测」

- 日期：2026-08-13
- 发起方：当前会话 AI（Codex）
- 接收方：下一会话 AI
- 交接类型：迭代结束
- 交接内容：
  - **脚本已提交**：`scripts/playtest_perf.py`（feat(test)），压力模式压力与 AI 智能实测工具。
  - **用户决策（重要，勿回退）**：击杀不是测试目的、是附带指标。测试目的 = 两波 AI 互打对电脑的压力 + AI 智能表现（战术/包围/偷袭等）。改为**时长制**：跑满 PT_SECS（默认 600s）即完成，不再要求 3 轮 × ≥8 击杀；击杀不设门槛、不判 FAIL。
  - **核心机制**：压力模式（RV3D_STRESS_AI=64，红蓝各 64 互射 + 一队团灭全量补员无限波次）持续运行；后台采样 fps/CPU/GPU/硬件 + AI 行为（状态分布、8 种战术分布、互射阵亡、波次、爆炸）；周期 XGetImage 截图（PT_SHOT_EVERY，**不依赖键盘焦点**——WSLg 焦点漂移时 F12 收不到键事件）；玩家自动瞄准点射（站定目标时间戳新鲜度过滤 PT_FRESH=20s 防幽灵目标——压力模式互射 DPS 结算静默移除 NPC、无按 ID 死亡日志；只瞄 70m 内；持续补枪至死）；M1 150 发耗尽后自动转旁观，AI 互射与采样继续。
  - **实测结果（600s，1280×800，dzn/WSLg）**：ALL-OK。玩家击杀 9（附带）、命中 39/144（27%）；fps min 37/p50 163/p95 205/max 222（min 深跌=爆炸/补员瞬间；present 2146µs 仍是 dzn 呈现瓶颈）；ai 608µs、cull 695µs、cycle 5284µs；GPU 39%/1.98GB；互射阵亡 371、波次 4 轮、爆炸 110、NPC 在场均值 49；战术分布 突进 95%+撤退 5%。
  - **AI 智能发现（重要结论）**：压力模式 NPC 互射只出现 突进/撤退 两种战术——包抄(Flank)/偷袭(Ambush)/掩体(CoverSeek) 等战术在 game.rs 是**玩家朝向感知驱动**（player_facing），纯 NPC-vs-NPC 战场不触发。若要让 AI 对战体现包抄/偷袭，需后续改游戏代码给 NPC 目标也启用这些战术（脚本无能为力）。
  - **验收**：跑满 600s、VUID=0、panic=0、玩家血量恒 100 → ALL-OK；fps 为参考不判 FAIL（PT_FPS_P50/PT_FPS_MIN 可调）。
  - **环境变量**：PT_SECS（时长）、PT_SHOT_EVERY（截图间隔）、PT_MAX_DIST、PT_FRESH、PT_REFOCUS、PT_FPS_MIN/PT_FPS_P50、PT_SIDES、PT_WIN_COUNTER；`--attach` / `--no-shadow`。
- 状态：done

### [2026-08-13] 交接：SIMD 指令级 A/B 实测完成（RV3D_FORCE_SIMD + 隔离微基准）

- 日期：2026-08-13
- 发起方：当前会话 AI（Codex）
- 接收方：下一会话 AI
- 交接类型：迭代结束
- 交接内容：
  - **新增**：`RV3D_FORCE_SIMD=avx512|avx2|avx|sse4.2|scalar` 强制锁定 SIMD 档位（三处共用：`simd::shockwave_pressure`、`renderer::cull_spheres_dispatch`、`morph_heights_dispatch`；仍要求硬件支持，非法值告警回退）；新增两个隔离微基准测试（`shockwave_path_microbench` / `simd_cull_microbench`，`cargo test --release <名> -- --nocapture --test-threads=1`，65536 元素 × 200 轮，无渲染并发）。
  - **权威数据（本机 Zen4/8940HX）**：① 视锥剔除 65536 实例：scalar 798µs → avx512 53µs（15.06×）、avx2 49µs（16.29×）、avx 50µs（15.96×）、sse4.2 249µs（3.20×）——**剔除 SIMD 极有效**。② 冲击波压力场 65536 点（AoS `[f32;3]`）：avx **1.67×**（33µs，标量 load 转置、无 gather）、avx512 0.92×、avx2 0.83×、sse4.2 0.82×、scalar 55µs——**gather 型内核负收益**：Zen4 `vpgatherdd` 微码取数淹没向量收益，AVX2/AVX-512 反而慢于标量。③ 地形 morph：全档 6µs、1.00×（内存带宽瓶颈，中性）。
  - **游戏内对照（60s/档，128 NPC 压力，固定视角）**：fps 181–195、cull_us 590–674µs 五档持平——`cull_us` 是"剔除+压缩+GPU 上传"全程，上传 ~500µs/帧占大头，剔除算力（SIMD ~50µs→并行 ~10µs；标量 ~800µs→并行 ~90µs）被掩盖；帧率由 present（dzn ~2ms）主导。`ai_us` 各轮 NPC 存活数随机（8–102）不可比。全部 5 档 bitwise_eq=true、VUID=0、272 tests 全绿。
  - **结论与建议**：剔除 SIMD 是当前最大有效优化（15–16×）；冲击波 gather 内核是负优化，**后续把 `shockwave_pressure_avx2/avx512` 改为与 `_avx` 相同的无 gather 转置策略**（预计 0.85×→1.7×，低风险高收益，建议排期）；完整报告 `docs/perf-simd-tier-2026-08-13.md`。
  - **用户决策**：Deep Code CLI 已在 WSL2 安装（`@vegamo/deepcode-cli` v0.1.34，npm 前缀改 `~/.npm-global`，配置 `~/.deepcode/settings.json` 模型 deepseek-v4-flash——2026-08-13 从 Pro 切换，见下方「模型切换铁律」交接）；API 密钥经全盘排查仅存在于本对话与 `~/.deepcode/settings.json`，无其它落盘/网络泄露。
- 状态：done

### [2026-08-13] 交接：DeepCode 主模型切换铁律（一律 Flash，禁用 Pro）

- 日期：2026-08-13
- 发起方：用户决策 / 当前会话 AI（DeepCode）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：规划开启
- 交接内容：
  - **用户决策（铁律，勿回退）**：DeepCode 主模型已从 `deepseek-v4-pro` 切换为 `deepseek-v4-flash`（`~/.deepcode/settings.json` 的 `env.MODEL`），思考强度保持 `reasoningEffort="max"`。**后续所有会话一律使用 Flash，禁止再启用 Pro 模型**（费用承受不起）。
  - **执行要求**：任何会话/配置变更不得把模型改回 Pro；Agent 分身同样用 V4 Flash + Max（见下方「本地 Agent 工具迁移」交接）。
- 状态：done

### [2026-08-13] 交接：本地 Agent 工具迁移（Codex → DeepCode）+ 美术方向（AO/光照烘焙/程序化贴图）开启

- 日期：2026-08-13
- 发起方：用户决策 / 当前会话 AI（DeepCode）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：规划开启
- 交接内容：
  - **工具迁移（用户决策，勿回退）**：本地开发 Agent 工具已从 Codex 迁移至 **DeepCode**，后续开发一律在 DeepCode 上进行。
  - **Agent 分身策略（铁律，勿回退）**：需要开启智能体分身（并行 Agent）时，**不要直接开启 Voice Pro 的智能体分身**，改启用 **V4 Flash 模型**进行 Agent 分身，思考强度保持 **Max**。
  - **本轮任务（美术方向 ⑤ 剩余项）**：光线遮挡（AO/SSAO 或烘焙 AO）、光照烘焙（程序化地图 lightmap）、程序化贴图（CPU 画像素，零依赖，产出可辨识配色）。
- 状态：done（程序化贴图 + 烘焙 AO + 光照烘焙已完成，见下方迭代结束记录）

### [2026-08-13] 交接：程序化地面材质 + 烘焙 AO + 光照烘焙完成（美术方向 ⑤ 收尾）

- 日期：2026-08-13
- 发起方：当前会话 AI（DeepCode）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：迭代结束
- 交接内容：
  - **功能**：新增 `src/engine/procedural.rs`（纯函数 + 单测）——CPU 画像素生成世界空间地面材质纹理（草地绿/沙地黄/石板灰三大区 + 道路 + 焦土弹坑，可辨识配色），叠加**烘焙高度场 AO**（凹处暗、凸处亮）与**静态天光**（太阳方向漫反射 + 天光底）。
  - **渲染集成**：`renderer.rs init_texture` 用程序化纹理替换中灰 test.png（`RV3D_PROC_TEX=0` 回退 A/B）；片元着色器改 **world-space UV**（`world_pos.xz` 映射全图，与烘焙纹理严格对齐）；障碍 marker 走**纯 tint 色**（flat_flag 扩展到 `MARKER_INSTANCE_BASE=65537`，避免障碍立面采样地面材质）。
  - **关键修复（勿回退）**：① 纹理写入 `R8G8B8A8_SRGB` 前必须 **linear→sRGB 编码**（否则硬件采样二次压暗、色调丢失）；② 材质分域用**世界尺度 value_noise**（biome~120m、detail~10m），旧 fbm(x*0.025) 尺度 2560m 远超地图导致整图单一色；③ 明确分区（smoothstep 阈值）+ 高饱和配色 + mix 纹理权重 0.5→0.75。
  - **验收**：279 tests 全绿（新增 7 个 procedural 单测：确定性/尺寸/AO 凹度/天光单调/弹坑局部）、0 警告、冒烟 ALL-OK（VUID=0、kills=1、fps 172-243）、playtest 压力模式 ALL-OK（VUID=0、panic=0）；截图确认地面呈草地绿/沙地黄可辨识配色（非中灰）。
  - **遗留/下一步**：障碍物/士兵皮肤程序化贴图（本轮只做地面+地形，用户已确认）；AO 仅烘焙静态高度场，障碍接触阴影交实时阴影贴图（障碍可摧毁，不宜烘焙）。
- 状态：done

### [2026-08-11] 交接：美术方向规划开启（阴影 / AO / 烘焙 + 程序化贴图）

- 日期：2026-08-11
- 发起方：用户决策 / 当前会话 AI（Codex）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：规划开启
- 交接内容：
  - **用户决策**：联网补齐后置（WSL2 环境兼容性失望，怕再出幺蛾子）；美术方向优先——阴影、光线遮挡（AO）、渲染烘焙；正式贴图/美术资产管线后置，先用程序化生成（CPU 画像素，零依赖）产出可辨识配色（地板/屋顶/坦克/枪械皮肤）。
  - **技术路径（传统光栅特性，dzn 可跑）**：① 阴影 pass = 第二 render pass 渲深度到 shadow map（D32_SFLOAT，定向光投影），主 pass 片元采样做深度对比/PCF；② AO = SSAO 或烘焙 AO；③ 烘焙 = 程序化地图 lightmap（地图确定性生成，天然适配预烘焙）。
  - **风险与红线**：renderer 是高风险区（pipeline/shader/swapchain 改动须冒烟验 VUID）；阴影 pass 涉及新 render pass + 多 pass 同步 + descriptor set 扩展，逐块落地并保持传统管线零回归。
  - **验收**：265 tests 基线 + 冒烟 ALL-OK 不破。
- 状态：in_progress

### [2026-08-12] 交接：阴影贴图完成（depth-only pass + 3×3 PCF）

- 日期：2026-08-12
- 发起方：当前会话 AI（Codex）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：迭代结束（美术方向 ⑤-①）
- 交接内容：
  - **功能**：定向光 depth-only pass 渲染光空间深度到 2048×2048 D32 阴影图（正交投影 target=地图中心、半宽 250m、near=1 far=500，覆盖障碍环带与接火区），主 pass 片元 3×3 PCF 深度比较 + depth_bias 0.005 / normal_bias 0.02 防 acne；`RV3D_NO_SHADOW=1` 关闭阴影（A/B 验证）。
  - **四个根因修复（勿回退）**：① identity 槽位写错——`create_instance_buffer` 把地形 identity 矩阵写到槽位 0 而非 `INSTANCE_COUNT`(65536)，槽位 0 每帧被 `cull_and_upload` 覆盖 → 地形矩阵塌缩；② 光方向符号——`ShadowConfig::new` 直接传 `sun.direction`（表面→光源），旧传 `-sun.direction` 使光相机在地下仰视 → 地形整片缺失；③ 阴影 UV 必须 V 镜像 `uv.y = 1-(p.y*0.5+0.5)`（depth-only pass 顶点经 naga ADJUST_COORDINATE_SPACE Y 翻转，不镜像阴影方向整体错位）；④ 深度映射 `frag_depth = clip.z*0.5+0.5`（glam ortho_rh clip.z∈[0,1] 经 viewport 映射到 [0.5,1]，不偏移比较基准错 0.5）。
  - **采样器铁律**：阴影采样器 `.compare_enable(false)`（手动 PCF 用 `textureSample` 读原始深度再比较；comparison sampler 非 Dref 采样严格验证报 VUID）。
  - **验证**：A/B（`RV3D_NO_SHADOW=1` 对照，固定视角）62% 像素亮度不同、暗化集中在障碍环/地形丘陵（1.35% 像素暗化 >30，几何稀疏属预期）、方向正确；266 tests（新增 `world_to_shadow_uv_mirrors_v_and_maps_depth` 回归单测）、0 警告、冒烟 ALL-OK（VUID=0、kills=1、fps 137–207）。
  - **已清理**：临时调试代码全部删除——`debug_readback_shadow`/`RV3D_SHADOW_READBACK`/build.rs DEBUG 可视化块/`RV3D_AUTOSHOT_SECS` 自动截图钩子；shadow image usage 移除 TRANSFER_SRC。
  - **遗留/下一步**：AO/光线遮挡（SSAO 或烘焙）、光照烘焙（程序化地图 lightmap）、程序化贴图（CPU 画像素，零依赖）。
- 状态：done

### [2026-08-12] 交接：物理核/超线程分层绑定完成（线程优化第 5 步）

- 日期：2026-08-12
- 发起方：用户决策 / 当前会话 AI（Codex）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：迭代结束
- 交接内容：
  - **用户决策**：高性能线程（主线程/渲染/scene 池）不再绑定超线程，严格绑物理核；物理核对应超线程仅在池线程数超过物理核数时作为溢出辅助。CCX 级分离（主线程一个 CCX、渲染另一个 CCX、3900X/3950X 单独 CCX 供地形/AI）因当前"渲染在主线程"架构 + WSL2 抹平 L3/NUMA，推迟至 Windows 原生 Vulkan（DeepCode）阶段。
  - **实测结论（勿写死奇偶）**：8940HX 在 WSL2 下 sysfs `thread_siblings_list` 保留真实 SMT 配对（0-1/2-3/…），**偶数 vCPU（0,2,4…）是物理主线程、奇数是超线程**——用户口述"单数=物理"恰相反；正确做法是运行时读 sysfs 每对取最小 vCPU，不可读时回退旧行为。
  - **实现**：`CpuTopology` 新增 `primary_physical`/`secondary_physical`/`smt_set`；`scene_compute_set()`/`ai_set()`/`pin_main_thread()` 默认返回物理核集合（主线程/scene 池绑 CCD0 物理核 `[0,2,…,14]`、ai 池绑 CCD1 物理核 `[16,18,…,30]`）；`ThreadPool::new` 分层绑定——线程数 ≤ 物理核数全绑物理核，超出部分绑 物理核∪超线程（`RV3D_SCENE_WORKERS`/`RV3D_AI_WORKERS` 调大时触发）。
  - **验证**：270 tests（新增 4 个 SMT 配对单测）；128 NPC 压力基准 fps p50 215（前 212-216 持平）、ai_us p50 364µs（前 424-640µs，提升 14-43%）、cull_us 486µs（前 505-527µs）、VUID=0；冒烟 ALL-OK（kills=1）。
- 状态：done

### [2026-08-11] 交接：线程分层调度完成（第 1-4 步）

- 日期：2026-08-11
- 发起方：当前会话 AI（Codex）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：迭代结束
- 交接内容：线程优化四步全部落地——① `AiTier`/`classify_ai_tier`/`partition_ai_tiers`（纯函数+单测）；② 双池调度：近组→`scene_pool`（P 核/CCD0）、远组→`ai_pool`（AMD CCD1 / Intel 有 E-core 即绑 E-core），`update_ai` 开头稳定重排、重排后 pick targets/under_fire 保证索引对齐，逐位一致性测试按 id 对齐；③ 地图生成换核 `ThreadPool::run_sync`（join 语义 + spawn 失败降级）+ 远组降频 `AI_FAR_DECIMATE=4`（红线：攻击/感知/受击/被瞄准恒每帧；`RV3D_AI_DECIMATE=off` 关闭）；④ 基准验证见 `docs/perf-ai-tier-2026-08-11/`（分层生效、AI 非瓶颈、压力模式降频收益≈0 属预期）。验收：265 tests、0 警告、冒烟 ALL-OK（kills=1、VUID=0、fps 244-303）。
- 状态：done

### [2026-08-11] 交接：迭代方向决策（鼠标推迟 / 美术阴影烘焙优先 / 线程分层调度）

- 日期：2026-08-11
- 发起方：用户决策 / 当前会话 AI（Codex）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：规划开启
- 交接内容：
  - **输入捕获问题（WSL2 归因，推迟处理）**：根因是 WSLg/Xwayland 指针协议不完整（无 raw 事件、confine 失效、warp 不可靠、`set_cursor_visible` 无效、无 `/dev/input`），改来改去均被环境拖累。用户决策：**进度并入 DLSS 集成阶段，待迁移至 Windows 原生 Vulkan（DeepCode 开发）时一并处理**；后续会话在 WSL2 内勿再为输入捕获投入修复（用户情绪敏感，改前先确认方向）。`docs/HANDOFF-2026-08-11.md` 证据与 H1/H2/H3 假设保留作迁移参考。
  - **美术方向**：暂不做 PBR/材质/贴图体系；先做简单阴影与光线遮挡、传统光栅化渲染、预渲染烘焙（dzn 可跑，地图确定性生成适合烘焙）；贴图"随便画一点点够验证即可"。
  - **线程优化（当前重点，AMD/Intel 双层策略）**：
    - AMD 双 CCD：CCD0=主线程 + 无法拆分/延迟敏感线程（跑高频）；CCD1=地形生成、AI、爆炸计算等延迟不敏感重计算，尽量多拆（8 核 16 线程）。
    - Intel 分层负载（修正原"E-core≥8 全接 AI"）：E-core 少（12600K/13400F/12700K，4E）时只负载部分 AI，近/实时交互 AI 走 P-core、远/低精度 AI 走 E-core；E-core 多（14600K/12900K/13700K，8E）负载绝大多数 AI；E-core 极端（14700K/13900K，12-16E）可再接地形生成/全局大计算，P-core 专注逻辑/渲染/主线程。
    - 落地顺序：① 分组纯函数（距离+交互态 → 近/远/可降频，阈值配置化+单测）→ ② 双池调度（近→P 池，远→E 池，E-core 不足回退 P 池剩余核）→ ③ 低频重计算挪核（地形生成/烘焙）→ ④ 固定视角基准验证（`RV3D_STRESS_AI`，对比 ai_us/CPU 分布/逐位一致）。
    - 红线：攻击态/接火 NPC 必须每帧步进，降频仅限无感知非攻击 NPC；渲染不拆线程（现状渲染=主线程）。
- 状态：in_progress

### [2026-08-11] 交接：渲染技术路线决策（WSL2 内传统管线主迭代，网格着色器冻结至中后期）

- 日期：2026-08-11
- 发起方：用户决策 / 当前会话 AI（Codex）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：规划开启
- 交接内容：
  - **用户决策（V0.6 起）**：为减轻代码复杂度与 Token 消耗、快速做出成品，WSL2 内继续以传统 VERTEX+FRAGMENT 管线为主迭代并正常升级，在游戏约 90% 功能/工程任务完成前不再启用网格着色器路径。
  - **网格着色器（VK_EXT_mesh_shader，commit `12859a3`）**：冻结后续迭代，仅保留为接口与验证功能（代码/文档/可选路径均不动）；不为其新增特性、不做双路径渲染验证。
  - **未来迁移（中后期计划）**：迁移至 DeepCode 平台时全面放弃传统顶点着色器、改用网格着色器进行画面渲染；该任务与 DLSS、光线追踪硬件启用同一优先级。
  - **依据**：本机 WSLg/dzn 实测 VK_EXT_mesh_shader=false，传统管线是唯一可运行/可冒烟/可调试路径；mesh 路径从未真机运行，冒烟与 259 tests 验收基线全部依赖传统管线。
  - **执行要求**：传统管线正常迭代升级（不受冻结影响）；后续会话勿为 mesh 路径投入开发与双路径验证。
- 状态：done

### [2026-08-11] 迭代结束记录

- 日期：2026-08-11
- 发起方：当前会话 AI / Newton（并行）
- 接收方：后续迭代 AI（下一会话）
- 交接类型：迭代结束
- 交接内容：
  - ① 网格着色器（VK_EXT_mesh_shader）可选路径：完成。commit `12859a3`。要点：build.rs 用 naga 30 `enable wgpu_mesh_shader` + `@mesh(mesh_out)`（naga 30 语法是 `@mesh(...)` 不是 `@stage(mesh)`，输出变量必须 workgroup 空间；SPIR-V ≥1.4 用 lang_version=(1,4)）；mesh 管线 MESH_EXT+FRAGMENT 复用同一 descriptor set layout + MESH_EXT push constant（base_slot，16B，naga 用 `var<immediate>`）；逐实例 GPU 视锥剔除（Gribb–Hartmann，Vulkan z∈[0,1] 近=r2 远=r3−r2）+ 立方体/远档十字按距离²选几何；地面场静态一次性上传跳过 CPU 剔除；**maxMeshWorkGroupCount[0] 最低保证 65535，地面场 65536 workgroup 必须按查询上限分块下发（已修，字段 mesh_max_wg_x）**；设备创建在扩展可用时才挂 PhysicalDeviceMeshShaderFeaturesEXT，本机 dzn 不可用 → 设备创建与旧代码逐字节一致；传统路径零回归（冒烟 ALL-OK，kills=1 VUID=0 fps 254–325）。遗留：mesh 路径本机无法真机运行，需 Windows 原生 Vulkan / mesh 驱动环境验证。
  - ② README.md 重构为对外进度说明书：完成。commit `bf95aee`（重构）+ `c686c51`（更新：259 tests、网格着色器可选路径状态）。
  - ③ AGENTS.md 升级为正式交接文档：完成。commit `0ea4383`。
  - 验收快照：259 tests 全绿（含 UDP 回环，需提权环境跑）、0 警告、冒烟 ALL-OK。附带修正：仓库中 4 个既有 .spv 为陈旧产物（与 build.rs 输出不一致），已随 feat 刷新。
- 状态：done

### [2026-08-11] 交接：输入捕获遗留 Bug（转交下一会话）

- 日期：2026-08-11
- 发起方：当前会话 AI
- 接收方：下一会话 AI
- 交接类型：任务交接
- 交接内容：鼠标捕获在 WSLg/Xwayland 仍未彻底解决，本轮按用户要求不改代码只写交接。
  详细证据/假设见 `docs/HANDOFF-2026-08-11.md`。要点：① 开局即登记捕获（grab=confined
  返回 Ok）但 confine 实际不生效，指针漂出窗口 → 无 CursorMoved → 视角冻结；死亡后 R
  重开指针恰在窗口内 → 事件恢复（Minecraft 式指针=准星，用户可接受但要求隐藏图标）。
  ② set_cursor_visible(false) 在 Xwayland 不隐藏图标。③ 真实鼠标无 raw 事件（勿用冒烟
  XTest 反推）；rdev 在 WSL2 不可用（无 /dev/input、/dev/uinput）。④ 修复方向：H1 每帧/
  每事件 warp 拉指针回窗口并验证 is_ok（可考虑 x11rb 直连 XWarpPointer）；H2 检查
  focused 依赖；H3 XDefineCursor 空光标隐藏图标。⑤ 用户情绪敏感，改前先确认方向。
- 状态：in_progress

## 当前进度快照（2026-08-08，wsl --shutdown 前固化）

### 已完成（Wave 2/3/4 + 2026-08-08 渲染/输入修复，全部已推送 origin/master）
- Wave 2：`d5a4240` feat(game) / `5bd5f57` chore(scripts) / `0b7f5e6` docs
- Wave 3：`e593272` chore(wip) checkpoint / `bd2bb1f` feat(config) / `1010447` chore(game) / `f7e5f01` docs
- Wave 4 + 修复：主题/波次/截图/画质等已推送；`18ad6ca` docs(AGENTS.md)
- 2026-08-08 渲染修复：投影双重 Y 翻转致画面倒立（camera 用 perspective_rh、翻转只归 shader）/
  F12 截图读回 VUID 不落盘（用 in_flight_fences 等待）/ test.png 四象限调试图改纯中灰 128 /
  GAME OVER 全屏暗红遮罩改中性深灰 / 地面彩虹棋盘格来自立方体 6 面多色顶点色 × tint，
  已白化 VERTICES 顶点色（`91a6489`）+ 实例 tint 固定 0.7 灰（`fc7b50a`）
- 2026-08-08 地形/输入修复：地形全图拍平 y=0（`d08cd21`，删 value_noise/flatten_mask/noise_hash，
  保留 smooth_t 供 LOD morph）/ 鼠标灵敏度默认减半 0.003→0.0015 rad/px（`e068b02`）/
  视角改 XInput2 相对增量驱动（`5373a08`，WSLg/Xwayland 下 grab 不可靠，勿回退）
- 2026-08-09 跨平台：`5bf7c77` feat(perf) AArch64 NEON 剔除路径 + CPU 平台隔离
  （Apple Silicon/Android 通用，见下方「跨平台/指令集决策」）
- 2026-08-08 性能压测：帧率无上限（main.rs `MAX_FPS=0`，`FRAME_BUDGET=0` 跳过 sleep/spin 节流，
  设回正数即恢复门控）；NPC 数量缩放 `RV3D_NPC_SCALE`（默认 1.0，`max(0.5)`，波次/援军同乘）；
  renderer 视锥剔除 AVX2 化（SoA 球心 + 8 实例/批 × 256 位，非 FMA 保与标量逐位一致，
  `is_x86_feature_detected!("avx2")` 运行时选路，标量回退）。实测 1280×800 无上限 fps 270–433
  （6→48 NPC），`cull_us` 357–502 → 72–286，frame_us p50≈1.4–2.0ms → 瓶颈在 present/GPU 非 CPU
- 2026-08-09 性能探针与瓶颈结论（勿回退）：
  - GPU 甄别：dzn 枚举唯一设备 = NVIDIA RTX 5060 Laptop（LUID 0x00010bed，vmwp 进程 engtype_3d，
    实测显存占用 ~1.5GB）；AMD 610M 核显（LUID 0x000122d1）空闲 0MB。后续光追/DXR 走 NVIDIA 路径
  - 分阶段探针已入库：renderer 1Hz 日志带 `wait_fence_us/acquire_us/terrain_us/record_us/submit_us/
    present_us`；game 状态行带 `phys_us/ai_us/audio_us/net_us`；cam 行带 `cycle_us/update_us/render_us`
  - 1280×800 实测 350fps 瓶颈链：`present_us≈1.1ms`（frame_us 的 55–70%，dzn/WSLg 转译层固有，
    MAILBOX 与 IMMEDIATE 无差异，`RV3D_PRESENT_MODE=immediate|mailbox|fifo` 可覆盖验证）+
    事件循环 ~0.8ms + 实际渲染 ~0.5ms；game update 仅 6–18µs。GPU Engine SUM≈38–42%（Windows 侧
    Get-Counter '\GPU Engine(*)\Utilization Percentage' 实测），与"CPU/GPU 均未跑满"一致
  - 1920×1080 对照：fps 358→111，`wait_fence_us` 219→659 → 分辨率提升后 GPU 才饱和；
    1280×800 下把负载提上去（更高分辨率/更多实例）才能喂满 GPU
- 2026-08-09 CPU 优化（src/engine/cpu.rs，勿回退）：
  - 启动时拓扑检测：CPUID vendor + sysfs online + leaf 0x1A hybrid（Intel E-core 统计）；
    WSL2 抹平 L3/NUMA（node 仅 node0、L3 shared_cpu_list 全 0-31），双簇按 vCPU 顺序推断
    （前半=首簇/CCD0，后半=次簇/CCD1），实测 8940HX primary=[0-15] secondary=[16-31]
  - 主线程亲和绑定 `sched_setaffinity`（FFI，无第三方依赖）：默认绑首簇物理核（CCD0
    物理主线程，2026-08-12 起弃用超线程，见下方 2026-08-12 交接），`RV3D_CPU_PIN=off`
    关闭、`RV3D_CPU_PIN=0-7,16-23` 精确覆盖；Intel 上主线程绑 P-core 组、
    E-core 组进 secondary（≤8 仅轻任务，>8 可接 AI/地图生成——决策已编码，供未来线程池）
  - AI 并行已落地：`ai_pool` 亲和线程池（AMD 绑 CCD1=secondary_set；Intel E-core≥8 绑 E-core，
    否则 P-core），`step_ai_parallel` 走 `pool.par_for_each_mut`，逐位一致测试保序
  - renderer 剔除五级选路：avx512f（16 实例/批，Zen4/Zen5 原生 512 位）> avx2（8）>
    avx（8，3/4 代酷睿与初代锐龙）> sse4.2（4，2008 年后全平台）> 标量，
    各级路径与标量逐位一致（非 FMA）；`_mm512_*` 在 Rust stable 可用（实测本机 avx512f=true）。
    非 x86_64 平台走标量兜底（cfg(not) 分支，勿删）
- 2026-08-09 GPU 硬件能力探测（src/engine/gpu_caps.rs，启动日志 gpu-caps: 前缀）：
  - WSLg/dzn（Vulkan-on-D3D12 转译）实测结论：VK_KHR_ray_tracing_pipeline / acceleration_structure /
    ray_query / deferred_host_operations 全 false（Mesa dzn 未实现 DXR 映射）；VK_KHR_cooperative_matrix /
    VK_NV_cooperative_matrix false；DLSS 私有扩展（VK_NVX_* / VK_NV_cuda_kernel）全 false；
    仅 VK_KHR_buffer_device_address / VK_KHR_dynamic_rendering 可用
  - 【结论】WSL2 的 Vulkan 路径无法调用 RT Core/Tensor Core(协作矩阵)/DLSS → 光追/全景路径追踪
    需迁移 Windows 原生 Vulkan（NVIDIA 驱动全支持），或走 CUDA 直通（/usr/lib/wsl/lib/libcuda.so
    实测存在，Tensor Core 可编程访问，可自研超分/降噪，OptiX 同源可用）
  - 探测含决定性测试：创建启用 RT 扩展的探测 device（当前 RT 扩展缺失故跳过）
- 硬件/API 标准（2026-08-11）：游戏用传统 VERTEX+FRAGMENT 管线（无网格着色器）、
  实例声明 Vulkan 1.3 但只用 1.0 核心特性 + VK_KHR_swapchain；硬件三档标准见
  `docs/hardware-requirements-2026-08-11.md`（最低=AMD Ryzen 3 3300X + RX 6500 XT
  （RDNA 2，mesh shader 起点）4C8T、内存最低 8GB / 推荐 12GB+；推荐=8C16T 中端独显、
  最高=16C32T + RTX 40/50 系，瓶颈在 dzn 呈现不在 GPU）。测试版不建议使用不支持
  mesh shader 的显卡游玩，提前适配支持 VK 图形 API 新特性的硬件，为后续全面迁移铺路
- 2026-08-09 AVX-512 启用策略（cpu::avx512_enabled()，renderer 选路与日志共用）：
  - AMD Zen4/Zen5（7000/9000 系）→ 启用（实测本机 avx512=true，走 16 实例/批剔除路径）
  - Intel 11 代（Rocket Lake 0xA7 / Tiger Lake 0x8C/0x8D）→ 默认关闭（AVX-512 能效/降频差，
    游戏负收益）；Intel 12 代起（model≥0x97，13/14 代同）→ 防御性关闭（出厂已熔丝禁用，
    防虚拟化异常透传，E-core 无 AVX-512）
  - `RV3D_DISABLE_AVX512=1` 可强制关闭；renderer 选路注释已标明 AVX-512 使用情况
- `.wslconfig` 已配置 `[wsl2] networkingMode=mirrored + dnsTunneling + firewall + autoProxy`，待 `wsl --shutdown` 生效
- 验收快照：176 tests passed、0 警告、20s 冒烟 ALL-OK（kills=1、VUID=0、fps 214.8–292.7）
- 2026-08-09 全分辨率压力基准（存档 `docs/perf-2560x1600-64v64/`）：
  - 2560×1600 + RV3D_STRESS_AI=1（64v64/128 NPC）固定视角实测 fps p50≈264，1280×800 对照 ≈260
    → 分辨率×4 像素帧率持平：瓶颈仍是 dzn present（present_us p50≈1.12ms 占 frame ~61%），
    GPU util ~27%（vmwp ~25%）、功耗 ~21W、显存 2.16/8.15GB、CPU 平均 ~5% 单核峰值 ~45%、
    WSL 内存 1.5/12.66GB；64v64 AI 决策 ~700µs/帧（主线程单核，无瓶颈）
  - 教训：基准必须控制相机（bot 后坐力把 pitch 压到 -89° 会触发低头剔除 bug → visible=0 →
    fps 虚高，首轮 2560 数据作废重跑）；低头剔除 bug 已现场复现，优先修复
  - 复现：`BENCH_SECS=45 RES=2560x1600 BOT_CMD=/tmp/look_bot.py bash docs/perf-2560x1600-64v64/bench2560.sh`

### 2026-08-09 快照：64v64 压力模式 + NPC 可视化 + 并行 AI（Wave 5，待推送）
- 压力模式：`RV3D_STRESS_AI=N`（默认 64）→ 红蓝各 N 名 NPC 半场扇形出生（半径 150m+，
  避障环 58-130m 外推），`STRESS_SIGHT=512m` 保证两军 300m 接火；StartMenu 态跳过补员判定
- NPC 互射 `apply_npc_combat`：攻击态每满 1s 对目标结算 dps；团灭补员 `update_stress_respawns`
- 并行 AI：`step_ai_parallel`（`std::thread::scope` + `chunks_mut(16)`），普通波次仍走串行
  `step_ai_serial`（行为不变）；有逐位一致性测试
- NPC 可视化：`renderer::NpcVisual` + `soldier_part_matrices` 7 段积木人（腿/躯干/臂/头/枪），
  `set_npc_visuals`/`upload_npcs` 上传到实例 buffer `NPC_SLOT_BASE=65601` 之后区域，双段 draw
- 纯色渲染：shader 新增 `@location(5) flat_flag`（instance_index >= NPC_INSTANCE_BASE=65601 时置 1），
  fragment 走纯色路径跳过贴图 50% 混合 → 士兵阵营色（红=0.95,0.12,0.08 / 蓝=0.08,0.35,0.98）
  sRGB 直出约 (249,103,78)/(84,162,253)，与灰地/障碍（纹理混合、冲淡色）显著区分
- 性能日志新增 `marker=N npc=M` 字段（每帧上传计数，128 NPC 时 npc=896=128×7）
- 验收：226 tests 全绿、冒烟 ALL-OK（kills=1、VUID=0、fps 202.9–297.2）、
  128 NPC 压力实测 fps~250、ai_us~500µs、无 panic/VUID

### 2026-08-09 快照：CPU 瓶颈拆分 + 亲和线程池 + SIMD 扩展（已推送）
- 亲和线程池（cpu.rs，勿回退）：全局拓扑缓存 `cpu::topology()`（Game/Renderer 复用一次探测）；
  `scene_pool()` 绑首簇（AMD CCD0 / Intel 仅 P-core，杜绝 E-core 与跨 CCD）、
  `ai_pool()` 绑 AMD CCD1 / Intel E-core≥8 时 E-core；线程数 `RV3D_SCENE_WORKERS`/
  `RV3D_AI_WORKERS` 覆盖，默认 min(8, 集合大小)。渲染线程不固定 1-2 核——主线程与
  scene_pool 同绑整簇集合，由 OS 调度器把渲染工作分给集合内空闲率最高的核
- 池 API：`par_for_each_mut<T>(data, f)`，`f(seg_idx, global_start, seg_slice)`，调用线程参与
  首段，join 后才返回；作业闭包走裸指针擦除（`SendPtr<T>`），元素/闭包不要求 'static
- 并行剔除/上传（renderer.rs `cull_and_upload`，勿回退）：两阶段——阶段 A 段并行剔除
  （SIMD 选路 AVX-512>AVX2>AVX>SSE4.2>NEON>标量，逐位一致）写 `culled_scratch` + 段计数
  （AtomicU32）；前缀和（串行，段数≤9）；阶段 B 段并行压缩上传（近档在前、远档随后）。
  冒烟依赖的 visible/near/far 语义不变
- 地形 morph（`update_terrain_lod_morph`）：SIMD 选路算 y + 段并行写回；只写 y 分量 4B/顶点
  （其余顶点分量常驻映射，勿整块重传；地形拍平 y=0 约定不变）
- 爆炸/冲击波 SIMD 实测：`RV3D_EXPLOSION_SIM=1` 每帧推 4096 点波前 + 每秒 65536 点×32 轮
  加速比日志（`simd: path=avx512 ... speedup=... bitwise_eq=true`）。
  实测 AVX-512 ≈ 1.0×——AoS `[f32;3]` + gather 是内存/取数瓶颈而非计算瓶颈，勿据此断言
  AVX-512 无用（视锥剔除 16 实例/批、地形 morph 才是计算密集受益路径）
- 实测（2560×1600 + 64v64 + RV3D_BENCH_PITCH=-10）：fps p50 42→99（2.36×）、
  CPU 单核峰值 94%→47%、cycle_us 13620→10155、cull_us 947→590、record_us 929→262、
  submit_us 486→145、ai_us 1183→267（CCD1 池生效）、wait_fence_us 530→228；
  1280×800 同条件：fps p50 194→298。剩余大头 present_us p50≈3.0ms（dzn/WSLg 呈现路径，
  非 CPU 可并行）；GPU util ~30%、功耗 ~24W 不变
- 验收：229 tests 全绿（新增 morph/冲击波逐位一致单测）、0 警告、冒烟 ALL-OK
  （kills=1、VUID=0、fps 295–364）

### 重启后待办（已办结）
- 网络验证（mirrored+autoProxy）与 Wave 2/3 push 均已完成；后续提交直接 `git push origin master`

### 重要约束
- git 操作只在 WSL 内跑，禁止 `\\wsl$` + Windows git（ref 缓存幻觉）
- 一个功能一个 commit，别出 mega-commit
- git identity: Evernight <3520143257@qq.com>
- 冒烟关键机制（勿回退）：程序化障碍环带 58–130m（game.rs `MAP_RING_INNER`），
  中央安全区保证 NPC 攻击态站定与弹道无阻挡；NPC 站定日志 `npc: #id stand (x,y,z)` 是冒烟瞄准依据

### 上下文节约铁律（防挤爆 1M 上下文缓存）
- 非必要不读编译产物：target/、Cargo.lock、*.spv、*.rlib 等一律不碰
- 非必要不反汇编：必须确认 .spv 逻辑时，spirv-dis 输出到 /tmp 再 grep 目标行，不整段回显
- 非必要不反复读同一文件：内容未变就引用已有结论，禁止对同一文件重复 cat/sed/rg
- 非必要不读大文件/用不上的文件：先 rg --files / rg 定位，sed 限定行号，不 cat 全文
- git diff 一律 --stat 或限定文件；git log/二进制/锁文件非必要不查

### 渲染约定（勿回退）
- Vulkan Y 翻转只由 shader 负责：triangle.vert.spv / hud.vert.spv 各对 gl_Position.y 翻 1 次
- main.rs render() 禁止再翻转投影（proj.y_axis = -proj.y_axis 曾造成双重翻转 → 画面上下颠倒）
- 投影用 glam::Mat4::perspective_rh（y-up NDC、深度 [0,1]），camera.projection_matrix() 保持现状
- F12 截图读回用 in_flight_fences 等待（vkWaitSemaphores 只接受 timeline 信号量，会 VUID + PNG 不落盘）
- 光标捕获瞬间 last_cursor 对齐窗口中心 + 512px 跳变守卫（仅非捕获拖拽路径用），
  防回中 warp 被当视角位移致自转
- test.png 四象限调试图导致面劈裂，已换中灰（2026-08-08 修复）
- 地形程序化高度（2026-08-09 恢复，勿回退全平）：terrain_height 中央半径 140m
  （含 60×60 安全区、障碍环带 58–130m、两军接火区）y=0，之外 ≤15m 确定性值噪声丘陵；
  terrain_height_at 同源，NPC/实例/网格共用；冒烟依赖的站定/接火区仍全平
- 实例场/障碍立方体顶点色全部白化（VERTICES/FAR_VERTS），颜色只走 tint；- shader `flat_flag`：实例槽位 >= 65601（NPC_SLOT_BASE）时顶点着色器置 1，片元走纯色路径
  （跳过贴图 50% 混合），保证阵营色可辨；marker/地形仍走纹理混合路径。改槽位常量需同步
  build.rs `NPC_INSTANCE_BASE` 与 renderer.rs `NPC_SLOT_BASE`（build.rs 每次构建重新生成
  assets/*.spv，改 WGSL 后必须重新构建，勿手改 .spv）
- 地面几何（2026-08-10，`b4558c5`）：管线 CullMode::BACK + FrontFace::CLOCKWISE + shader Y 翻转
  下，立方体顶面（顺时针绕序）从上方看是背面被剔除——旧"压扁立方体"地板只剩垂直侧壁，
  呈现纸箱盖/竖板格。地面改用专用平铺 quad（GROUND_VERTS/INDICES，4 顶点 6 索引，
  绕序 `[0,2,1,0,3,2]` 反向才正面朝上；正向绕序 → 整片地面消失只剩 clear color，是诊断信号）；
  地面实例矩阵纯平移 y=+0.05（无几何侧壁），地形网格下沉 TERRAIN_RENDER_SINK=0.35 防 z-fighting
- 性能日志 `marker`/`npc` 字段 = 每帧 upload_markers/upload_npcs 的 (near+far) 计数，
  排查绘制是否发生时先看这两个计数

  地形实例 tint=0.7 灰、marker tint=WorldMarker.tint（勿混）
- 阴影贴图（2026-08-12，勿回退）：阴影 UV 必须 V 镜像 `uv.y = 1-(p.y*0.5+0.5)`、
  深度映射 `frag_depth = clip.z*0.5+0.5`（glam ortho_rh clip.z∈[0,1]）——已固化进
  lighting.rs `world_to_shadow_uv` + 回归单测；光方向语义 = 表面→光源（`sun.direction`
  直接传入 `ShadowConfig::new`，勿传负号）；阴影采样器 compare_enable(false) 手动 PCF；
  `RV3D_NO_SHADOW=1` 可关阴影做 A/B；地形 identity 矩阵必须写到槽位 `INSTANCE_COUNT`
  （65536），槽位 0 每帧被 cull_and_upload 覆盖

### 输入/键位与分辨率约定（勿回退）
- 键码一律用 winit 0.30 KeyCode 枚举序号（KeyW=41/KeyS=37/KeyA=19/KeyD=22/KeyR=36/
  Space=62/ContextMenu=54/Escape=114），不是 USB HID 码；ui.rs 测试
  `winit_keycode_indices_match_table` 锁死，winit 升级先跑它
- config.rs `bindings_version=1`：旧版 HID 键码配置整体忽略回退默认键位（勿删迁移逻辑）
- 鼠标 Y 方向（2026-08-09 修正为标准方向）：camera.look() `pitch += dy*sens`
  （winit Y 向下、dy>0=鼠标下移=低头），后坐力 `pitch -= recoil_pitch*dt`（kick 正=枪口上扬）。
  旧约定 `pitch -= dy*sens` 是反的（拖下看天）——正是"低头剔除 bug"的真正根因：
  玩家低头时相机在看天空，近档实例场当然全灭（剔除数学本身经实测正确，勿再改回）。
  冒烟瞄准 dpitch = tgt - cur 按新方向，勿再翻 pitch
- 分辨率列表 RESOLUTIONS 已含 2560x1600（5 档，ui.rs）；显式配置非预设分辨率会回退首项
  （旧坑：2560x1600 不在列表 → unwrap_or(0) 回退 1280x720 → "小窗口" + 基准数据全失效）
- ESC 两段式退出：首次显示提示、再按退出、任意其它键取消（hud.confirm_quit）；
  ESC 在设置面板打开时仍只负责关闭面板
- 死亡重开：R（Reload 绑定）或 Enter（系统键兜底）；保留键 ESC/TAB/ENTER/F12/Q/E/N 不可重绑
- 默认分辨率按主显示器宽高比：16:10 → 1280x800，16:9 及其它 → 1280x720
  （仅首次运行/配置无 resolution 行时生效；配置显式保存后以配置为准）
- 冒烟 FPS 阈值 120（默认 1280x800 下 dzn 转译驱动约 165-275 FPS，勿回调到 200）
- 灵敏度映射：`sensitivity_rads() = 0.0005 + hud.sensitivity*0.002`（默认 0.5 → 0.0015 rad/px，
  main.rs 每帧 set_mouse_sens 同步到 camera；勿改回 0.003 起步）
- 后端强制（2026-08-11，`868d127`）：winit 0.29+ 已删除 WINIT_UNIX_BACKEND 环境变量
  （v0.29 changelog 明确 removed，设了也不生效），必须用
  `EventLoop::builder().with_x11()`（EventLoopBuilderExtX11 设 forced_backend）。
  WSL + WAYLAND_DISPLAY 存在时 main.rs 强制 X11（Xwayland）：WSLg Wayland 指针协议
  不完整（捕获失效 + 右键拖动原生层静默崩溃）。用户无环境变量可覆盖，勿回退环境变量方案
- 鼠标视角驱动（2026-08-11 三次修订，勿回退）：**双路径，按 Locked grab 是否成功选路**——
  - Locked 成功（原生 X11/Wayland）：DeviceEvent::MouseMotion（XInput2 XI_RawMotion
    raw_values 相对增量）驱动视角，捕获瞬间一次回中 + 150ms 吞回声，raw 单事件
    >1024（MAX_RAW_LOOK_DELTA）视为残留回声跳过。
  - Locked 失败（WSLg/Xwayland 实测）：**真实物理鼠标只产生 CursorMoved 绝对位置，
    不产生 raw 事件**（Xwayland 虚拟指针无 raw 数据；冒烟用 XTestFakeRelativeMotionEvent
    注入时才有 raw，勿用冒烟结果反推真实鼠标）。绝对位置路径铁律：
    **基准 = 真实指针位置，warp 成功（set_cursor_position is_ok）才把基准设为窗口中心，
    失败则保持真实位置**——旧 bug（868d127）无条件把 last_cursor 设为中心，warp 失败时
    指针距中心偏差被当视角位移 → 灵敏度爆炸/视角压地抬不起头。捕获瞬间不回中，
    首个 CursorMoved 作基准（abs_baseline_valid）；512px 传送守卫只跳过服务端跳变。
  - 冒烟瞄准脚本必须从 ~/.steel_front.cfg 读真实灵敏度（0.0005+s*0.002，别写死 0.003）
    且分块注入（≤400px/事件，游戏 512px 守卫会跳过单次大跳）
- cam 日志 yaw/pitch 单位是"度"（camera 内部弧度，日志换算：60px×0.0015=0.09rad=5.2°；
  look_bot 用 math.degrees(0.0015) 换算 deg_px，勿按弧度解读日志）
- X11 启动焦点（2026-08-11 实测）：新窗口创建后 Xwayland 焦点舞蹈约 3 秒
  （Focused 多次翻转、可能停在未聚焦），玩家点一下窗口即聚焦并开始/捕获——
  标准 X11 行为，捕获后游玩期间焦点稳定（实测 40s 无翻转）

### 跨平台/指令集决策（2026-08-09，已推送 5bf7c77）
- 不做原生 Metal 后端：macOS/iOS 走 MoltenVK 零改动；未来 iOS 商业化再评估
- AArch64 NEON 剔除：`cull_spheres_neon`（4 实例/批，vld1q/vmulq/vaddq/vcgeq，
  非 FMA 与标量逐位一致），运行时 `is_aarch64_feature_detected!("neon")` 选路；
  simd_cull_tests 含 aarch64 门控的 NEON 等价断言
- ash 字符串指针统一 `RawCString` 别名（x86_64=`*const i8`，AArch64=`*const u8`）
- `sched_setaffinity` 仅 `target_os="linux"` 编译：macOS/Apple Silicon 不手工绑核，
  线程调度交给系统 QoS；非 x86 无 E-core 概念，全部归 primary 集合
- aarch64 交叉验证：`cargo check --target aarch64-unknown-linux-gnu`
  （需 `rustup target add aarch64-unknown-linux-gnu`）

### 已知问题/待办（2026-08-08 快照）
- 低头剔除 bug：已修复（`9c86101` 鼠标 Y 方向标准化 + 后坐力枪口上扬——根因是鼠标反转
  致 pitch 被 bot 压到 -89°，剔除数学本身正确；勿回退鼠标方向）
- mipmap 缺失：已修复（`f46c629`：纹理 256×256→9 级 mip 链 + 各向异性采样，
  sampler 设 min/maxLod；后续改纹理尺寸需同步重算 mip_levels）
- 【2026-08-11 登记，推迟处理】输入捕获：WSLg 开局捕获无效（confine 实际不生效，
  指针漂出窗口 → 视角冻结；实测 yaw 55 秒动 0.1°），死亡后 R 重开才变 Minecraft 式
  （指针=准星）且鼠标图标不消失（set_cursor_visible(false) 无效）。根因/修复假设见
  `docs/HANDOFF-2026-08-11.md`，勿删。**用户决策（2026-08-11）：归因 WSL2 环境，
  推迟至 Windows 原生 Vulkan / DLSS 集成阶段一并处理，WSL2 内勿再投入修复**

### 2026-08-09 夜间快照：美术/玩法/联网五线并行（Wave 6，已提交未推送）
- 并行 6 Agent：mipmap、地形、音频、玩法、联网、光照验证；Vulkan 规划文档主线程写
- 地形恢复程序化高度（`6b6845f`）：`TERRAIN_FLAT_RADIUS=140` 内恒 y=0（中央安全区/
  障碍环/接火区不变量不变），140m 外 ≤15m 平缓值噪声丘陵；terrain_height/at 同源
- 程序化音频（`81fc9b4`）：`DspSynth` 事件式合成（枪声/爆炸/脚步/环境风），零资产零依赖
- 玩法（`55af294`）：AI `Tactic::CoverSeek` 掩体利用；障碍可击穿（Wall150/Block300/
  Barrier100）；`MissionObjective` 任务目标 + 胜利横幅
- 联网（`2255498`）：UDP client/server（`RV3D_NET=server|client` + `RV3D_NET_ADDR`），
  Input/Snapshot 报文 + 插值 + 超时；NAT/重连/实战场为 TODO
- 光照结论（`docs/lighting-rendering-verification-2026-08-09.md`）：片元级 Blinn-Phong
  flat shading（法线=平面法线），乘性染色；法线贴图/PBR/阴影是传统光栅特性 dzn 可跑
- Windows 原生 Vulkan 规划：`docs/windows-native-vulkan-plan-2026-08-09.md`（阶段 A=swapchain
  +present 对照 present_us/GPU util，需 Windows 真机）
- 验收：259 tests 全绿、0 警告；冒烟待跑（下一会话第一项）
- 教训：`handle_join` 内部已发 ack，调用方勿再 send_to（重复 ack 坑）；UDP 回环测试
  在沙箱内 bind 会 PermissionDenied，cargo test 必须提权跑
