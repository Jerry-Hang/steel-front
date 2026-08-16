# 钢铁前线 (Steel Front)

二战题材第一人称射击游戏，使用 **Rust + Vulkan** 从零构建的游戏引擎，**零第三方游戏依赖**——玩法、AI、渲染、音效、联网全部自研，无 Unity/Unreal 式引擎黑盒。渲染底层为 ash 0.38 + winit 0.30 + glam，WGSL 着色器经 naga 30 在 `build.rs` 构建期编译为内联 SPIR-V。

> **项目状态（2026-08-16 文档更新）**
>
> 开发快照 ｜ 单元测试：**364 passed / 0 警告**（dead-code=0）｜ 玩法冒烟：ALL-OK（kills≥1、VUID=0、fps≥120）｜ 渲染主路径：**VK_EXT_mesh_shader 网格着色器（MESH + FRAGMENT）**，在支持扩展的物理设备上自动启用（RTX 5060 Laptop 真机实测启用）；传统 VERTEX + FRAGMENT 顶点管线仅作 WSLg/dzn 回退，**已冻结维护，不再新增功能** ｜ 实例声明 Vulkan 1.3，实际仅用 1.0 核心特性 + `VK_KHR_swapchain`

---

## 一、项目简介

《钢铁前线》是一款以二战战场为背景的 FPS 游戏。项目从空白的 Vulkan 渲染器起步，经过多轮迭代逐步长成包含完整玩法的游戏：第一人称移动与射击、波次式关卡、战术 AI、程序化地图与纹理、程序化音效、HUD/菜单 UI、配置持久化，以及面向压力测试的大规模 NPC 对抗模式。

工程上，本项目把「游戏性引擎」与「渲染/平台层」分开：`src/engine/game.rs` 是每帧更新中枢（物理、武器、AI、UI、音频、网络编排），`src/engine/renderer.rs` 负责 Vulkan 渲染（65536 实例场、地形 LOD、HUD 覆盖层、阴影贴图），`src/main.rs` 承载 winit 事件循环与输入。性能方面投入了大量 CPU 侧优化（SIMD 剔除、亲和线程池、跨平台指令集选路），在 2560×1600 + 128 NPC 的压力场景下仍能保持流畅帧率；Windows 原生驱动下呈现瓶颈消失（present_us 101–373µs），瓶颈已回到渲染/游戏逻辑本身。

## 二、技术特性

### 渲染

- **网格着色器主路径（VK_EXT_mesh_shader）**：物理设备支持扩展时实例场自动走 `MESH + FRAGMENT` 管线——每个 workgroup 负责一个实例槽位，**GPU 端逐实例视锥剔除（Gribb–Hartmann 六平面）+ 顶点变换**，CPU 侧跳过 SIMD 剔除与压缩上传；65536 地面实例 workgroup 按 `maxMeshWorkGroupCount` 查询上限分块下发。片元着色器与顶点路径复用同一模块，渲染结果一致。
- **65536 实例地面场**：地面实例场 + 地形 identity 槽 + 64 障碍 marker + 1024 NPC 积木人（7 段身体部件）+ 32 自发光实体（爆炸闪光）分区槽位；片元按槽位走纹理混合 / `flat_flag` 纯色阵营 / 自发光直出三条路径。
- **阴影贴图 PCF**：定向光 depth-only pass 渲染光空间深度到 2048×2048 D32 阴影图（正交投影覆盖 250m 半宽），主 pass 片元 3×3 PCF 深度比较 + depth/normal bias 缓解 acne；`RV3D_NO_SHADOW=1` 可关阴影做 A/B 验证。
- **程序化纹理**：CPU 画像素零资产产出——地面材质（草地/沙地/石板/道路/焦土弹坑 + 烘焙 AO + 静态天光，`RV3D_PROC_TEX=0` 回退 test.png）、障碍木板墙皮肤、NPC 迷彩军服皮肤（`RV3D_SKIN_TEX=1` 启用，缺省纯色回退）。
- **第一人称枪模**：M1 加兰德积木拼装（胡桃木枪托/护木 + 磷化钢金属件），视空间固定、贴近 near 平面；开镜（ADS）时按 FOV 反比补偿缩放，保证枪模视觉大小恒定。
- **几何双档 LOD**：近档立方体（120m 内，随画质调整）、远档十字双 quad；地面专用平铺 quad（特殊绕序防背面剔除），地形下沉 0.35 防 z-fighting，远档地面 400–900m 淡出。
- **程序化地形**：257×257 顶点、三级网格密度 + smoothstep LOD morph；中央 140m 半径接火区拍平（战斗公平 + 弹道无阻），外围 ≤15m 确定性值噪声丘陵。
- **画质三档预设**：低 / 中 / 高分别对应地形 LOD 阈值与实例近档半径。
- **性能路径**：视锥剔除五级 SIMD 选路（AVX-512 > AVX2 > AVX > SSE4.2 > NEON > 标量，逐位一致）、两阶段并行剔除 + 上传、CPU 拓扑检测与主线程/线程池亲和绑定（`RV3D_CPU_PIN` 可调）。

