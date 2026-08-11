# AGENTS.md — Steel Front 项目记忆

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
  - 主线程亲和绑定 `sched_setaffinity`（FFI，无第三方依赖）：默认绑首簇（CCD0），
    `RV3D_CPU_PIN=off` 关闭、`RV3D_CPU_PIN=0-7,16-23` 精确覆盖；Intel 上主线程绑 P-core 组、
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
  `docs/hardware-requirements-2026-08-11.md`（最低=1.3 驱动 4C8T、推荐=8C16T 中端独显、
  最高=16C32T + RTX 40/50 系，瓶颈在 dzn 呈现不在 GPU）
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
- 光标捕获瞬间 last_cursor 对齐窗口中心 + 512px 跳变守卫，防回中 warp 被当视角位移致自转
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
- 鼠标视角驱动（2026-08-11 修订，勿回退）：X11 后端（WSLg/Xwayland）禁用
  DeviceEvent::MouseMotion raw 路径——Xwayland 的 raw 增量异常放大且捕获限制产生持续反馈，
  实测 yaw 自转到万级 rad/s（Xvfb 对照：raw 仅 ~2×、绝对路径精确 60px→5.2°）；
  改走绝对位置 CursorMoved + 每帧回中 + 150ms warp 回声吞噬（512px 跳变守卫）。
  Wayland 后端保留 raw 路径（libinput 相对增量正常，use_relative_mouse=true）
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
- 验收：254 tests 全绿、0 警告；冒烟待跑（下一会话第一项）
- 教训：`handle_join` 内部已发 ack，调用方勿再 send_to（重复 ack 坑）；UDP 回环测试
  在沙箱内 bind 会 PermissionDenied，cargo test 必须提权跑
