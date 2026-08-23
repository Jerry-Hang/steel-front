# 钢铁前线 (Steel Front)

> 架空历史 · 2020 年代 · 大规模战场 FPS
> Rust + Vulkan 自研引擎（ash / winit / glam），零第三方游戏依赖

**一句话**：从零构建的现代大战场射击游戏引擎与玩法原型——程序化渲染、程序化建模、程序化音效、程序化地图，无 Unity/Unreal 黑盒，无外部资产（音频直接用 winmm `waveOut` 原生 FFI 发声，无 rodio）。

---

## 项目状态（2026-08-22）

| 维度 | 状态 |
|---|---|
| 单元测试 | **cargo test 全绿**（src 内 399 个 #[test]，dead-code=0） |
| 玩法冒烟 | ALL-OK（kills≥1、VUID=0、fps≥120、panics=0） |
| 渲染主路径 | **VK_EXT_mesh_shader 网格着色器**（RTX 5060 真机启用），无该扩展的显卡回退传统顶点管线 |
| 武器系统 | **35 把现代枪械已实现**（V3.0 数据表：初速/子弹下坠/散射MOA/距离衰减/部位倍率分段/开火模式/ADS），AK-12M 已重建 |
| 大战场 | 默认 **红 128 vs 蓝 127+玩家（128v128，256 人）**；RV3D_STRESS_AI=0 恢复波次模式 |
| 地图 | **手工绘制现代城市**（55m 街区网格）：写字楼/仓库/公园/商铺/哨卡/停车场/消防栓/路灯/围墙，替代随机种子生成 |
| 音频 | **waveOut 真实输出**（无设备时静默降级，混音/合成链路恒运行） |
| CPU 调度 | Windows 拓扑检测 + 亲和绑核（CCX/能效核分簇，逻辑/渲染同簇，AI/后台分簇） |
| 当前阶段 | **玩法迭代期**（射击手感/受击反馈/命中系统持续打磨中） |
| 设计文档 | [大战场枪械设计V3.0](./docs/大战场枪械设计V3.0.txt)（35 把枪全参数权威数据源） |

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
| 主渲染路径 | **VK_EXT_mesh_shader（网格着色器）**：GPU 逐实例视锥剔除 + 顶点变换，65536 实例地面场按 maxMeshWorkGroupCount 分块下发。新渲染功能一律走 mesh 路径 |
| 传统顶点管线 | **已冻结维护（仅回退）**：无 VK_EXT_mesh_shader 的显卡（WSLg/dzn 等）自动回退使用，不再新增功能/不再修复 |
| HUD | 独立屏幕空间管线（CPU 转 NDC），GDI 中文字形系统（Windows） |
| 阴影 | 2048×2048 D32 阴影图 + 3×3 PCF（**默认关闭**，待高楼城市全覆盖修复后另开） |

### 程序化生成（零外部资产）

- **建模**：meshgen.rs 程序化网格引擎——圆角盒（beveled box）/ 锥台 / 圆柱 / 球 / 圆环弧段，法线烘焙光照；枪械等模型由数学函数生成
- **纹理**：CPU 画像素——地面材质（城市分区 1024 纹理 + 烘焙 AO）、混凝土砌块表面纹理、皮肤纹理
- **地形**：257×257 顶点、三级 LOD + smoothstep morph、确定性值噪声
- **音效**：程序化 DSP 合成（枪声/爆炸/脚步/环境）
- **地图**：**手工绘制现代城市布局**（街区/建筑/遮挡/装饰），RV3D_PROC_MAP=1 可回退程序化生成用于 A/B

### 游戏性

- 第一人称移动/跳跃/射击/开镜（ADS FOV 补偿）/手雷/切枪
- 波次防守 + 关卡递进 + Boss/援军波
- 战术 AI：A* 寻路、状态机（巡逻/追击/攻击/掩体）、包抄/偷袭/协同，压力模式两军对抗
- **人形 AI 指挥体系**：三三制（营连排班）+ 逐级军情汇报 + AI 司令按前线态势下进攻/防御/侧翼/重组命令
- HUD：血条/弹药/准星/命中标记/击杀提示/小地图/ESC 菜单/设置面板（键位/分辨率/灵敏度/音量/画质）
- 联机基础：UDP client/server、快照同步（规划中扩展）