### 玩法系统

- **FPS 移动**：WASD + 玩家刚体碰撞推回 + 地形贴地，第一人称相机（灵敏度可调）；**Space 跳跃**（JUMP_SPEED=3.3，约 0.55m 跳高，2026-08-15 调低去除“月球漫步”感）。
- **武器系统**：M1 加兰德（25 伤 / 3 发每秒）+ Thompson SMG（12 伤 / 10 发每秒）+ 手榴弹（抛物线 + 引信 1.5–2.5s）；弹匣/备弹/换弹进度、后坐力 kick、命中反馈 hit marker、开火音效；数字键 1/2 或滚轮切枪（0.6s 切换计时）。
- **波次与关卡**：每关 3 波、清关重建地图并递进难度；每 5 波 Boss 主怪、每 3 波援军增援；程序化地图确定性种子生成，三种主题轮换。
- **爆炸多层视觉**：AoE 伤害 + 径向击退 + 震屏 + 自发光闪光 + 枪口焰/弹壳粒子（重力落地），爆炸可摧毁障碍（AoE 结算）。
- **玩家状态与任务**：受伤/死亡/结算/一键重开，任务目标与胜利横幅；补给键补满弹药与手榴弹。

### 战术 AI

- A* 寻路 + NPC 状态机（巡逻 / 追击 / 攻击 / 掩体），NPC 为 7 段积木人可视化 + 阵营色/迷彩皮肤。
- 战术行为：就近掩体搜索（CoverSeek）、包抄（Flank）、协同冲锋、躲避、偷袭绕路（Ambush）、掩护射击；压力模式下红蓝两军互射、团灭自动补员。
- AI 决策走亲和线程池并行（AMD 绑 CCD1 / Intel 绑 E-core 或 P-core）；`RV3D_AI_PARALLEL=off` 可串行 A/B，`RV3D_AI_DECIMATE=off` 可关远组降频。

### 音效

- 程序化 DSP 合成（枪声 / 爆炸 / 脚步 / 环境风 + 手榴弹哨声/落地声），零音频资产、零额外依赖；M1 与 Thompson 音色参数化区分。
- 程序化环境音乐：低音 pad + 112BPM 行军节奏 + A 小调五声旋律动机，菜单/战斗按状态淡入淡出。

### UI / 输入

- **HUD（GDI 中文字形系统）**：`font_cjk.rs` 用 Windows GDI 把 CJK 字符（微软雅黑）光栅化为 8×8 点阵掩码，按需生成缓存；配合内置 5×7 ASCII 位图字体自绘文本，零外部字体依赖。
- **ESC 菜单**：半透明毛玻璃观感菜单（全屏暗色遮罩 + 居中面板），Tab 切换选中、Enter 确认（退出游戏 / 打开设置）、ESC 关闭。
- **击杀提示**：右上角战地风格 kill feed（最多 4 条、6 秒消退），玩家击杀 NPC / NPC 互杀 / NPC 杀玩家三处钩子。
- **设置面板**：键位绑定（重复键自动互斥）/ 分辨率（5 档至 2560×1600）/ 灵敏度 / 音量 / 音乐音量 / 画质，即时生效并持久化。
- HUD 布局按 1280×800 设计空间计算、出口统一乘 ui_scale 缩放，高分辨率下字体/面板等比放大。

