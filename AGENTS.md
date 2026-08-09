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
  - AI/地图生成现阶段仍单线程（每帧强耦合玩家状态/协同，跨线程需双缓冲+同步，破坏冒烟确定性），
    未强行线程化；secondary_set 语义已预留
  - renderer 剔除五级选路：avx512f（16 实例/批，Zen4/Zen5 原生 512 位）> avx2（8）>
    avx（8，3/4 代酷睿与初代锐龙）> sse4.2（4，2008 年后全平台）> 标量，
    各级路径与标量逐位一致（非 FMA）；`_mm512_*` 在 Rust stable 可用（实测本机 avx512f=true）。
    非 x86_64 平台走标量兜底（cfg(not) 分支，勿删）
- `.wslconfig` 已配置 `[wsl2] networkingMode=mirrored + dnsTunneling + firewall + autoProxy`，待 `wsl --shutdown` 生效
- 验收快照：176 tests passed、0 警告、20s 冒烟 ALL-OK（kills=1、VUID=0、fps 214.8–292.7）

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
- 地形已全图拍平 y=0（terrain_height 返回 0.0，terrain_height_at 同源；NPC/实例/网格共用）
- 实例场/障碍立方体顶点色全部白化（VERTICES/FAR_VERTS），颜色只走 tint；
  地形实例 tint=0.7 灰、marker tint=WorldMarker.tint（勿混）

### 输入/键位与分辨率约定（勿回退）
- 键码一律用 winit 0.30 KeyCode 枚举序号（KeyW=41/KeyS=37/KeyA=19/KeyD=22/KeyR=36/
  Space=62/ContextMenu=54/Escape=114），不是 USB HID 码；ui.rs 测试
  `winit_keycode_indices_match_table` 锁死，winit 升级先跑它
- config.rs `bindings_version=1`：旧版 HID 键码配置整体忽略回退默认键位（勿删迁移逻辑）
- 鼠标 Y 方向：camera.look() `pitch -= dy*sens`（winit Y 向下）为标准方向，
  /tmp/probe_mouse.py X11 实测确认、冒烟瞄准按此方向，禁止再翻 pitch
- ESC 两段式退出：首次显示提示、再按退出、任意其它键取消（hud.confirm_quit）；
  ESC 在设置面板打开时仍只负责关闭面板
- 死亡重开：R（Reload 绑定）或 Enter（系统键兜底）；保留键 ESC/TAB/ENTER/F12/Q/E/N 不可重绑
- 默认分辨率按主显示器宽高比：16:10 → 1280x800，16:9 及其它 → 1280x720
  （仅首次运行/配置无 resolution 行时生效；配置显式保存后以配置为准）
- 冒烟 FPS 阈值 120（默认 1280x800 下 dzn 转译驱动约 165-275 FPS，勿回调到 200）
- 灵敏度映射：`sensitivity_rads() = 0.0005 + hud.sensitivity*0.002`（默认 0.5 → 0.0015 rad/px，
  main.rs 每帧 set_mouse_sens 同步到 camera；勿改回 0.003 起步）
- 鼠标视角驱动：捕获态优先 DeviceEvent::MouseMotion（XInput2 相对增量，与光标位置无关）；
  WSLg/Xwayland 下 set_cursor_grab(Locked) 返回 Err、Confined 无效，绝对位置+warp 路径
  会产生回声乱转，勿回退到纯 CursorMoved+warp

### 已知问题/待办（2026-08-08 快照）
- 低头剔除 bug：pitch < 约 -30° 时近档实例场被视锥剔除全灭（日志 visible=near=0），
  画面只剩远档+雾；排查 extract_frustum_planes / near plane（与地形拍平无关，既有问题）
- mipmap 缺失：纹理 mip_levels(1) + sampler mipmap_mode(LINEAR) 无 mip 链，
  地平线远处锯齿闪烁（渲染约定勿回退：加 mip 链需同步放宽 image view/sampler）
