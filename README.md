# 钢铁前线 (Steel Front)

> 架空历史 · 2020 年代 · 大规模战场 FPS
> Rust + Vulkan 自研引擎，零第三方游戏依赖

**一句话**：从零构建的现代大战场射击游戏引擎与玩法原型——程序化渲染、程序化建模、程序化音效、程序化地图，无 Unity/Unreal 黑盒，无外部资产。

---

## 项目状态（2026-08-19）

| 维度 | 状态 |
|---|---|
| 单元测试 | **384 passed / 0 警告**（dead-code=0） |
| 玩法冒烟 | ALL-OK（kills≥1、VUID=0、fps≥120） |
| 渲染主路径 | **VK_EXT_mesh_shader 网格着色器**（RTX 5060 真机启用） |
| 当前阶段 | **设计文档整理期**（玩法开发暂停，等待设计文档定稿） |
| 设计文档 | [GAME_DESIGN.txt](./GAME_DESIGN.txt)（弹药/护甲/爆炸/手雷/载具规划） |

---

## 转型说明（重要）

项目原为二战题材，现已决定**转型为架空历史 2020s 现代大战场**。原因：

1. **竞争力**：二战大战场与头部产品（战地系列）正面竞争无优势；
2. **武器平衡**：二战枪械 TTK 长、栓动主导、平衡空间窄，现代口径矩阵（5.56/7.62×39/7.62×51/9×19）有成熟平衡模板；
3. **载具体验**：二战空战各打各的、轰炸机超模、坦克国别差距离谱，现代轻型载具与支援型空袭（空袭呼叫/无人机）体验更好且平衡可控；
4. **开发时机**：当前进入美术/建模阶段，此时转型成本最低（引擎/AI/地图生成全部与题材无关，仅武器数据与主题资产需更换）。

**转型范围**：武器库数据、枪械程序化网格、地图主题元素、NPC 外观 → 换为现代版本；引擎/渲染/网络/程序化管线不变。

---

## 技术架构

### 图形 API 与着色器

