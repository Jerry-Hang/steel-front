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
  - **背景**：WSL2/dzn 转译层瓶颈（present_us 1-2ms）、GPU 能力全锁（mesh/RT/DLSS 全 false）、
    宣发目标是 Windows——经用户决策，开发环境整体迁移 Windows 原生（RTX 5060 真机）。
  - **① 迁移内容**：仓库已 clone 至 Windows（git clone + push 权限实测 OK）；rustc 1.96.1 +
    VS 2026 已在本机；修复 2 处跨平台编译问题（cpu.rs AMD 分支 collect 类型标注 +
    main.rs 非 Linux 平台 force_x11 冗余变量），commit `4504f89` fix(win)。
  - **② Windows 原生能力解锁（实测，勿回退）**：VK_EXT_mesh_shader=true（网格着色器管线
    首次真机创建成功，WSL2 冻结两个月后开光）、光追 RT pipeline/AS/ray_query=true、
    DLSS VK_NVX=true（可直接接 DLSS SDK）、present_us 101-373µs（WSL2 的 1-2ms 瓶颈消失）。
  - **③ 冒烟移植（commit 待登记）**：scripts/gameplay_smoke_win.py（SendInput 替代 XTest、
    FindWindowW 替代 XQueryTree、日志 UTF-8 容错读）+ scripts/run_gameplay_smoke.ps1
    （启动器：杀进程/双文件重定向/断言）。实测 ALL-OK：VUID=0、fps 262-325、kills=1、
    hit=4、yaw/pitch 视角注入正常（一次 -1753px 注入即收敛瞄准）。
  - **④ 环境铁律更新（Windows 侧生效）**：git 操作可在 Windows 直接跑（push 走 GitHub
    令牌，仅限 steel-front 仓库）；cargo 构建/测试/冒烟均在 Windows 原生执行；12GB
    内存约束不变（一次一个 cargo）；WSL2 不再承担开发/验证（可关）。
  - **⑤ 遗留/下一步**：A. 呈现层欠账（枪模/动画/粒子/弹孔）在真 GPU 上开发——总指挥
    将直接改代码（用户已授权）；B. playtest_perf.py 的 Windows 移植未做（冒烟已够基线）；
    C. 输入捕获遗留（WSL2 冻结项）在原生 win32 下验证，大概率已解决（冒烟视角注入成功
    即证据）；D. mesh 路径真机验证 + DLSS 立项评估。
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