# 钢铁前线 · Steel Front

> 架空历史 · 2020 年代 · 大规模战场 FPS ｜ Rust + Vulkan 自研引擎（ash / winit / glam）｜ 零第三方游戏引擎依赖

**📖 中文（当前）** · [English](#steel-front--english)

---

## 重大功能上线时间

| 时间 | 上线内容 |
|---|---|
| 2026-08-09 | Windows 原生 Vulkan 1.3 落地；CPU 拓扑检测与线程亲和绑定；爆炸/冲击波 SIMD 加速；GPU 能力枚举（启动打印 `gpu-caps:`）；程序化世界空间地面材质纹理（分区着色 + 烘焙 AO + 静态天光） |
| 2026-08-11 | AI 线程分层调度（scene_pool / ai_pool 双池，按 CCX 与能效核自动绑定，远组降频） |
| 2026-08-12 | 阴影贴图（方向光 2048² + 点光立方体 6 面）、烘焙高度场 AO、静态光照烘焙进顶点色 |
| 2026-08-13 | 障碍物 / NPC 程序化皮肤纹理（混凝土墙、迷彩军服） |
| 2026-08-16 | **确立 `VK_EXT_mesh_shader` 网格着色器为主渲染路径**（GPU 逐实例剔除 + 顶点生成），传统顶点管线定为回退 |
| 2026-08-22 | 手工绘制的现代城市布局（55m 街区网格）取代随机地图；三三制连排班指挥体系；火-机动交替；大战场压力模式扩至 128v128；winmm `waveOut` 原生 FFI 真实发声 |
| 2026-08-27 | 第一人称枪模高模路线；自发光实例槽位区间修复（枪槽不再被误判为火光） |
| 2026-08-28 | **外部资产导入管线**：OBJ / glTF GLB 零依赖解析器、Windows GDI+ PNG/JPEG 解码、Blender 无头批处理闭环；AK-12 GLB（63,283 顶点）实装；联机 NAT 中继 `rdv.exe`（房间名直连 + 打洞） |
| 2026-08-31 | 建模系统重构：形状标签进 `tint.w`（不改顶点 stride）、marker 预算扩至 8192、圆柱/二十面体/球体模板、零共面三原则写入 `city.rs` |
| 2026-09-01 | 城市外围街区肌理与 8192 marker 预算落地；AI 并行第二波；据点标识与士兵可辨识度修正 |
| 2026-09-03 | 路径追踪管线打通（`VK_KHR_ray_query` 全景 PT + NEE + 时域累积降噪）；**建模路线改为 Blender 资产化** |
| 2026-09-04 | **地面大面积纯黑根治**（未绑定描述符乘零）；**GLB 道具首次上屏**；主管线开启深度遮挡（枪模拆独立管线）；修复实例 buffer 三份抄写导致的静默越界读；GLB 加载器四个错读修复 |
| 2026-09-05 | **网格着色器恢复为主路径**（顶点管线冻结）；道具空间分桶 + 逐桶视锥剔除（fps 112→152）；13 把外部枪械模型规范化并接入 |

---

## 当前进度（截至 2026-09-01）

### 已完成

**引擎与渲染**
- 纯 Rust + Vulkan 1.3，无 Unity/Unreal 黑盒；`ash 0.38` 全量 1.3 头，实例与设备均 1.3。
- 网格着色器主路径（MESH + FRAGMENT），GPU 侧逐实例视锥测试与顶点生成，一个 workgroup 对应一个实例槽位，视锥测试在着色器内完成。
- 地形三级 LOD 网格（257² / 129² / 65²）+ 帧间形态过渡（消除切换跳变）；65536 实例地面场；MSAA 4×；各向异性过滤与完整 mip 链。
- 阴影贴图（方向光 + 点光立方体）、烘焙高度场 AO、静态光照烘焙、程序化皮肤纹理。
- HUD 覆盖层自包含（独立管线与顶点缓冲，不侵入主渲染）；Windows GDI 中文字形光栅化。

**玩法与系统**
- 35 把现代枪械，V3.0 数据表驱动（初速、下坠、散射、距离衰减、部位伤害倍率、开火模式、ADS 参数）。
- 大战场：默认红 128 vs 蓝 127+玩家；波次模式与压力模式可切换。
- AI：三三制编制（营→连→排→班）、火-机动交替、连级目标横向铺开、掩体点选择、LLM 战时指挥官（llama.cpp，零依赖）。
- 联机：服务器权威 + 快照插值 + 断线重连 + 协议版本握手 + NAT 中继。
- 关卡数据化：TOML 地图描述（出生点/目标/障碍/规则），F5 热重载，多关卡索引。

**外部资产管线**
- OBJ 与 glTF GLB 零依赖解析器（多 mesh 合并、componentType 感知 accessor、COLOR_0 顶点色、材质基色）。
- Blender 无头控制闭环：导入 → 材质/AO 烘焙 → 节点净化 → 导出 GLB → 渲染 PNG → 看图自检。
- AK-12 GLB 模型实装，第一/三人称与开火全链路验证。

### 优化方面待完成

- **路径追踪启动崩溃**：`0xC0000005`，当前 `pt_enable=false` 停用；依赖版本与驱动状态两项假设已被证据排除。
- **地面场剔除**：网格路径将 65536 个地面 workgroup 静态全量上传、不做 CPU 剔除，是当前帧率天花板。
- **道具剔除粒度**：分桶边长为固定值，未按街区密度自适应；远景道具尚无 LOD 分级。
- **存量编译警告**：约 50 条待专项清理（本项目不使用 `#[allow(dead_code)]` 掩盖警告）。

### 改进方面待完成

- **碰撞盒与视觉体尺寸校准**：为保证"GLB 不小于碰撞盒"（避免无形墙）而取的等比缩放，代价是玩家可能站进楼体。
- **PBR 贴图采样**：金属度/环境反射；GLB 嵌入贴图（`images` / `bufferViews.byteStride`）解析。
- **建筑变体覆盖**：部分变体尚未被选取，街道重复度仍可降低。
- **第三方枪械素材清理**：一张狙击枪源文件含两把重叠枪身，需人工删重后接入。
- **音频**：仅单声道输出；缺乏遮挡/距离衰减的声学模型。

---

## 配置要求

| 场景 | 推荐处理器 | 推荐显卡 | 内存 |
|---|---|---|---|
| **1080P 非光追** | Ryzen 3 3300X ／ Intel 11 代 i3 | RX 6500 XT ／ RTX 3050 6GB | **最低 8 GB** |
| **1080P 光追** | i5-11400F ／ Ryzen 7 3700X | RTX 2070 Super ／ Arc A750 | 推荐 12 GB |
| **2K 非光追** | i5-12490F ／ Ryzen 5 5600X | RTX 3060 Ti ／ RX 6700 XT | 推荐 12 GB |
| **2K 光追** | i7-12700K ／ Ryzen 7 7700X | RTX 5060 ／ RX 9060 XT | 推荐 16 GB |
| **4K 低画质** | i7-13700K ／ Ryzen 9 7900X | RTX 3080 Ti ／ RX 7900 GRE | 推荐 24 GB |
| **4K 高画质** | 270K Plus ／ Ryzen 9 9950X | RTX 4080 Super ／ RX 9070 XT | 推荐 32 GB |

### 硬件门槛说明

- **CPU 是硬门槛，不是显卡。** 128v128 场景下每帧要并行推进 256 名士兵的寻路、视线检测与弹道模拟，外加物理、音频合成与网络快照。**核心数量比单核频率更重要**——引擎按 CCX（AMD 双 CCD）与 P/E 核（Intel 混合架构）做分层绑定，6 核以下会明显掉帧。
- **内存**：8 GB 是可运行下限，此时须避免同时进行编译（见下方本地部署）。大战场 + 完整资产加载建议 12 GB 起。
- **显存**：非光追路径 2 GB 即可；开启路径追踪参照视图时因需额外分配 RGBA32F 累积缓冲与加速结构，建议 6 GB 起。
- **笔记本平台**：建议独显直连（MUX 独显模式）。混合输出下 IMMEDIATE 呈现模式最稳定。

### 图形 API 要求

- **Vulkan 1.3**（必需）。实例与设备均按 1.3 创建。
- **`VK_EXT_mesh_shader`**：主渲染路径所需。缺失时自动回退传统顶点管线，功能不受影响但帧率下降。
- **`VK_KHR_acceleration_structure` + `VK_KHR_ray_query`**：仅路径追踪参照视图所需，可完全缺失不影响主流程。
- **`VK_KHR_swapchain`**（必需）；`VK_KHR_dynamic_rendering` 为后续升级方向（已入 1.3 核心）。
- 启动时会完整打印 `gpu-caps:` 与 `RT 管线属性` 日志，用于确认实际拿到的扩展与限制。

---

## 操作说明

| 按键 | 功能 |
|---|---|
| W / A / S / D | 移动 |
| 鼠标左键 | 射击（玩法状态）；非第一人称窗口下拖拽旋转视角 |
| 鼠标右键 | 开镜（ADS） |
| Tab | 相机模式循环（第一人称 → 轨道 → 飞行） |
| 1 / 2 或 `/` 命令窗口 | 切换武器 |
| B | 切换开火模式（单发 / 三连发 / 连发） |
| R | 进入操控；死亡结算后按 R 复活；游戏中为换弹 |
| G | 投掷手榴弹 |
| N | 补给 / 进入下一关 |
| Q / E | 升降（飞行与轨道模式） |
| F5 | 关卡 TOML 热重载 |
| Enter | 结算界面重开 |
| ESC | 菜单与设置 |

> **鼠标捕获说明**：进入玩法后引擎会**自行抓取光标**（无需点击窗口），抓取期间系统光标被锁定。进行自动化截图测试时务必使用带 `finally` 强制结束进程的封装脚本（见 `scripts/cap_safe.ps1`），不要直接裸启动游戏进程。

---

## 联机说明与指导

### 模式

- **同机双人**：一台机器开两个进程，走 `127.0.0.1`。
- **局域网双人**：加入方填主机的局域网 IP。
- **异地双人**：经由中继服务 `rdv.exe` 做地址发现与打洞。

### 步骤

```powershell
# ── 主机 ──
$env:RV3D_NET = 'server'
$env:RV3D_NET_ADDR = '0.0.0.0:27015'      # 监听全部网卡
start target\release\steel-front.exe

# ── 加入方 ──
$env:RV3D_NET = 'client'
$env:RV3D_NET_ADDR = '主机IP:27015'        # 同机用 127.0.0.1
start target\release\steel-front.exe
```

异地联机额外需要中继，双方使用同一房间名：

```powershell
# 先在一台公网可达的机器上跑中继
rdv.exe 0.0.0.0:27020

# 主机与加入方都追加以下两项（房间名必须一致）
$env:RV3D_NET_RDV  = '中继地址:27020'
$env:RV3D_NET_NAME = 'myroom'
```

### 机制与注意事项

- **服务器权威**：伤害判定、命中与胜负全部由主机结算，客户端只做快照插值与预测外推，因此不存在"客户端改数值"的作弊路径。
- **协议版本握手**：加入时携带会话版本号，不匹配则直接拒绝并停止重试，避免新旧客户端混连产生不可名状的同步错误。
- **断线重连**：客户端检测到快照超时后进入重连等待，主机恢复后自动续接。
- **枪口焰同步**：快照内含"正在开火"指示位，加入方能看到主机方的枪口焰。
- 发布包内已附 `联机主机.bat` / `联机加入.bat`，双击即用，无需手动设置环境变量。
- 端口默认 `27015`（游戏）与 `27020`（中继）；跨网络时需在路由器放行，或依赖中继打洞。

---

## Vulkan 图形 API 特性说明

- 按 **Vulkan 1.3** 编写与运行，`ash` 全量 1.3 绑定，实例与设备版本均为 1.3。
- **`VK_EXT_mesh_shader` 主路径**：网格着色器同时承担剔除与顶点生成，一个 workgroup 对应一个实例槽位，视锥测试在 GPU 上完成，避免 CPU 每帧回读与重传。地面实例场超过规格下限的 `maxMeshWorkGroupCount[0]`，因此按设备实际上限分块派发。
- **传统顶点管线**：仅作为缺 `VK_EXT_mesh_shader` 时（WSLg、dzn 软件驱动、老显卡）的兼容回退，**已冻结**，不再接受功能开发。
- 采用经典 `vkRenderPass`（1.0 核心子集，在 1.3 设备上合法）。后续升级方向为 dynamic rendering。
- 同步模型：每帧一份命令缓冲 + 帧信号量 + `max_frames_in_flight` 份 uniform/实例缓冲，避免读写竞态。
- 所有几何缓冲为 `HOST_VISIBLE | HOST_COHERENT` 持久映射；纹理走 staging buffer 上传。
- 实例数据以 80 字节 `InstanceData`（4×4 模型矩阵 + RGBA tint）存入 storage buffer，形状标签借用 `tint.w` 传递，因此**新增形状不改变顶点 stride**。

## 画面特性说明

- **地形**：三级 LOD 网格 + 帧间形态过渡（消除切换跳变）；高度场为纯函数，CPU 与 GPU 同源，玩法与渲染读到的地形完全一致。
- **光照**：方向光 + 点光，Blinn-Phong 模型；百分比渐近透明（PCF）软阴影；烘焙高度场 AO 与静态天光。
- **纹理**：世界空间烘焙地面材质图（沥青/方砖/草地/沙地分区，2 texel/米）+ 近景微细节平铺层（约 128 texel/米，双 mip 层级衔接）；障碍物与士兵各有程序化皮肤纹理。
- **大气**：距离雾，雾色与天空色同源，保证远处"楼—天"交界连续。
- **自发光**：爆炸、枪口焰、烟雾走独立实例区间，靠视线相关径向衰减伪造体积光晕（不依赖 alpha 混合，主 pass 保持全不透明）。
- **MSAA 4×** + 各向异性过滤 + 完整 mip 链。
- **路径追踪参照视图**（当前停用）：`VK_KHR_ray_query` 全景 PT，含太阳阴影射线 NEE 与漫反射弹跳，spp 时域累积降噪，收敛后可作为光照烘焙真值。

### 做资产前必须知道的着色约束

- **法线不会上传到 GPU**：顶点格式为 `pos(3) + color(3) + uv(2)` 共 32 字节，着色法线全部由屏幕空间导数重建 ⇒ **只能纯平着色**。平滑着色的高模会棱面毕现；AO 与烘焙光照必须写进**顶点色**。
- 由此推论：**绕序错误的面会直接变黑而不报任何错误**。外部建模的网格在进入引擎时统一做一次索引交换。
- 材质身份目前由顶点色的通道比例嗅探判定（玻璃、树冠等）。外部建模资产通过形状标签退出这套程序化立面加工，否则引擎会在建模好的立面上再画一层错位的程序化窗带。

---

## 项目结构

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

## 文档索引与指导

| 文档 | 用途 | 什么时候该读 |
|---|---|---|
| [GAME_DESIGN.txt](./GAME_DESIGN.txt) | **玩法设计的唯一依据** | 任何涉及数值、机制、关卡的判断之前 |
| [AGENTS.md](./AGENTS.md) | AI 交接日志与迭代留痕（本项目唯一的正式交接载体） | 接手开发前必读；每次迭代结束必须追加 |
| [大战场枪械设计V3.0](./docs/大战场枪械设计V3.0.txt) | 35 把枪的完整数据表与设计依据 | 改武器数值或新增枪械时 |
| [LICENSE](./LICENSE) | 许可与商业授权条款 | 再分发或商用前 |
| `docs/experiment-*` | 渲染、光照、性能的验证记录与实测数据 | 怀疑某个"已知结论"是否仍然成立时 |
| `docs/perf-*` | 帧率与瓶颈基准存档 | 做性能优化前后对比 |
| `docs/HANDOFF-*` | 历史交接快照 | 追溯某个决定是在哪一轮做出的 |

**阅读建议**：先读 `GAME_DESIGN.txt` 建立玩法认知 → 再读 `AGENTS.md` 顶部的项目概览与两条铁律 → 需要改渲染时读 `docs/` 下对应的验证文档，而不是直接改代码。`AGENTS.md` 里的「渲染技术路线铁律」与「验证纪律」是本项目最容易踩的两个坑。

---

## 本地部署方法

### 前置条件

| 项目 | 要求 |
|---|---|
| 操作系统 | Windows 10 / 11（x86_64）。Linux 走 WSLg 时会自动回退传统顶点管线 |
| Rust 工具链 | 稳定版 `rustc` / `cargo`（需支持 2024 edition） |
| 显卡驱动 | Vulkan 1.3 兼容驱动（NVIDIA 512.xx+ / AMD 22.5+ / Intel 最新） |
| 磁盘 | 约 2 GB（含 `target/` 构建产物） |
| 内存 | 建议 12 GB 起；**12 GB 及以下时同一时刻只能跑一个 `cargo`**，并行构建会导致内存耗尽式挂起 |

### 依赖说明

**运行时依赖全部为系统自带组件，不下载任何第三方库：**

- `ash` / `winit` / `glam` —— Rust crate，构建时由 cargo 拉取。
- **Vulkan 加载器**（`vulkan-1.dll`）—— 由显卡驱动提供。
- `winmm`（`waveOut` 音频）、`gdi32` / `user32`（中文字形光栅化、窗口与线程亲和）—— Windows 系统自带，通过 FFI 直接调用。
- 无 C/C++/C# 运行时，无 Python 运行时依赖，无外部 DLL。

**构建期依赖：**

- `naga`（仅 build-dependencies，用于 WGSL → SPIR-V）与 `spirv-tools`（校验）。
- 可选：`glslangValidator`（仅路径追踪 GLSL 需要，见 `scripts/compile_pt.ps1`）。
- 可选：Blender（仅重新生成 3D 资产时需要，headless 调用，不需要 GUI）。

### 部署步骤

```powershell
# 1) 克隆
git clone <仓库地址> steel-front
cd steel-front

# 2) 构建（首次为全量编译，耗时较长）
cargo build --release

# 3) 运行：工作目录必须是仓库根！
#    assets/*.spv 与 assets/props/ 按进程 cwd 相对路径加载，
#    从别处启动会报「打开着色器文件失败 (os error 3)」
$env:RV3D_AUTOSTART = '1'
.\target\release\steel-front.exe
```

### 验证与自检

```powershell
# 单元测试（纯逻辑，不触碰 GPU）
cargo test --release

# 冒烟测试：启动、采图、检查 VUID 与 device-lost 后自动结束进程
powershell -ExecutionPolicy Bypass -File scripts\run_gameplay_smoke.ps1

# 资产体检：判断一批 GLB 能否被本引擎直接加载
python tools\glb_survey.py "路径\到\模型目录"
```

### 接入外部 3D 资产

```powershell
# 枪械：体检 → 规范化（应用变换 / 合并单网格 / 贴图烘进顶点色 / 减面 / 密集缓冲导出）→ 按武器 key 安装
& "blender.exe" --background --factory-startup --python tools\blender\prep_guns.py -- --in "D:\我的模型"
python tools\install_guns.py

# 世界道具：重新生成 assets/props/ 并出顶点色预览图自查
& "blender.exe" --background --python tools\blender\gen_props.py
& "blender.exe" --background --python tools\blender\gen_props.py -- --preview screenshots/kit.png
```

### 发布打包

```powershell
powershell -ExecutionPolicy Bypass -File scripts\publish.ps1
```

产物在 `release_dist/`，含启动器、游戏本体与 `联机主机.bat` / `联机加入.bat`。
⚠ 打包时务必确认 `assets/guns/` 子目录与 `assets/rt/` 一并随包发布——缺失不会报错，只会静默回退到程序化枪模。

---

## 许可证

本项目采用 **AGPL-3.0 + 附加商业使用条款**。完整条款见 [LICENSE](./LICENSE)。

```
Copyright (c) 2026 黄少杰

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/agpl-3.0.html>.
```

### 附加商业使用条款

以下条款是对上述 AGPL-3.0 许可的补充，旨在明确商业使用场景下的授权条件。

**1. 开源使用（永远免费）**
只要您在使用本软件或其衍生作品时，**完全遵守 AGPL-3.0 协议**（即保持源代码公开，并且所有修改也以 AGPL-3.0 发布），则无论您的商业规模或收入多少，您都**无需支付任何授权费用**。

**2. 闭源使用豁免（小规模免费）**
如果您选择**不遵守 AGPL-3.0 的开源要求**（即进行闭源分发或提供闭源的网络服务），但您（或您所代表的实体）的**最近一个完整季度的总营业收入**低于**人民币 1000 万元**（或等值外币），则您**自动获得闭源使用的免费授权**，无需支付任何费用，也无需另行联系。

**3. 闭源使用收费（大规模需购买）**
如果您选择闭源使用，且您（或您所代表的实体）的**最近一个完整季度的总营业收入**达到或超过**人民币 1000 万元**，则您**必须联系版权所有者**并获得单独的书面商业授权许可，否则您的使用行为将被视为侵权，并需承担相应法律责任。

**4. 重要定义与说明**
- "总营业收入"指您（或您的法律实体）在最近一个完整季度内从所有业务活动中获取的总收入，不限于本软件的直接使用收益。
- 如果您同时运营多个项目或产品，营业收入应合并计算。
- 本附加条款不构成对 AGPL-3.0 的修改，而是对使用本软件施加的额外授权条件。如有任何冲突，以本附加条款为准，但仅限涉及商业授权部分。

### 商业授权联系方式

如需咨询商业闭源授权或获得书面许可，请联系：

- 邮箱：**huangshaojie925@gmail.com**

### 关于第三方素材的授权范围

上述许可仅覆盖本仓库中由版权所有者持有的原创代码与文档。`assets/guns/` 与 `assets/guns_ext/` 中由第三方站点下载的枪械模型**不适用本许可**，其使用受各自来源条款约束；引入前须逐个确认其许可证是否允许再分发与商用。`assets/props/` 由本项目 headless Blender 脚本原创生成，适用上述许可。

---
---

# Steel Front — English

**Alternate-history 2020s large-scale battlefield FPS** ｜ Self-built Rust + Vulkan engine (ash / winit / glam) ｜ Zero third-party game-engine dependencies

**[中文](#钢铁前线--steel-front)** · **English**

---

## Major Feature Milestones

| Date | Shipped |
|---|---|
| 2026-08-09 | Native Windows Vulkan 1.3 brought up; CPU topology detection and thread-affinity pinning; SIMD-accelerated blast/shockwave; GPU capability enumeration (`gpu-caps:` at startup); procedural world-space ground material texture (zone tinting + baked AO + static sky light) |
| 2026-08-11 | Tiered AI thread scheduling (scene_pool / ai_pool dual pools, auto-pinned to CCX and efficiency cores, far-group decimation) |
| 2026-08-12 | Shadow mapping (directional 2048² + point-light cubemap, 6 faces), baked height-field AO, static light baking into vertex colour |
| 2026-08-13 | Procedural skin textures for obstacles and soldiers (concrete wall, camouflage uniform) |
| 2026-08-16 | **`VK_EXT_mesh_shader` established as the main render path** (GPU per-instance culling + vertex generation); classic vertex pipeline demoted to fallback |
| 2026-08-22 | Hand-authored modern city layout (55 m block grid) replacing random generation; section-level 3×3 command hierarchy; fire-and-maneuver tactics; large-battlefield pressure mode extended to 128v128; real audio output via winmm `waveOut` native FFI |
| 2026-08-27 | High-poly first-person weapon route; emissive instance-slot range fix (the gun slot is no longer misread as muzzle flash) |
| 2026-08-28 | **External asset import pipeline**: zero-dependency OBJ / glTF GLB parsers, Windows GDI+ PNG/JPEG decode, closed-loop headless Blender batch processing; AK-12 GLB (63,283 verts) shipped; multiplayer NAT rendezvous `rdv.exe` (room-name direct connect + hole punching) |
| 2026-08-31 | Modelling system refactor: shape tags moved into `tint.w` (vertex stride unchanged), marker budget raised to 8192, cylinder / icosahedron / sphere templates, zero-coplanarity rules written into `city.rs` |
| 2026-09-01 | Perimeter urban fabric on the 8192-marker budget landed; second wave of parallel AI; capture-point and soldier readability fixes |
| 2026-09-03 | Path-tracing pipeline brought up (`VK_KHR_ray_query` panoramic PT + NEE + temporal accumulation denoise); **modelling route switched to Blender-authored assets** |
| 2026-09-04 | **Large-area black ground fixed** (an unbound descriptor multiplying albedo to zero); **GLB props on screen for the first time**; depth testing enabled on the main pipeline (weapon moved to its own pipeline); silent out-of-bounds read caused by three copied instance-buffer sizes repaired; four GLB loader misreads fixed |
| 2026-09-05 | **Mesh shader restored as the main path** (vertex pipeline frozen); spatial binning + per-bin frustum culling for props (fps 112→152); 13 external weapon models normalised and wired in |

---

## Current Progress (as of 2026-09-01)

### Completed

**Engine and rendering**
- Pure Rust + Vulkan 1.3, no Unity/Unreal black box; `ash 0.38` full 1.3 headers, instance and device both at 1.3.
- Mesh-shader main path (MESH + FRAGMENT): culling and vertex generation on the GPU, one workgroup per instance slot, frustum test performed inside the shader.
- Three-level terrain LOD meshes (257² / 129² / 65²) with inter-frame morphing (removes switch popping); 65536-instance ground field; MSAA 4×; anisotropic filtering with full mip chains.
- Shadow mapping (directional + point-light cubemap), baked height-field AO, static light baking, procedural skin textures.
- Self-contained HUD overlay (own pipeline and vertex buffer, does not intrude on the main render pass); Windows GDI CJK glyph rasterisation.

**Gameplay and systems**
- 35 modern firearms driven by the V3.0 data table (muzzle velocity, bullet drop, spread, distance falloff, per-body-part damage multipliers, fire modes, ADS parameters).
- Large battlefield: red 128 vs blue 127 + player by default; wave mode and pressure mode switchable.
- AI: 3×3 hierarchy (battalion → company → platoon → section), fire-and-maneuver, company-level lateral objective spread, cover-point selection, optional LLM battlefield commander (llama.cpp, zero-dependency).
- Multiplayer: server-authoritative + snapshot interpolation + reconnect + protocol version handshake + NAT rendezvous.
- Data-driven levels: TOML map descriptions (spawns / objectives / obstacles / rules), F5 hot reload, multi-level index.

**External asset pipeline**
- Zero-dependency OBJ and glTF GLB parsers (multi-mesh merge, componentType-aware accessors, COLOR_0 vertex colour, material base colour).
- Headless Blender control loop: import → material/AO bake → node cleanup → GLB export → render to PNG → visual self-check.
- AK-12 GLB model shipped, verified end-to-end across first person, third person and firing.

### Optimisation Backlog

- **Path-tracing startup crash**: `0xC0000005`, currently disabled via `pt_enable=false`; both the dependency-version and driver-state hypotheses have been disproved by evidence.
- **Ground field culling**: the mesh path statically uploads all 65536 ground workgroups with no CPU culling — the remaining frame-rate ceiling.
- **Prop culling granularity**: the bin edge length is a fixed constant, not adaptive to block density; no LOD tiering for distant props yet.
- **Standing compiler warnings**: ~50 to be cleared in a dedicated pass. This project does not use `#[allow(dead_code)]` to hide warnings.

### Improvement Backlog

- **Collision vs visual size calibration**: the uniform scale chosen so that "GLB ≥ collision box" (avoiding invisible walls) costs the possibility of the player standing inside a building's visual volume.
- **PBR texture sampling**: metallic / environment reflection; parsing of embedded GLB textures (`images`, `bufferViews.byteStride`).
- **Building variant coverage**: some variants are never selected; street repetition can still be reduced.
- **Third-party weapon asset cleanup**: one sniper source file contains two overlapping rifle bodies and needs manual de-duplication before it can ship.
- **Audio**: mono output only; no occlusion or distance-attenuation acoustic model.

---

## System Requirements

| Scenario | Recommended CPU | Recommended GPU | Memory |
|---|---|---|---|
| **1080p, no ray tracing** | Ryzen 3 3300X / Intel 11th-gen i3 | RX 6500 XT / RTX 3050 6GB | **8 GB minimum** |
| **1080p, ray tracing** | i5-11400F / Ryzen 7 3700X | RTX 2070 Super / Arc A750 | 12 GB recommended |
| **1440p, no ray tracing** | i5-12490F / Ryzen 5 5600X | RTX 3060 Ti / RX 6700 XT | 12 GB recommended |
| **1440p, ray tracing** | i7-12700K / Ryzen 7 7700X | RTX 5060 / RX 9060 XT | 16 GB recommended |
| **4K, low quality** | i7-13700K / Ryzen 9 7900X | RTX 3080 Ti / RX 7900 GRE | 24 GB recommended |
| **4K, high quality** | 270K Plus / Ryzen 9 9950X | RTX 4080 Super / RX 9070 XT | 32 GB recommended |

### Hardware Threshold Notes

- **The CPU is the hard gate, not the GPU.** At 128v128 every frame advances pathfinding, line-of-sight tests and ballistics for 256 soldiers, plus physics, audio mixing and network snapshots. **Core count matters more than single-core clock** — the engine pins work by CCX (AMD dual-CCD) and P/E cluster (Intel hybrid); below 6 cores the frame rate drops noticeably.
- **Memory**: 8 GB is the runnable floor, and at that level you should avoid compiling at the same time (see Local Deployment below). 12 GB or more is advised for the large battlefield with full assets loaded.
- **VRAM**: 2 GB suffices for the non-ray-traced path. Enabling the path-tracing reference view additionally allocates an RGBA32F accumulation buffer and acceleration structures — 6 GB or more advised.
- **Laptops**: discrete-GPU direct mode (MUX discrete) recommended. Under hybrid output, IMMEDIATE present is the most stable.

### Graphics API Requirements

- **Vulkan 1.3** (required). Instance and device are both created at 1.3.
- **`VK_EXT_mesh_shader`**: required for the main render path. When absent the engine falls back to the classic vertex pipeline — functionality unaffected, frame rate lower.
- **`VK_KHR_acceleration_structure` + `VK_KHR_ray_query`**: needed only by the path-tracing reference view; may be entirely absent without affecting the main flow.
- **`VK_KHR_swapchain`** (required); `VK_KHR_dynamic_rendering` is the future upgrade direction (already core in 1.3).
- Startup prints full `gpu-caps:` and RT pipeline property logs so the actually granted extensions and limits can be confirmed.

---

## Controls

| Key | Function |
|---|---|
| W / A / S / D | Move |
| Left mouse | Fire (in play state); drag to orbit in non-first-person windows |
| Right mouse | Aim down sights (ADS) |
| Tab | Cycle camera mode (FirstPerson → Orbit → Flight) |
| 1 / 2 or `/` command box | Switch weapon |
| B | Cycle fire mode (single / burst / auto) |
| R | Enter controls; revive after the death screen; reload during play |
| G | Throw grenade |
| N | Resupply / advance to next level |
| Q / E | Descend / ascend (flight and orbit modes) |
| F5 | Hot-reload the TOML level |
| Enter | Restart from the results screen |
| ESC | Menu and settings |

> **Mouse capture**: on entering gameplay the engine **grabs the cursor by itself** (no window click needed) and the system cursor is locked while held. For automated screenshot testing always use a wrapper with a `finally` force-kill (see `scripts/cap_safe.ps1`); never launch the game process bare.

---

## Multiplayer Guide

### Modes

- **Same machine, two players**: one host and one client process over `127.0.0.1`.
- **LAN, two players**: the client points at the host's LAN IP.
- **Across networks**: rendezvous through the `rdv.exe` relay with hole punching.

### Steps

```powershell
# ── Host ──
$env:RV3D_NET = 'server'
$env:RV3D_NET_ADDR = '0.0.0.0:27015'      # listen on all interfaces
start target\release\steel-front.exe

# ── Client ──
$env:RV3D_NET = 'client'
$env:RV3D_NET_ADDR = 'hostIP:27015'        # 127.0.0.1 on the same machine
start target\release\steel-front.exe
```

Cross-network play additionally needs a relay; both sides use the same room name:

```powershell
# On any reachable machine
rdv.exe 0.0.0.0:27020

# Both host and client add these (the room name must match)
$env:RV3D_NET_RDV  = 'relayAddress:27020'
$env:RV3D_NET_NAME = 'myroom'
```

### Mechanics and Caveats

- **Server-authoritative**: damage, hits and win conditions are resolved by the host. Clients only interpolate snapshots and extrapolate prediction, so there is no client-side stat-tampering path.
- **Protocol version handshake**: the join request carries a session version; a mismatch is refused outright and retries stop, preventing undefined desync between different builds.
- **Reconnect**: on snapshot timeout the client enters a reconnect wait and resumes automatically once the host returns.
- **Muzzle flash sync**: snapshots carry a per-unit "firing" flag, so the client sees the host's muzzle flash.
- The release package ships Host / Join batch files as double-click launchers — no manual environment setup required.
- Default ports are `27015` (game) and `27020` (relay); across networks either open them on the router or rely on the relay's hole punching.

---

## Vulkan Feature Notes

- Written and run against **Vulkan 1.3**; `ash` full 1.3 bindings; instance and device both at 1.3.
- **`VK_EXT_mesh_shader` main path**: the mesh shader performs both culling and vertex generation. One workgroup equals one instance slot, the frustum test runs on the GPU, and there is no per-frame CPU readback or re-upload. The ground instance field exceeds the spec minimum `maxMeshWorkGroupCount[0]` and is therefore dispatched in device-limited chunks.
- **Classic vertex pipeline**: retained only as a compatibility fallback where `VK_EXT_mesh_shader` is missing (WSLg, the dzn software driver, older GPUs). It is **frozen** and receives no feature work.
- Uses classic `vkRenderPass` (a 1.0 core subset, legal on a 1.3 device). Dynamic rendering is the planned upgrade.
- Synchronisation model: one command buffer per frame, frame semaphores, and `max_frames_in_flight` copies of the uniform and instance buffers to avoid read/write races.
- All geometry buffers are persistently mapped `HOST_VISIBLE | HOST_COHERENT`; textures go through a staging buffer.
- Instance data lives in a storage buffer as 80-byte `InstanceData` (4×4 model matrix + RGBA tint); the shape tag rides in `tint.w`, so **adding a shape never changes the vertex stride**.

## Rendering Feature Notes

- **Terrain**: three LOD meshes with inter-frame morphing (removes switch popping); the height field is a pure function shared by CPU and GPU, so gameplay and rendering read exactly the same terrain.
- **Lighting**: directional + point lights, Blinn-Phong model; PCF soft shadows; baked height-field AO and static sky light.
- **Textures**: world-space baked ground material map (asphalt / paver / grass / sand zones, 2 texels per metre) plus a near-field micro-detail tile layer (~128 texels per metre, two mip levels blended); procedural skin textures for obstacles and soldiers.
- **Atmosphere**: distance fog whose colour is shared with the sky colour, keeping the far "building-to-sky" boundary continuous.
- **Emissives**: explosions, muzzle flash and smoke use a dedicated instance range and fake volumetric glow through view-dependent radial falloff — no alpha blending, the main pass stays fully opaque.
- **MSAA 4×** + anisotropic filtering + full mip chains.
- **Path-tracing reference view** (currently disabled): `VK_KHR_ray_query` panoramic PT with sun-shadow-ray NEE and diffuse bounces, temporal spp accumulation denoising; once converged it can serve as ground truth for light baking.

### Shading Constraints to Know Before Authoring Assets

- **Normals never reach the GPU**: the vertex format is `pos(3) + color(3) + uv(2)` = 32 bytes, and shading normals are reconstructed from screen-space derivatives ⇒ **flat shading only**. Smooth-shaded high-poly imports will look faceted; AO and baked lighting must go into **vertex colour**.
- Consequently, **a wrongly-wound face renders black with no error at all**. Meshes from external modelling get one deliberate index flip at the engine boundary.
- Material identity is currently sniffed from vertex-colour channel ratios (glass, canopy, etc.). Authored assets opt out of that procedural facade work via a shape tag — otherwise the engine paints an extra set of misaligned procedural window bands over modelled facades.

---

## Project Structure

```
src/
├── main.rs            # winit event loop / input / render orchestration / inspect mode
├── ui.rs              # HUD, menus, settings, CJK glyph rasterisation (GDI)
├── audio.rs           # Procedural audio synthesis/mixing (AudioSink abstraction)
├── audio_out.rs       # Windows waveOut native FFI output backend
├── config.rs          # Config persistence
├── net.rs             # UDP client/server snapshot sync
└── engine/
    ├── renderer.rs    # Vulkan rendering (instance field/LOD/HUD/shadow/mesh+classic path)
    ├── meshgen.rs     # Procedural mesh generation (rounded box/frustum/sphere/torus)
    ├── game.rs        # Game logic hub (physics/weapons/AI/waves/pressure mode)
    ├── weapons.rs     # Weapon framework (Firearm/ProjectileWeapon/WeaponRack)
    ├── weapon_data.rs # 35-firearm V3.0 data table (ALL_WEAPONS)
    ├── guns/          # Per-weapon procedural modelling pure functions
    ├── ai.rs          # Tactical AI (A*/state machines/cooperation)
    ├── ai_command.rs  # Company/platoon/section hierarchy (3x3, reports, commander)
    ├── camera.rs      # Three-mode camera (first-person/orbit/flight)
    ├── physics.rs     # Player and rigid-body physics
    ├── map.rs         # Map generation / TOML levels
    ├── city.rs        # Hand-authored modern city layout
    ├── procedural.rs  # Procedural texture baking (city ground/skins)
    ├── cpu.rs         # CPU topology detection and thread affinity (Windows FFI)
    ├── lighting.rs    # Directional/point light and shadow uniforms
    ├── font_cjk.rs    # Windows GDI CJK glyph rasterisation
    ├── simd.rs        # SIMD dispatch and speedup measurement
    ├── gpu_caps.rs    # GPU capability enumeration
    └── ...            # objective / window / cjk_glyphs etc.
launcher/              # Zero-dependency Win32 GUI launcher (update/shortcut/wallpaper)
build.rs               # WGSL → SPIR-V compile at build time (naga 30)
assets/                # Generated SPIR-V, fallback textures, level TOMLs
scripts/               # Smoke / publish / debug scripts
GAME_DESIGN.txt        # Gameplay design document (single source of truth)
```

---

## Documentation Index

| Document | Purpose | When to read it |
|---|---|---|
| [GAME_DESIGN.txt](./GAME_DESIGN.txt) | **Single source of truth for gameplay design** | Before any judgement about numbers, mechanics or levels |
| [AGENTS.md](./AGENTS.md) | AI handoff log and iteration record (the project's only formal handoff medium) | Must-read before taking over; must be appended at the end of every iteration |
| [Large-Battlefield Firearm Design V3.0](./docs/大战场枪械设计V3.0.txt) | Full data table and design basis for the 35 weapons | When changing weapon numbers or adding a firearm |
| [LICENSE](./LICENSE) | Licence and commercial grant terms | Before redistribution or commercial use |
| `docs/experiment-*` | Verification records and measured data for rendering, lighting, performance | When you doubt whether a "known conclusion" still holds |
| `docs/perf-*` | Frame-rate and bottleneck benchmark archives | Before/after performance work |
| `docs/HANDOFF-*` | Historical handoff snapshots | To trace which round a decision was made in |

**Reading order**: start with `GAME_DESIGN.txt` to build gameplay understanding → then the project overview and the two doctrines at the top of `AGENTS.md` → when touching rendering, read the matching verification document under `docs/` rather than editing code directly. The "rendering technology route doctrine" and "verification discipline" entries in `AGENTS.md` are the two easiest places to regress.

---

## Local Deployment

### Prerequisites

| Item | Requirement |
|---|---|
| OS | Windows 10 / 11 (x86_64). Under WSLg the engine auto-falls back to the classic vertex pipeline |
| Rust toolchain | Stable `rustc` / `cargo` (2024 edition support) |
| GPU driver | Vulkan 1.3 capable (NVIDIA 512.xx+ / AMD 22.5+ / Intel current) |
| Disk | ~2 GB including `target/` build output |
| Memory | 12 GB or more advised; **at 12 GB or less only one `cargo` may run at a time** — concurrent builds stall out of memory |

### Dependencies

**All runtime dependencies are built-in OS components; nothing third-party is downloaded:**

- `ash` / `winit` / `glam` — Rust crates fetched by cargo at build time.
- **Vulkan loader** (`vulkan-1.dll`) — supplied by the GPU driver.
- `winmm` (`waveOut` audio), `gdi32` / `user32` (CJK glyph rasterisation, windowing and thread affinity) — Windows built-ins called directly through FFI.
- No C/C++/C# runtime, no Python runtime dependency, no external DLLs.

**Build-time dependencies:**

- `naga` (build-dependency only, WGSL → SPIR-V) and `spirv-tools` (validation).
- Optional: `glslangValidator` — only for the path-tracing GLSL, see `scripts/compile_pt.ps1`.
- Optional: Blender — only when regenerating 3D assets; invoked headless, no GUI required.

### Steps

```powershell
# 1) Clone
git clone <repo-url> steel-front
cd steel-front

# 2) Build (first run is a full compile and takes a while)
cargo build --release

# 3) Run: the working directory MUST be the repository root!
#    assets/*.spv and assets/props/ are loaded by path relative to the process cwd;
#    launching from elsewhere fails with "failed to open shader file (os error 3)"
$env:RV3D_AUTOSTART = '1'
.\target\release\steel-front.exe
```

### Verification and Self-Checks

```powershell
# Unit tests (pure logic, never touches the GPU)
cargo test --release

# Smoke test: launch, capture, check for VUID and device-lost, then force-kill
powershell -ExecutionPolicy Bypass -File scripts\run_gameplay_smoke.ps1

# Asset triage: can a batch of GLB files be loaded by this engine as-is?
python tools\glb_survey.py "path\to\models"
```

### Importing External 3D Assets

```powershell
# Weapons: triage → normalise (apply transforms / join to one mesh / bake textures into
# vertex colour / decimate / export dense buffers) → install under each weapon's key
& "blender.exe" --background --factory-startup --python tools\blender\prep_guns.py -- --in "D:\my\models"
python tools\install_guns.py

# World props: regenerate assets/props/ and emit a vertex-colour preview render for self-check
& "blender.exe" --background --python tools\blender\gen_props.py
& "blender.exe" --background --python tools\blender\gen_props.py -- --preview screenshots/kit.png
```

### Packaging a Release

```powershell
powershell -ExecutionPolicy Bypass -File scripts\publish.ps1
```

Output lands in `release_dist/` with the launcher, the game binary and the multiplayer host/join batch files.
⚠ When packaging, confirm `assets/guns/` and `assets/rt/` are shipped along with it — their absence raises no error; the game silently falls back to procedural weapon models.

---

## Licence

This project is licensed under **AGPL-3.0 with additional commercial use terms**. Full text in [LICENSE](./LICENSE).

```
Copyright (c) 2026 Huang Shaojie

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/agpl-3.0.html>.
```

### Additional Commercial Use Terms

These terms supplement the AGPL-3.0 licence above, to make the grant conditions in commercial scenarios explicit.

**1. Open-source use (free forever)**
Provided you **fully comply with AGPL-3.0** when using this software or its derivatives (i.e. keep the source public and release all modifications under AGPL-3.0 as well), you **owe no licence fee regardless of commercial scale or revenue**.

**2. Closed-source exemption (small scale, free)**
If you choose **not to comply with AGPL-3.0's open-source requirement** (i.e. you distribute in closed source or offer a closed-source network service), but you (or the entity you represent) had **total operating revenue below CNY 10,000,000** (or the equivalent in foreign currency) in your **most recent complete quarter**, then you **automatically receive a free closed-source use licence** — no payment and no need to get in touch.

**3. Closed-source licensing (large scale, paid)**
If you use this software in closed source and your (or your entity's) total operating revenue in the most recent complete quarter **reaches or exceeds CNY 10,000,000**, you **must contact the copyright holder** and obtain a separate written commercial licence; otherwise your use constitutes infringement and carries the corresponding legal liability.

**4. Definitions and clarifications**
- "Total operating revenue" means all income your (or your legal entity's) business activities generated in the most recent complete quarter, not limited to revenue derived directly from this software.
- If you operate multiple projects or products, revenue is aggregated.
- These additional terms do not modify AGPL-3.0; they are extra grant conditions on the use of this software. Where they conflict, these terms prevail — limited to the commercial authorisation portion.

### Commercial Licence Contact

To enquire about a closed-source commercial licence or to obtain written permission:

- Email: **huangshaojie925@gmail.com**

### Scope Regarding Third-Party Assets

The licence above covers only original code and documentation held by the copyright holder in this repository. Weapon models in `assets/guns/` and `assets/guns_ext/` downloaded from third-party sites are **not covered** and remain bound by their own source terms; each must be individually confirmed to permit redistribution and commercial use before it is introduced. `assets/props/` is generated originally by this project's headless Blender scripts and is covered by the licence above.