| 项目 | 说明 |
|---|---|
| API | Vulkan（ash 0.38），实例声明 1.3，实际使用 1.0 核心 + VK_KHR_swapchain |
| 窗口 | winit 0.30（Windows/Linux） |
| 数学 | glam 0.29 |
| 着色器语言 | WGSL，经 **naga 30 在 build.rs 构建期编译**为内联 SPIR-V（assets/*.spv） |
| 主渲染路径 | **VK_EXT_mesh_shader（网格着色器）**：GPU 逐实例视锥剔除 + 顶点变换，65536 实例地面场按 maxMeshWorkGroupCount 分块下发。**全面转向网格着色器，后续迭代不再维护传统顶点着色器路径** |
| 传统顶点管线 | **已冻结**：不再新增功能、不再修复、**不提供回退**。不支持 VK_EXT_mesh_shader 的显卡下载最新版本无法游玩（最低要求见下方硬件推荐） |
| HUD | 独立屏幕空间管线（CPU 转 NDC），GDI 中文字形系统（Windows） |
| 阴影 | 2048×2048 D32 阴影图 + 3×3 PCF |

### 程序化生成（零外部资产）

- **建模**：meshgen.rs 程序化网格引擎——圆角盒（beveled box）/ 锥台 / 圆柱 / 球 / 圆环弧段，法线烘焙光照；枪械等模型由数学函数生成
- **纹理**：CPU 画像素——地面材质（草地/沙地/石板/道路 + 烘焙 AO）、皮肤纹理
- **地形**：257×257 顶点、三级 LOD + smoothstep morph、确定性值噪声
- **音效**：程序化 DSP 合成（枪声/爆炸/脚步/环境）
- **地图**：确定性种子生成（障碍/据点/波次）

### 游戏性

- 第一人称移动/跳跃/射击/开镜（ADS FOV 补偿）/手雷/切枪
- 波次防守 + 关卡递进 + Boss/援军波
- 战术 AI：A* 寻路、状态机（巡逻/追击/攻击/掩体）、包抄/偷袭/协同、压力模式两军互射
- HUD：血条/弹药/准星/命中标记/击杀提示/小地图/ESC 菜单/设置面板（键位/分辨率/灵敏度/音量/画质）
- 联机基础：UDP client/server、快照同步（规划中扩展）

---

## 当前进度（2026-08-16 快照）

**已完成**
- Vulkan 渲染全链路（交换链/管线/UBO/实例场/阴影/纹理）
- mesh shader 主路径（GPU 剔除/实例变换）
- 程序化地形/纹理/音效/建模（meshgen）
- 第一人称战斗循环（武器/弹药/换弹/后坐力/命中反馈）
- 战术 AI + 波次关卡
- HUD/菜单/设置/中文界面（GDI 字形）
- 配置持久化 + 性能日志（帧率/阶段耗时）
- 冒烟自动化（VUID/击杀/fps 门槛）

**进行中**
- 设计文档整理期：弹药口径体系 / 护甲系统 / 爆头机制 / 爆炸三段 / 手雷体系（见 GAME_DESIGN.txt）
- UI 美化与中文字形完善
- 性能日志增强（每次启动打包存档）

**规划**
- 现代武器库（M4A1 / AK-74M / MP5 / M14 EBR 等）
- 护甲/体力/爆头机制落地
- 地图主题现代化（集装箱/路障/废弃车辆）
- 载具与支援系统（轻载具/空袭/无人机，远期）

---

## 硬件推荐配置

> 目标：1080p~4K 大战场体验（网格着色器渲染，需支持 VK_EXT_mesh_shader 的显卡）

| 画质档位 | 处理器（Intel/AMD 任一） | 显卡（NVIDIA/Intel/AMD 任一） | 内存 |
|---|---|---|---|
| 1080P 最低 | 11 代酷睿 或 3300X | A380 或 RX 6500 XT | 最低 8GB，推荐 12GB |
| 1080P 高画质 | 9900 或 3700X | RTX 2060 SUPER 或 A580 | 12GB+ |
| 2K 低画质 | 11700K 或 5700X | RTX 2080 SUPER 或 A770 | 12GB+ |
| 2K 高画质 | 12600KF 或 5800X | RTX 3060 Ti 或 B580 | 16GB+ |
| 4K 低画质 | 12700K 或 7700X | RTX 3080 12GB 或 RTX 4070 | 16GB+ |
| 4K 高画质 | 13700K 或 7900X | RTX 4070 Ti Super 或 RX 9070 GRE | 32GB+ |

**当前开发验证环境**：RTX 5060 Laptop（8GB）+ Ryzen（16 核 32 线程）+ 2560×1600@144Hz，release 构建稳定 330–420 fps（65536 实例场压力场景）。

---

## 构建与运行

### 前置

- Rust 工具链（stable，MSRV 以 Cargo.toml 为准）
- Windows 10/11 或 Linux（WSLg 可运行但性能受限）

### 构建

```bash
cargo build --release
```

### 运行

- Windows：直接运行 `target/release/steel-front.exe`，或使用仓库内 [SteelFront.bat](./SteelFront.bat)（先杀残留进程 → 自动拉取更新 → 构建 → 启动）
- Linux：`cargo run --release`

### 冒烟测试

```bash
powershell -ExecutionPolicy Bypass -File scripts/run_gameplay_smoke.ps1
```

验收门槛：VUID=0、kills≥1、fps≥120、panics=0。

---

## 配置

配置文件：`~/.steel_front.cfg`（分辨率/键位/音量/灵敏度/画质，设置面板内即时生效并持久化）

环境变量（RV3D_* 前缀，共 20 个，按需查阅代码）：

| 变量 | 作用 |
|---|---|
| RV3D_PRESENT_MODE | immediate / mailbox / fifo 呈现模式 |
| RV3D_PROC_TEX | 0=回退 test.png（程序化纹理 A/B） |
| RV3D_SKIN_TEX | 1=启用 marker/NPC 皮肤纹理 |
| RV3D_NO_SHADOW | 1=关闭阴影 |
| RV3D_NPC_SCALE | NPC 数量缩放 |
| RV3D_STRESS_AI | 压力模式 AI 规模（默认 64） |
| RV3D_FORCE_SIMD | 强制 SIMD 选路（avx512/avx2/avx/sse4.2/scalar） |
| RV3D_CPU_PIN / RV3D_SCENE_WORKERS / RV3D_AI_WORKERS | 线程调度 |
| RV3D_AI_PARALLEL / RV3D_AI_DECIMATE | AI 并行/降频开关 |
| RV3D_BENCH_YAW / RV3D_BENCH_PITCH | 基准相机角 |
| RV3D_EXPLOSION_SIM | 爆炸模拟 |
| RV3D_NET / RV3D_NET_ADDR | 联机（server/client + 地址） |
| RV3D_MAP / RV3D_MAPS | 关卡系统 TOML |

---

## 测试与质量门槛

- `cargo test`：**384 个单元测试**（武器/物理/AI/地图/渲染/UI/网络），必须全绿
- `cargo build --release`：**0 警告**（dead-code=0 强制）
- 冒烟：VUID=0 / kills≥1 / fps≥120 / panics=0
- 提交规范：`feat/` `fix/` `docs/` 前缀，一次提交一个关注点

---

## 目录结构

```
src/
├── main.rs            # winit 事件循环 / 输入 / 渲染编排
├── ui.rs              # HUD/菜单/设置/中文字形渲染（GDI）
├── config.rs          # 配置持久化
└── engine/
    ├── renderer.rs    # Vulkan 渲染（实例场/LOD/HUD/阴影/mesh 路径）
    ├── meshgen.rs     # 程序化网格生成（圆角盒/锥台/球/环）
    ├── game.rs        # 游戏逻辑中枢（物理/武器/AI/波次）
    ├── weapons.rs     # 武器框架（Firearm/ProjectileWeapon/WeaponRack）
    ├── ai.rs          # 战术 AI（A*/状态机/协同）
    ├── camera.rs      # 三模式相机
    ├── physics.rs     # 玩家/刚体物理
    ├── map.rs         # 地图生成/TOML 关卡
    ├── audio.rs       # 程序化音效/音乐
    ├── font_cjk.rs    # Windows GDI 中文字形光栅化
    ├── procedural.rs  # 程序化纹理烘焙
    └── ...            # cpu/simd/lighting/net/objective 等