---

## 当前进度（2026-08-22 快照）

**已完成（本轮 2026-08-19 ~ 08-22）**
- **手绘现代城市地图**：55m 街区网格手工绘制（写字楼/仓库/公园/商铺/哨卡/停车场残骸/消防栓/路灯/围墙），替代随机种子生成；建筑为真正高楼（含半高/中心高/材质 tint），城市地面纹理 1024 + 蓝天清屏
- **障碍物立体化**：障碍物/NPC 实时 Blinn-Phong 光照（告别「纸片剪影」），混凝土砌块表面纹理（墙体 6 面立体），公园树冠按棵大小/高度错落
- **枪械检视模式**：白色虚空背景 --inspect=N / RV3D_INSPECT，围绕枪模 Orbit 相机拖拽/缩放；长焦产品级取景（修复极端透视畸变）、补护木导轨缝隙、紧凑深色枪口组件；AK-12M 重建
- **真实音频输出**：audio_out.rs 用 winmm waveOut 原生 FFI 环形缓冲发声，替换 SilentSink 占位
- **CPU 拓扑与亲和绑定**：Windows GetLogicalProcessorInformationEx 解析物理核/CCX(L3)/能效等级，SetThreadAffinityMask 亲和绑核（逻辑/渲染同 CCX、AI/后台分簇、音频/低延迟走能效核）
- **人形 AI 指挥体系**：三三制（营→连→排→班→战士），逐级军情汇报，AI 司令按前线推进度/兵力/伤亡每 0.5s 下进攻/防御/侧翼/重组命令，班长投掷压制指挥权加成
- **128v128 大战场压力模式**：默认红 128 vs 蓝 127+玩家（256 人），以海量 NPC 逼近真人联机压力；NPC 实例容量扩容至 2048 段/几何区

**进行中**
- 阴影图默认关闭（待高楼城市全覆盖修复）；设计文档整理期（弹药/护甲/爆头/手雷体系，见 GAME_DESIGN.txt）
- UI 美化与中文字形完善；性能日志增强（每次启动打包存档）

**规划**
- 护甲/体力/爆头机制落地；地图主题现代化（集装箱/路障/废弃车辆）
- 载具与支援系统（轻载具/空袭/无人机，远期）

---

## 操作说明

### 键位

| 按键 | 功能 |
|---|---|
| W / A / S / D | 前后左右移动 |
| 鼠标左键 | 射击（Playing）；非第一人称窗口拖拽旋转 |
| 鼠标右键 | 开镜（ADS，第一人称）；飞行模式拖拽转视角 |
| Tab | 相机循环（Orbit → Flight → FirstPerson）；设置面板循环选中项 |
| 数字键 1/2 或 / 命令窗口 | 切换武器 |
| G | 投掷手榴弹 |
| R | 换弹 |
| Enter | 死亡/胜利/失败结算后重开本关 |
| ESC | 打开/关闭菜单 |
| Q / E | 升降（飞行/轨道模式） |
| B | 切换开火模式（单发/三连发/连发） |
| N | 补给弹药（设置面板）/ 胜利后进入下一关 |
| F5 | 关卡 TOML 热重载 |

### 启动参数与环境变量

