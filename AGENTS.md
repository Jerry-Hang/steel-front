# AGENTS.md — Steel Front 项目记忆与 AI 交接文档

## 项目
二战题材 FPS，Rust + Vulkan（winit 0.30），零第三方游戏依赖，纯 bin crate。
- 入口：`src/main.rs`（GameApp + winit 事件循环）
- 运行时中枢：`src/engine/game.rs`（每帧 `update(dt, camera)` 编排物理/武器/AI/UI/音频/网络）
- 渲染：`src/engine/renderer.rs`（地形 LOD + 65536 实例场 + HUD 覆盖层；改 pipeline/shader/swapchain 风险高，须先跑冒烟验 VUID）
- 地形高度纯函数在 renderer.rs（`terrain_height` / `terrain_height_at`），中央 60×60 压平 y=0
- 配置持久化：`src/config.rs` → `$HOME/.steel_front.cfg`（原子写 + 容错加载，测试不写盘）
- 测试：`cargo test`（纯逻辑，不碰 GPU）；冒烟 `bash scripts/run_gameplay_smoke.sh`（需 X/Vulkan，20s，断言 kills>0）
- 验收约束：dead-code=0（0 警告）、测试全绿、不新增第三方依赖、commit 规范 `feat(game)/chore/docs`
- 内存约束：12GB，一次只跑一个 cargo，禁止并行构建

## 迭代规划与交接日志（AI 交接规范）

本文件是**唯一的正式 AI 交接载体**（项目记忆 + 迭代规划 + 交接留痕一体化）。所有 AI 会话（含并行智能体，如 Newton）在本仓库工作，必须遵守以下交接协议：

- **规划开启**：每次迭代/任务规划开始时，必须在本节登记：目标、任务拆解、负责人、状态（in_progress）。
- **迭代结束**：迭代结束时必须在本节写完成记录：完成项、验收结果（测试数/警告数/冒烟结果）、遗留问题与下一步。
- **AI 间交接**：AI 与 AI 之间的所有交接必须在此留痕——上下文交接（记忆/结论迁移）、任务交接（接手/移交）、绘画/美术素材交接（素材路径、用途、规格、验收标准），格式：日期、发起方、接收方、交接内容、状态。
- **git 提交规范不变**：一个功能一个 commit，禁止 mega-commit；提交信息 `feat/chore/docs/fix` + 范围（如 `feat(game)`、`docs(AGENTS.md)`）；git 操作只在 WSL 内跑，禁止 `\\wsl$` + Windows git。

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