build.rs               # WGSL → SPIR-V 构建期编译（naga 30）
assets/                # 生成的 SPIR-V 与回退贴图
scripts/               # 冒烟/调试脚本
GAME_DESIGN.txt        # 玩法设计文档（唯一设计依据）
```

---

## 已知注意事项

- **naga 30 的 ADJUST_COORDINATE_SPACE 对多成员输出结构/网格写入器为死代码**：顶点/HUD/阴影路径的 Y 翻转行为以 build.rs 中 WGSL 显式处理为准（详见 build.rs 注释）
- **12GB RAM 内存限制**：开发环境一次只运行一个 cargo 进程
- **验证层**：release 构建无 Vulkan 验证层（性能），调试用 debug 构建
- **中文界面**：中文字形依赖 Windows GDI（font_cjk.rs）；非 Windows 平台回退 ASCII

---

## 路线图

- [ ] 设计文档定稿（弹药/护甲/爆头/爆炸/手雷/载具/兵种）
- [ ] 批1 战斗核心：护甲系统 + 爆头判定 + 受击减速/体力
- [ ] 批2 武器矩阵：口径参数框架 + 首批 4 把现代武器
- [ ] 批3 爆炸与手雷：三段衰减 + 进攻/防御型 + 破片/高爆
- [ ] 批4 细节：破片反弹、穿甲穿墙、地图主题现代化
- [ ] 批5 载具/兵种（文档补充后）