- --inspect=N 或 RV3D_INSPECT=N：进入 N 号武器检视模式（白色虚空背景，拖拽旋转/滚轮缩放）
- RV3D_STRESS_AI=N：大战场两侧 NPC 规模（默认 128 = 128v128，≥4 生效；0/off 恢复传统波次模式）
- RV3D_AUTOSTART=1：测试用自动开局（绕过菜单直接进 Playing，复现/冒烟）
- RV3D_CAM=fly:x,y,z:yaw,pitch：调试固定机位（飞行模式，地图/场景检查用）
- RV3D_SKIN_TEX：障碍/NPC 皮肤纹理（默认开启，0 关闭回退纯色）
- RV3D_NO_SHADOW=1：关闭阴影
- RV3D_CPU_PIN：覆盖精确亲和性掩码（如 0-7,16-23；off 关闭绑核）
- 其余见下方 [配置](#配置) 表。

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
| 4K 高画质 | 13700K 或 7900X | RTX 4070 Super 或 RX 9070 GRE | 32GB+ |

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

- Windows：直接运行 target/release/steel-front.exe，或使用仓库内启动器（先杀残留进程 → 自动拉取更新 → 构建 → 启动）
- Linux：cargo run --release
- **发布包**：release_dist/（SteelFrontLauncher.exe + game/，含桌面快捷方式），由 scripts/make_release.ps1 生成

### 冒烟测试

```bash
powershell -ExecutionPolicy Bypass -File scripts/run_gameplay_smoke.ps1
```

验收门槛：VUID=0、kills≥1、fps≥120、panics=0。

---

## 配置

配置文件：~/.steel_front.cfg（分辨率/键位/音量/灵敏度/画质，设置面板内即时生效并持久化）

环境变量（RV3D_* 前缀，共 20 个，按需查阅代码）：

| 变量 | 作用 |
|---|---|
| RV3D_PRESENT_MODE | immediate / mailbox / fifo 呈现模式 |
| RV3D_PROC_TEX | 0=回退 test.png（程序化纹理 A/B） |
| RV3D_PROC_MAP | 1=回退程序化地图生成（城市手绘布局 A/B） |
| RV3D_SKIN_TEX | 0=关闭障碍/NPC 皮肤纹理（默认开启） |
| RV3D_NO_SHADOW | 1=关闭阴影 |
| RV3D_NPC_SCALE | NPC 数量缩放 |
| RV3D_STRESS_AI | 压力模式 AI 规模（默认 128 = 128v128，0/off=波次） |
| RV3D_FORCE_SIMD | 强制 SIMD 选路（avx512/avx2/avx/sse4.2/scalar） |
| RV3D_CPU_PIN / RV3D_SCENE_WORKERS / RV3D_AI_WORKERS | 线程调度/亲和绑核 |
| RV3D_AI_PARALLEL / RV3D_AI_DECIMATE | AI 并行/降频开关 |
| RV3D_BENCH_YAW / RV3D_BENCH_PITCH | 基准相机角 |
| RV3D_EXPLOSION_SIM | 爆炸模拟 |
| RV3D_NET / RV3D_NET_ADDR | 联机（server/client + 地址） |
| RV3D_MAP / RV3D_MAPS | 关卡系统 TOML |
| RV3D_INSPECT | 枪械检视模式（武器编号 1-35） |
| RV3D_AUTOSTART / RV3D_SWITCH_WEAPON / RV3D_SWITCH_WEAPON_AFTER | 测试自动开局/延迟切枪 |
| RV3D_CAM | 调试固定机位（fly:x,y,z:yaw,pitch） |

---

## 测试与质量门槛

- cargo test：**399 个单元测试**（武器/物理/AI/地图/渲染/UI/网络），必须全绿
- cargo build --release：**0 警告**（dead-code=0 强制）
- 冒烟：VUID=0 / kills≥1 / fps≥120 / panics=0
- 提交规范：feat/ fix/ docs/ 前缀，一次提交一个关注点

---

## 技术要点

- **程序化几何与纹理**（meshgen / procedural）：图元组装建模 + CPU 烘焙光照/分区纹理，零外部资产
- **网格着色器路径**（VK_EXT_mesh_shader + 传统回退）：GPU 逐实例剔除/变换为主路径，无扩展显卡自动回退冻结的顶点管线
- **地形 LOD**：三级 LOD + smoothstep morph + 确定值噪声，远距降密度
- **方向光/点光/阴影**：DirectionalLight（方向/颜色/强度）+ PointLight（衰减）+ 2048 D32 阴影图 3×3 PCF（默认关）
- **音频程序化合成**：DSP 事件式合成（枪声/爆炸/脚步/环境），ADSR + 低通 + 多声部混音；waveOut 后端发声
- **CPU 拓扑与线程亲和**：Windows 物理核/CCX(L3)/能效等级检测 + SetThreadAffinityMask 绑核
- **AI 分层调度**（scene_pool / ai_pool）：近组/交互 AI 走 scene_pool（P 核/CCD0），远组/后台走 ai_pool（E 核/CCD1）

---

## 目录结构

```
src/
├── main.rs            # winit 事件循环 / 输入 / 渲染编排 / 检视模式
├── ui.rs              # HUD/菜单/设置/中文字形渲染（GDI）
├── audio.rs           # 程序化音频合成/混音（AudioSink 抽象）
├── audio_out.rs       # Windows waveOut 原生 FFI 真实发声后端
├── config.rs          # 配置持久化
├── net.rs             # UDP client/server 快照同步
└── engine/
    ├── renderer.rs    # Vulkan 渲染（实例场/LOD/HUD/阴影/mesh+传统路径）
    ├── meshgen.rs     # 程序化网格生成（圆角盒/锥台/球/环）
    ├── game.rs        # 游戏逻辑中枢（物理/武器/AI/波次/压力模式）
    ├── weapons.rs     # 武器框架（Firearm/ProjectileWeapon/WeaponRack）
    ├── weapon_data.rs # 35 把枪 V3.0 数据表（ALL_WEAPONS）
    ├── guns/          # 各枪程序化建模纯函数
    ├── ai.rs          # 战术 AI（A*/状态机/协同）
    ├── ai_command.rs  # 连排班指挥体系（三三制/军情汇报/司令决策）
    ├── camera.rs      # 三模式相机（第一人称/轨道/飞行）
    ├── physics.rs     # 玩家/刚体物理
    ├── map.rs         # 地图生成/TOML 关卡
    ├── city.rs        # 手工绘制现代城市布局
    ├── procedural.rs  # 程序化纹理烘焙（城市地面/皮肤）
    ├── cpu.rs         # CPU 拓扑检测与线程亲和（Windows FFI）
    ├── lighting.rs    # 方向光/点光/阴影 Uniform
    ├── font_cjk.rs    # Windows GDI 中文字形光栅化
    ├── simd.rs        # SIMD 选路与加速比
    ├── gpu_caps.rs    # 显卡能力枚举
    └── ...            # objective/window/cjk_glyphs 等
launcher/              # 零依赖 Win32 GUI 启动器（更新/快捷方式/壁纸）
build.rs               # WGSL → SPIR-V 构建期编译（naga 30）
assets/                # 生成的 SPIR-V 与回退贴图、地图 TOML
scripts/               # 冒烟/发布/调试脚本
GAME_DESIGN.txt        # 玩法设计文档（唯一设计依据）
```

---

## 已知注意事项

- **naga 30 的 ADJUST_COORDINATE_SPACE 对多成员输出结构/网格写入器为死代码**：顶点/HUD/阴影路径的 Y 翻转行为以 build.rs 中 WGSL 显式处理为准（详见 build.rs 注释）
- **阴影图默认关闭**：待高层建筑城市全覆盖阴影修复后再开（当前关 shadows 以保帧率/正确性）
- **12GB RAM 内存限制**：开发环境一次只运行一个 cargo 进程
- **验证层**：release 构建无 Vulkan 验证层（性能），调试用 debug 构建
- **中文界面**：中文字形依赖 Windows GDI（font_cjk.rs）；非 Windows 平台回退 ASCII
- **工作区未提交内容**：audio_out.rs、ai_command.rs、128v128 默认值与 NPC 容量扩容目前仍在工作区（未 commit），交付前请确认

---

## 路线图

- [ ] 设计文档定稿（弹药/护甲/爆头/爆炸/手雷/载具/兵种）
- [ ] 批1 战斗核心：护甲系统 + 爆头判定 + 受击减速/体力
- [ ] 批2 武器矩阵：口径参数框架 + 后续现代武器
- [ ] 批3 爆炸与手雷：三段衰减 + 进攻/防御型 + 破片/高爆
- [ ] 批4 细节：破片反弹、穿甲穿墙、地图主题现代化
- [ ] 批5 载具/兵种（文档补充后）