### 联网（基础版）

- UDP client/server（`RV3D_NET=server|client` + `RV3D_NET_ADDR`），手写大端字节编码协议（magic/version/type/length）；输入上报、整帧快照广播、远端插值平滑、乱序/重复丢弃、超时处理；ObjectiveState(0x07) 消息广播据点归属/进度。NAT 穿透、断线重连与实战场联机为后续路线。

### 配置持久化

- `~/.steel_front.cfg`：键位 / 音量 / 灵敏度 / 分辨率 / 画质，原子写 + 容错加载（坏行忽略、缺省回退）；旧版本键位配置自动回退默认（`bindings_version=1`）。

## 三、开发方向（重要）

> **传统顶点着色器管线冻结维护，全面转向 VK_EXT_mesh_shader 网格着色器开发。**

- **网格着色器是唯一主开发路径**：所有新渲染功能、性能优化、视觉迭代都在 mesh 路径（`MESH + FRAGMENT`）上进行。
- **mesh 路径在支持扩展的设备上自动启用**（如 NVIDIA RTX 5060 等，真机实测 VK_EXT_mesh_shader=true），无需任何环境变量干预。
- **顶点路径（VERTEX + FRAGMENT）仅作 WSLg/dzn 回退**：为不支持扩展的转译环境保留可运行性，**不再为它新增任何功能**，只做必要的兼容性维护（冒烟基线仍双路径验证零回归）。
- 配套技术路线：项目与 DLSS、光线追踪硬件启用同期推进，提前适配支持 VK 图形 API 新特性的显卡，避免后期硬件断层。

## 四、构建与运行

### Windows 原生构建（当前正式开发/验证环境）

需要 rustc 1.96+ 与 VS 2022/2026 C++ 工具链（MSVC）。Windows 原生驱动下 VK_EXT_mesh_shader / 光追 / DLSS 探测全部可用（WSLg/dzn 不可用）。

```bash
# Release 编译（Rust stable，需支持 Vulkan 1.3 的驱动；build.rs 构建期编译 WGSL→SPIR-V）
cargo build --release

# 运行
./target/release/steel-front

# 单元测试（纯逻辑，不触碰 GPU）
cargo test

# 玩法冒烟测试（Windows 原生，SendInput 注入 + 日志断言，约 30 秒）
powershell -ExecutionPolicy Bypass -File scripts/run_gameplay_smoke.ps1
```

### SteelFront.bat 启动器

项目根目录提供 `SteelFront.bat` 一键启动器：**先杀残留游戏进程（taskkill steel-front）再执行 `cargo build --release`，构建成功后启动游戏**。先杀进程是为了避免旧的 steel-front.exe 占用 assets/*.spv 与交换链资源导致构建/链接失败或启动异常（与冒烟启动器“清场再启动”同一约定）。

### WSL2 说明（历史）

早期开发在 WSL2（WSLg/dzn Vulkan 转译层）进行，2026-08-15 起已整体迁移至 Windows 原生（呈现瓶颈 present_us 1–2ms 消失、GPU 能力全解锁）。WSL2 相关内容（X11 冒烟、dzn 转译、`docs/perf-*` 基准）**保留作历史记录，不再作为开发/验证环境**。

## 五、配置

### 配置文件

首次运行自动生成 `~/.steel_front.cfg`（Windows: `C:\Users\<user>\.steel_front.cfg`；不进入仓库目录）。可持久化键位绑定（`bind_*`）、音量、灵敏度、分辨率与画质预设；配置损坏/坏行时自动回退默认值，不会导致启动失败。显式保存的分辨率不在预设列表时会回退首项（1280×720），默认分辨率按主显示器宽高比选择（16:10 → 1280×800）。

### RV3D_* 环境变量（全部以代码实测为准）

| 变量 | 说明 | 默认值 |
|---|---|---|
| `RV3D_PRESENT_MODE` | 交换链呈现模式：`immediate` / `mailbox` / `fifo` | `mailbox` |
| `RV3D_PROC_TEX` | `0` 关闭程序化地面材质，回退 `assets/textures/test.png`（A/B 验证） | 启用程序化纹理 |
| `RV3D_SKIN_TEX` | `1` 启用障碍/NPC 程序化皮肤纹理（木板墙/迷彩军服），缺省纯色回退 | `0` |
| `RV3D_NO_SHADOW` | `1` 关闭阴影贴图（仅环境光+点光源），阴影 A/B 验证用 | `0` |
| `RV3D_NPC_SCALE` | NPC 数量整体缩放（`max(0.5)`），压测/低配调低用 | `1.0` |
| `RV3D_STRESS_AI` | 设置即启用压力模式：红蓝各 N 名 NPC 大战场对抗（N≥4） | `64` |
| `RV3D_CPU_PIN` | 主线程 CPU 亲和：`off` 关闭，或如 `0-7,16-23` 精确指定 | 首簇（AMD CCD0 / Intel P-core） |
| `RV3D_DISABLE_AVX512` | `1` 强制关闭 AVX-512 剔除选路（Intel 11 代起默认按策略关闭） | 按策略自动 |
| `RV3D_SCENE_WORKERS` / `RV3D_AI_WORKERS` | 场景 / AI 亲和线程池线程数 | `min(8, 集合大小)` |
| `RV3D_AI_PARALLEL` | `off` 关闭并行 AI 更新（串行 A/B 对比） | 并行 |
| `RV3D_AI_DECIMATE` | `off` 关闭远组 AI 降频（红线：攻击/感知/受击/被瞄准恒每帧） | 降频 |
| `RV3D_FORCE_SIMD` | 强制锁定 SIMD 剔除档位：`avx512` / `avx2` / `avx` / `sse4.2` / `scalar` | 按硬件自动选路 |
| `RV3D_BENCH_YAW` / `RV3D_BENCH_PITCH` | 基准测试固定相机朝向（度），保证对照实验视角一致 | 无 |
| `RV3D_EXPLOSION_SIM` | `1` 运行爆炸/冲击波 SIMD 压力自测（加速比日志） | `0` |
| `RV3D_NET` / `RV3D_NET_ADDR` | 联网模式（`server` / `client`）与对端地址 | 无 |
| `RV3D_MAP` | 启用关卡系统并加载单张地图（如 `assets/maps/street_fight.toml`） | 无（程序化地图） |
| `RV3D_MAPS` | 启用关卡系统并加载关卡列表（如 `assets/maps/index.toml`，`N` 键按序进入下一关） | 无（程序化地图） |

### 关卡系统（TOML 地图）

游戏默认使用程序化生成地图（不设置任何环境变量时行为与旧版一致）。设置 `RV3D_MAP=<单关 toml>` 或 `RV3D_MAPS=<index.toml 列表>` 后启用关卡系统：障碍物、出生点、目标与胜负规则全部由 TOML 关卡文件描述（capture / kill / time / survive 四种规则），物理碰撞 / AI 导航 / 渲染 marker 自动适配。游玩中按 `F5` 热重载当前地图。内置 5 张关卡：巷战废墟（capture）、开阔地（kill 30）、工厂伏击（kill 40）、桥头堡（time 180s）、防线（survive 5 波）。详细格式见 `assets/maps/*.toml` 与 `assets/maps/index.toml`。

### 键位操作

| 操作 | 键位 |
|---|---|
| 移动 | `W` / `S` / `A` / `D` |
| 跳跃 | `Space` |
| 开火 | 鼠标左键 |
| 开镜瞄准（ADS） | 鼠标右键按住（准星收窄 + 枪模居中 + FOV 70°→45°） |
| 换弹 | `R` |
| 切枪 | `1` / `2` 或滚轮 |
| 投掷手榴弹 | `G` |
| 设置面板 | `ContextMenu`（物理菜单键）；`Tab` 循环选中项，`Enter` 确认 |
| 补给 | `N` |
| 视角辅助（下蹲 / 起立） | `Q` / `E` |
| 截图 | `F12`（保存到 `/tmp/steel_front_*.png`） |
| 菜单 / 退出 | `ESC`（打开毛玻璃菜单，再按/Enter 确认退出；死亡后 `R` 或 `Enter` 重开） |

> 设置面板中键位行支持重绑定（重复键自动互斥）；`ESC` / `Tab` / `Enter` / `F12` / `Q` / `E` / `N` 为保留系统键，不可重绑。

## 六、测试与质量门槛

- **单元测试**：`cargo test`，当前基线 **364 个测试全绿 / 0 失败 / 0 警告**（纯逻辑测试，不触碰 GPU；含 UDP 回环需提权环境）。
- **dead-code = 0 硬红线**：不允许任何未使用代码警告（map.rs / objective.rs / ui.rs 等模块的 `#![allow(dead_code)]` 已随接线移除）。
- **玩法冒烟**：`scripts/run_gameplay_smoke.ps1`（Windows 原生，SendInput 注入 + 日志断言，约 30s）——断言 **VUID=0（无 Vulkan 校验错误）、kills≥1（至少一次击杀）、fps≥120（min）**，yaw/pitch 有视角变化、hp/wave 有日志、无 panic；全部通过输出 `ALL-OK`。游戏 stdout → `smoke.log`、stderr → `smoke.log.err`。
- 渲染管线/pipeline/shader/swapchain 改动属高风险区，必须跑冒烟验证 VUID 零回归。

## 七、硬件要求

游戏实例声明 Vulkan 1.3，实际功能需求仅为 Vulkan 1.0 核心 + `VK_KHR_swapchain`（+ 可选各向异性过滤）。渲染默认走 `VK_EXT_mesh_shader` 网格着色器路径（支持时自动启用，不支持自动回退顶点路径）。**测试版最低线建议为支持网格着色器的显卡**：AMD RX 6500 XT（RDNA 2）或 NVIDIA RTX 20 系及以上，或 Intel Arc 全系列。

**实测平台**（RTX 5060 Laptop + Ryzen 9 8940HX，Windows 原生驱动 610.88）：VK_EXT_mesh_shader=true（网格路径真机启用）、光追 RT pipeline/AS/ray_query=true、DLSS VK_NVX=true、present_us 101–373µs；压力模式 2560×1600 + 64v64（128 NPC）150–400fps，1280×800 下 300+fps。

## 八、已知注意事项

- **naga 30 对网格着色器 vertices 数组内 position 的 ADJUST_COORDINATE_SPACE 翻转失效**：顶点路径由 naga 自动做 Y 翻转，但 mesh 路径 `@builtin(vertices)` 数组内的 `@builtin(position)` 不生效，因此 mesh 着色器在 WGSL 内**显式翻转 `gl_Position.y`**（见 `build.rs` 注释）；删除该翻转会导致 mesh 渲染内容（地面场/NPC/枪模）垂直镜像。
- **12GB RAM 内存限制**：一次只跑一个 cargo（构建/测试/冒烟串行），禁止并行构建，避免内存吃满。
- 冒烟 FPS 阈值 120 是针对 Windows 原生呈现路径的基线，勿回调到 200。
- WSLg/dzn 转译层不支持 VK_EXT_mesh_shader，该环境下自动走顶点路径回退。

## 九、2026-08-16 修复记录

- **世界垂直镜像修复（mesh 路径 Y 翻转）**：naga 30 网格写入器对 `@builtin(vertices)` 数组内 position 的 ADJUST_COORDINATE_SPACE 翻转失效，导致 mesh 路径渲染内容垂直镜像；已在 WGSL mesh 着色器内显式 `v.position.y = -v.position.y` 补齐（build.rs，勿删）。
- **HUD 双重缩放修复（设计空间布局）**：HUD 布局统一按 1280×800 设计空间计算，`layout_elements` 出口一次性乘 `ui_scale`；修复此前部分元素直接用 screen_w/h 导致的**双重缩放**（高分辨率下血条/准星被推出屏幕）。ESC 菜单遮罩/面板同样按设计空间计算。
- **窗口/交换链尺寸自动校验**：每帧诊断窗口 inner_size vs swapchain extent vs HUD 尺寸，检测到不匹配（DPI 缩放、全屏切换、resize 等）自动重建交换链（`size mismatch: window=… swapchain=… → 重建交换链`），viewport/scissor 每帧按当前 swapchain_extent 重设。
- **开镜 FOV 补偿**：ADS 时 FOV 70°→45° 平滑过渡（指数逼近），透视放大约 1.55×；第一人称枪模按 `scale = tan(70°/2) / tan(fov/2)` 反向补偿缩放（clamp 0.5–1.0），保证开镜时枪模视觉大小恒定、不穿模。
- **跳跃物理**：Space 跳跃（2026-08-15 起开火改鼠标左键，Space 让位）；跳跃初速 `JUMP_SPEED=3.3`（约 0.55m 跳高），从 4.6 调低去除“月球漫步”感，贴近真实二战士兵跳跃。
- **中文 HUD 字形系统**：新增 `font_cjk.rs`——Windows GDI（微软雅黑）把 CJK 字符光栅化为 8×8 点阵掩码、按需生成缓存，与内置 5×7 ASCII 位图字体组成中文 HUD 字形系统；非 Windows 平台回退 `?`。

## 十、版本迭代历史（摘要）

| 日期 | 里程碑 | 要点 |
|---|---|---|
| 2026-08-05 ~ 08-08 | Wave 1–4 | Vulkan 渲染器接线、FPS 玩法、程序化地图、波次/主题/画质/截图、渲染与输入修复 |
| 2026-08-09 ~ 08-10 | Wave 5–6 | 64v64 压力模式、并行 AI、SIMD 五级选路、亲和线程池、程序化地形/音效、UDP 联网基础版、爆炸冲击波 |
| 2026-08-11 | 文档重构 + 网格着色器可选路径 | README/AGENTS.md 重构；VK_EXT_mesh_shader 可选路径落地（自动启用/回退） |
| 2026-08-12 ~ 08-13 | 阴影贴图 / 美术 / 线程优化 | depth-only pass + 3×3 PCF；程序化地面/皮肤纹理、烘焙 AO、光照烘焙；物理核/超线程分层绑定 |
| 2026-08-14 | 指令单 #1–#4 | 关卡系统（TOML 四规则五图）、据点标记、AI 战术扩展、Thompson/手榴弹、survive 规则、爆炸纵深、音效差异化 |
| 2026-08-15 | Windows 原生迁移 | WSL2→Windows 原生（RTX 5060 真机）；ESC 毛玻璃菜单、击杀提示、冒烟移植、鼠标方向修正 |
| 2026-08-16 | 修复记录 + 方向转向 | 世界镜像/双重缩放/交换链校验/FOV 补偿/跳跃物理/中文字形修复；**顶点管线冻结、网格着色器优先** |

## 十一、路线图

- **网格着色器深化（当前主攻）**：在 mesh 路径上继续渲染纵深开发（阴影 pass 增强、法线贴图/PBR 等传统光栅特性的 mesh 版本）。
- **美术资产**：贴图与模型由程序化生成过渡到 AI 生成/人工制作。
- **音效**：DSP 合成基础上增加环境音乐与更丰富的混音/空间化。
- **联网**：NAT 穿透、断线重连、多人在线实战场。
- **渲染技术栈迁移（中后期）**：全面放弃传统顶点着色器、以网格着色器为唯一渲染路径；该任务与 DLSS、光线追踪硬件启用同一优先级。
- **Windows 原生能力**：光追 RT pipeline、DLSS（VK_NVX）已探测可用，规划后续启用（分阶段规划见 `docs/windows-native-vulkan-plan-2026-08-09.md`）。

---

*钢铁前线 —— 从零构建的二战 FPS 引擎。*
