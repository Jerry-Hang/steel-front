# 钢铁前线 · Steel Front

> 架空历史 · 2020 年代 · 大规模战场 FPS ｜ Rust + Vulkan 自研引擎（ash / winit / glam）｜ 零第三方游戏引擎依赖 ｜ Windows / Linux / Android 原生支持

> **🌐 语言 / Language** ：**📖 中文（当前）** ｜ [English ↓](#english)

> 📡 **跨设备记忆（ACDM）**：本项目实行**双轨** —— 代码提交 GitHub，同时把进展/决策写进
> [acdm-memory](https://github.com/Jerry-Hang/acdm-memory) 记忆库（`acdm-record steel-front "<摘要>" <仓库内相对路径...>` + `acdm-sync`）；
> 规范见 [Jerry-Hang/acdm](https://github.com/Jerry-Hang/acdm)。换设备后 `git pull` 记忆库即可接上上下文。

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
| 2026-09-09 | 枪口朝向根因根治（`detect_axes` 两端测量对调）；`assets/*.spv` 每次构建都变脏查清并修掉 |
| 2026-09-12 | 士兵观感定型：头/盔按真人比例收小、NPC 枪身收到真 AK-12 长度（0.94 m） |
| 2026-09-13 | **中文字模换源**：从专有 SimSun 换成 **Noto Sans SC（SIL OFL 1.1）**并裁到 1,580 码点 / 167 KB，附可复现提取器；士兵 GLB 上传移到构造之后（不再静默退回箱体） |
| 2026-09-15 | **弹孔系统**（子弹在障碍表面留弹着标记）；切枪到更小枪模不再重建在飞缓冲（按一下「2」不再丢设备） |
| 2026-09-16 | NPC 手榴弹改从手的高度出手（高帧率下不再原地自爆）；`RV3D_AI_DIAG=1` NPC 诊断埋点 |
| 2026-09-19 | 阴影建筑盒壳 LOD（sw12 fps 47→80）；GLB 建筑碰撞顶 = 视觉高（垂直方向同源） |
| 2026-09-22 | **survive 波次闭环**：harness 走位/重瞄 + Patrol 绕圈永不收敛的根因修复（真机首破 wave 1+2） |
| 2026-09-23 | 提交白名单守卫 + 历史密钥审计工具（链式保留原钩子）；枪模缓冲扩容改「先建后毁」 |
| 2026-09-25 | **所有 Vulkan 等待加上界**（acquire 1 s / 围栏 5 s + 卡死降级）——修掉「独显静默卡死」；命令缓冲按在飞帧槽位索引 |
| 2026-09-26 | **审计日**：阴影拆静态/动态两张图（**+23.5%**）、磨砂玻璃菜单、第一人称枪模动画（冲刺/换弹/呼吸）、SVD-12M 真模型实装、军情口径四连修（编制逐人闭合 / 阵亡计数 / 快照同源 / 重组可达）、十余处静默失败（perf_log / net / audio / assets / config）、GLB 与 TOML、网络三处变异模糊 |
| 2026-09-29 | **PT 与光栅同源日**：修掉 PT 取景水平拉伸 1.6×；PT 的地面与 marker 改采样**与实机同一套**程序化纹理；根治阴影内部棋盘格（PCF 抽头与阴影纹素相位锁死 ⇒ 改逐像素旋转核）；砌块皮肤两处尺度修正（砖行按世界尺度 + 最长轴 <1.5m 不发皮肤，护柱不再"刷条纹"）；🔴 查出 **PT 的 marker 盒自 09-17 起整体小一半**（改"渲染盒 = 碰撞 AABB"约定时漏改了这个消费者） |
| 2026-10-01 | **并行只读代理审计日**：修掉 `!is_canopy` 漏项——后加的砌块皮肤分支把 2026-08-23 的树冠值噪声**整块静默覆盖**；仓库天窗 **8 块玻璃原本一像素都不画**（被壳体完整包住，主通道全不透明）；占领点底盘/旗杆 **2× 超尺寸**（09-17 约定漂移的第二个漏改消费者，改由玩法半径推导）；新增两条守卫（跨文件重复常量、被完整包住的几何）。🔴 另记两条关于我自己的：`assets/*.spv` 运行时从磁盘读而 cargo 会缓存 `build.rs` ⇒ **改着色器只认 spv 哈希**；以及我为一次自我更正**编造过一条引用**，已撤回 |
| 2026-10-07 | **Linux 原生平台与工具链闭环**：Linux 性能尺子（`perf_run.sh`）、双进程联网真机验证（`net_pair.sh`，关闭未结案 #13）、交换链重建探针（`resize_probe.sh`）、A/A 噪声底探针（`aa_probe.sh`）、自动截图（`RV3D_SHOT_AT`）与看门狗；桌面集成（.desktop + SVG 图标 + 安装脚本）接入 `SteelFront.sh` |
| 2026-10-11 | **Android 原生移动端移植实装**：主体抽入 `lib.rs` 并编 `cdylib` 导出 `android_main`；引入只读资产抽象层 `AssetSource`（Fs / Mem / 原生 `AAssetManager`）；启用 `winit` 官方 `android-game-activity` 并适配生命周期挂起与恢复；触控双虚拟摇杆与开火 UI；sysfs CPU 容量聚类调度（1+3+4 核心亲和度绑定）；Vulkan 实例版本动态适配 `min(1.3, loader)` 与 FIFO 呈现 |
| 2026-10-14 | **渲染缩放与管线解耦（GSR2 阶段 ①）**：支持离屏场景图与升采样 blit，内部渲染分辨率与原生设备分辨率解耦；HUD 独立覆盖层渲染（保持设备高分辨率不受超分模糊影响）；新增 `RV3D_NO_MESH=1` 环境变量支持运行时强制走传统顶点管线 A/B 对比 |
| 2026-10-17 | **玩法、弹道、音频与网络深度排障收口**：子弹命中按参数 $t$ 排序（根治墙前 NPC 无法命中）；修复 NPC 朝向双坐标系导致的横向滑步与掩体原地冻结；修复 ADSR 衰减二次方过快与环境风相位丢失；网络打洞中继 `rdv` 增加 TTL（120s）与容量上限（4096），增加非法端口显式校验拒绝，客户端正确处理公网映射；35 把现代枪械的**拉机柄轴向 / 扳机与护圈位置 / 制退器前后径 / 两脚架朝向**批量修正，并新增 `tools/gun_sight_alignment.py` 建立机瞄共线审计（**17 条已知违规入基线，未全修**）；修复圆柱/圆锥侧面法线朝向反与全部图元绕序；**检视模式关背面剔除 + 开深度测试**消除内部结构穿模；守卫工具修正（`cjk_cover_check.py` 补「清单 ⊆ 表」校验，`prod_panic_sweep.py` 按路径排除 8 个纯测试文件后报 **10 处有守卫的生产 unwrap**） |

---

## 当前进度（截至 2026-10-17）

> 🔴 **状态与约束的唯一真源**：工程铁律 / 未结案清单 / 教训 = `AGENTS.md`，逐轮进度与交接 =
> `docs/PROGRESS.md`。本节只放"给外部读者看的一屏状态" —— **要改就改那两个文件**，
> 别在这里长出第二份会过期的清单（README 曾经就是这么落后 25 天的）。

**已完成（要点）**

**引擎与渲染**
- 纯 Rust + Vulkan 1.3，桌面 **10 个依赖**；网格着色器主路径（MESH + FRAGMENT）+ 传统管线兼容回退（支持 `RV3D_NO_MESH=1` 强制回退对比）。
- 内部渲染分辨率与动态升采样 blit（GSR2 阶段 ①），HUD 覆盖层保持设备原生分辨率独立渲染。
- 地形三级 LOD（257² / 129² / 65²）+ 帧间形态过渡；65536 实例地面场；MSAA 4×；完整 mip 链。
- 阴影：**静态/动态两张图分离**（静态每 30 帧、动态每 2 帧重画）；烘焙 AO 与静态天光；程序化贴图。
- HUD 覆盖层自包含（独立管线与顶点缓冲，不侵入主渲染）；磨砂玻璃菜单；Noto Sans SC 裁剪点阵字模光栅化。
- 检视模式（`RV3D_INSPECT=1`）深度测试与法线修复，彻底消除部件重叠穿透与看到内部结构。

**多平台与硬件调度**
- **跨平台原生支持**：Windows 原生（DirectX/WinMM/奥创中心）、Linux 原生（Wayland/KWin/ALSA/桌面集成）、Android 原生（`cdylib` + `android-game-activity` + 触控双摇杆 + `AAssetManager`）。
- **硬件拓扑调度**：自动识别 AMD CCX / Intel P/E 核与移动端 ARM 1+3+4 大小核拓扑，拆分 `scene_pool` 与 `ai_pool` 双池亲和度绑定，远端部队自动低频更新。

**玩法与系统**
- 35 把现代枪械，V3.0 数据表驱动（初速、下坠、散射、距离衰减、部位伤害、开火模式、ADS 机瞄视线），全枪型机瞄视线、准星、扳机护圈与制退器全局几何标定。
- 大战场：红 128 vs 蓝 127+玩家；波次 / 压力 / 据点三种规则；可破坏障碍与爆炸冲击波。
- AI：三三制编制（营→连→排→班，**连名单逐人闭合**）、火-机动交替、掩体选择；
  **HTTP LLM 战术指挥官**（红蓝各一上下文窗口互搏，无/失效时回退启发式）。
- 联机：服务器权威 + 快照插值 + 断线重连 + 协议握手 + `rdv` 极简文本式 UDP 打洞中继（带 TTL 与容量上限）。
- 关卡数据化：TOML 地图（出生点/目标/障碍/规则）、F5 热重载、6 张索引地图。

**外部资产管线**
- 抽象只读资产来源 `AssetSource`（支持文件系统 `FsSource`、内存 `MemSource`、Android 原生 `AAssetManager`）。
- OBJ 与 glTF GLB 零依赖解析器（多 mesh 合并、componentType 感知、交错 `byteStride`、COLOR_0 顶点色）。
- Blender 无头控制闭环：导入 → 材质/AO 烘焙 → 节点净化 → 导出 GLB → 渲染 PNG → 看图自检。
- 程序化城市（板楼/抹灰楼模块）+ 道具焊接（空间 40m 分桶 + 逐桶视锥剔除，fps 112→152）。

**验收现状（2026-10-17 实测）**
- `cargo test --release` **756 passed / 0 failed**；`cargo build --release` **0 警告**
  （🔴 判据只认 `cargo build --release` —— `cargo test` 会因 `cfg(test)` 漏报）。
- 生产代码的 unwrap/expect **全部有前置守卫或是不变式**（`tools/prod_panic_sweep.py` 报 10 处，逐条核过；扫描器按路径排除 8 个纯测试文件）。
- Noto Sans SC 裁剪点阵字模覆盖校验全绿（`tools/cjk_cover_check.py` 严格校验源码及注释中 1591 个 CJK 字符在字模表中逐一存在）。
- 冒烟（`scripts/run_smoke_pm.ps1` / `scripts/smoke_linux.sh`）判据 = `vuid==0 and panics==0 and killed>=1`；
  整局验证层跑法见 `AGENTS.md` 铁律 B。
- 会战军情不变式（判据工具 `tools/battle_tally_check.py`）：`阵亡 + Σ连强度 == 编制` 与
  `本营战果 == 敌方阵亡` —— 逐行成立才算过。

**主要待办**（完整清单与 lead 见 `AGENTS.md` 的未结案清单）

- **PT 与光栅同屏叠加**（当前是整屏替换）；**士兵骨骼动画**（`docs/HANDOFF-soldier.md`）。
- **GPU 空闲窗口的 n≥5 对轮转成本地图**（尺子三件套，见 `AGENTS.md` 教训 45）。

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
├── main.rs            # 桌面二进制薄入口（调用 steel_front::run_steel_front_desktop）
├── lib.rs             # 引擎主体 / GameApp 事件循环 / 输入 / 渲染编排 / Android android_main
├── bin/
│   └── rdv.rs         # 极简文本 UDP 联机打洞与房间名解析中继服务
├── ui.rs              # HUD / 菜单 / 设置 / Noto Sans SC 点阵中文字模渲染
├── audio.rs           # 程序化音频合成 / 混音（AudioSink 抽象）
├── audio_out.rs       # 原生音频输出后端（Windows waveOut / Linux ALSA）
├── config.rs          # 配置持久化（原子写 + 容错加载）
├── net.rs             # UDP client/server 快照同步与网络层
├── asset_source.rs    # 只读资产抽象层（FsSource / MemSource / AAssetManager）
└── engine/
    ├── renderer.rs    # Vulkan 渲染核心入口及类型声明
    ├── renderer/      # 拆分的 19 个渲染子模块（几何/实例/管线/描述符/交换链/PT/阴影/录制等）
    ├── game.rs        # 游戏逻辑中枢入口及类型声明
    ├── game/          # 拆分的 16 个游戏逻辑子模块（会话/玩家/武器/弹道/AI/波次/联机/地图等）
    ├── meshgen.rs     # 程序化网格生成（圆角盒/锥台/球/环/法线标定）
    ├── weapons.rs     # 武器框架（Firearm / ProjectileWeapon / WeaponRack）
    ├── weapon_data.rs # 35 把枪 V3.0 数据表（ALL_WEAPONS）
    ├── guns/          # 各枪程序化建模纯函数
    ├── ai.rs          # 战术 AI（A* / 状态机 / 掩体选择）
    ├── ai_command.rs  # 连排班指挥体系（三三制 / 军情汇报 / 司令决策）
    ├── camera.rs      # 三模式相机（第一人称 / 轨道 / 飞行）
    ├── physics.rs     # 玩家 / 刚体物理与精确 Y 轴支撑碰撞
    ├── map.rs         # 手写零依赖 TOML 解析器 / 地图关卡加载
    ├── city.rs        # 手工绘制现代城市布局（55m 街区网格 / 零共面）
    ├── procedural.rs  # 程序化纹理烘焙（城市地面 / 皮肤）
    ├── cpu.rs         # CPU 拓扑检测与线程亲和绑定（Windows / Linux / Android）
    ├── lighting.rs    # 方向光 / 点光 / 阴影 Uniform / 静态天光
    ├── cjk_glyphs.rs  # Noto Sans SC 裁剪点阵字模（1595 码点）
    ├── simd.rs        # SIMD 选路与爆炸冲击波加速
    ├── gpu_caps.rs    # 显卡能力枚举
    └── ...            # objective / window 等
android/               # Android 原生 Gradle 工程（game-activity / arm64-v8a）
desktop/               # Linux 桌面图标与 .desktop 启动文件
launcher/              # 零依赖 Win32 GUI 启动器（更新 / 快捷方式 / 壁纸）
build.rs               # WGSL → SPIR-V 构建期编译（naga 30）
assets/                # SPIR-V 着色器、回退贴图、地图 TOML、GLB 模型
scripts/               # 跨平台冒烟 / 打包 / 探针 / 调试脚本（PowerShell + Bash）
tools/                 # 自动化审计守卫（字模覆盖 / 生产 panic / 军情守恒 / 瞄准线标定）
```

---

## 文档索引与指导

| 文档 | 用途 | 什么时候该读 |
|---|---|---|
| [AGENTS.md](./AGENTS.md) | **工程约束的唯一载体**：铁律 A–H、未结案清单、46 条教训、验收红线 | 接手开发前必读；每次改动前对照 |
| [docs/PROGRESS.md](./docs/PROGRESS.md) | **逐轮进度与交接记录**（每轮一节，含红测→修法→判据） | 想知道"上一轮做了什么/下一步做什么"时 |
| [docs/pitfalls.md](./docs/pitfalls.md) | **历史事故与陷阱总集**（1334 个提交中 283 个 fix 提炼出 162 条误区，65% 集中在静默失败与判据失效） | 写判据、改管线或调查疑难 Bug 前必读 |
| [docs/linux-native.md](./docs/linux-native.md) | Linux 原生开发/验证环境指南、Wayland 与独显混合输出调试（命令在 §10–§15） | 在 Linux 环境开发或运行验证前 |
| [docs/HANDOFF-mobile.md](./docs/HANDOFF-mobile.md) | 移动端（Android / iOS）移植进展、架构与交接记录 | 开发或调试 Android 目标时 |
| [大战场枪械设计V3.0](./docs/大战场枪械设计V3.0.txt) | 35 把枪的完整数据表与设计依据 | 改武器数值或新增枪械时 |
| [LICENSE](./LICENSE) | 许可与商业授权条款 | 再分发或商用前 |
| [THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md) | 第三方依赖与素材的逐项来源核实、再分发清单、两项已知风险 | **再分发本仓库或其构建产物之前必读** |
| [docs/WEAPON-LICENCE-AUDIT.md](./docs/WEAPON-LICENCE-AUDIT.md) | 枪械模型的逐项许可核对工作表（14 个来源 slug / 判定表 / 已知线索 / 两个陷阱） | 清理第三方素材授权时 |
| `docs/progress/*.md` | 切档后的细分主题进度（光栅、光追、几何、枪械角色、AI网络音频、工程方法） | 深入了解特定子系统演进过程时 |
| `docs/perf-*` | 帧率与瓶颈基准存档 | 做性能优化前后对比 |
| `docs/HANDOFF-*` | 历史交接快照 | 追溯某个决定是在哪一轮做出的 |

**阅读建议**：先读 `大战场枪械设计V3.0` 与 [AGENTS.md](./AGENTS.md) 顶部的项目概览与铁律建立认知；写判据前必读 [docs/pitfalls/lying-gates.md](./docs/pitfalls/lying-gates.md)；改渲染时读 `docs/progress/` 下对应的验证记录，而不是直接改代码。`AGENTS.md` 里的「渲染技术路线铁律」与「验证纪律」是本项目最容易踩的两个坑。

---

## 本地部署方法

### 前置条件

| 项目 | 要求 |
|---|---|
| 操作系统 | Windows 10 / 11（x86_64）或 Linux 原生（Wayland / X11）。移动端支持 Android（arm64-v8a） |
| Rust 工具链 | 稳定版 `rustc` / `cargo`（需支持 2021/2024 edition）；Android 目标需 `cargo-ndk` |
| 显卡驱动 | Vulkan 1.3 兼容驱动（NVIDIA 512.xx+ / AMD 22.5+ / Intel 最新） |
| 磁盘 | 约 2 GB（含 `target/` 构建产物） |
| 内存 | 建议 12 GB 起；**12 GB 及以下时同一时刻只能跑一个 `cargo`**，并行构建会导致内存耗尽式挂起 |

### 依赖说明

**运行时依赖全部为系统自带组件，不下载任何第三方运行时：**

- `ash` / `winit` / `glam` —— Rust crate，构建时由 cargo 拉取（桌面直接依赖仅 10 个）。
- **Vulkan 加载器**（Windows 为 `vulkan-1.dll`，Linux 为 `libvulkan.so.1`）—— 由显卡驱动或系统提供。
- 音频：Windows 为 `winmm`（`waveOut` FFI），Linux 为 `libasound`（ALSA）。
- Android 目标：通过 `winit` 的 `android-game-activity` 接入原生 Activity 生命周期，使用 `AAssetManager` 读取打包资产。
- 无 C/C++/C# 运行时，无 Python 运行时依赖，无外部第三方 DLL。

**构建期依赖：**

- `naga`（仅 build-dependencies，用于 WGSL → SPIR-V）与 `rspirv`。
- 可选：`glslangValidator`（仅路径追踪 GLSL 需要，见 `scripts/compile_pt.ps1` 或 `compile_pt.sh`）。
- 可选：Blender（仅重新生成 3D 资产时需要，headless 调用，不需要 GUI）。

### 部署步骤

**Windows**：
```powershell
# 1) 克隆并构建
git clone https://github.com/Jerry-Hang/steel-front.git
cd steel-front
cargo build --release

# 2) 运行（工作目录必须是仓库根，或同级带 assets/）
$env:RV3D_AUTOSTART = '1'
.\target\release\steel-front.exe
```

**Linux 原生**：
```bash
# 1) 克隆并构建（详见 docs/linux-native.md）
git clone https://github.com/Jerry-Hang/steel-front.git
cd steel-front
cargo build --release

# 2) 运行（通过包装脚本设置 Wayland/KWin 环境）
./SteelFront.sh
```

**Android**：
```bash
# 需已安装 Android NDK，走 cargo-ndk 构建 cdylib
cargo ndk -t arm64-v8a build --release
```

### 验证与自检

```bash
# 单元测试（纯逻辑，不触碰 GPU；0 警告的判据要看 cargo build --release）
cargo test --release
cargo build --release

# 冒烟验证（PostMessage / 无头事件注入，不抢前台、不抓光标）
# Windows:
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_smoke_pm.ps1
# Linux:
./scripts/smoke_linux.sh
# 判据 = vuid==0 and panics==0 and killed>=1（无 fps 门槛）

# 生产代码无 panic 扫描
python3 tools/prod_panic_sweep.py

# 中文字模覆盖校验（确保源码中每个 CJK 字符在字模表中存在）
python3 tools/cjk_cover_check.py

# 会战 + 军情不变式（阵亡 + Σ连强度 == 编制、本营战果 == 敌方阵亡）
python3 tools/battle_tally_check.py logs/llmbattle.log.err
```

⚠ `scripts\run_gameplay_smoke.ps1` **是废弃的历史记录**（SendInput 注入在本机结构上不工作），别照它跑。

### 接入外部 3D 资产

```powershell
# 枪械：体检 → 规范化（应用变换 / 合并单网格 / 贴图烘进顶点色 / 减面 / 密集缓冲导出）→ 按武器 key 安装
& "blender.exe" --background --factory-startup --python tools\blender\prep_guns.py -- --in "D:\我的模型"
python tools\install_guns.py

# 世界道具：改生成器 → 重新生成 → 焊接（顶点数就是帧率，见 AGENTS.md 铁律 D）
& "blender.exe" --background --python tools\blender\build_city_kit.py -- <out_dir>
& "blender.exe" --background --python tools\blender\weld_props.py -- <in.glb> <out.glb>
& "blender.exe" --background --python tools\blender\preview_glb.py -- <in.glb> <out_prefix>
```

### 打包

仓库提供现成的跨平台自动化打包脚本：

- **Windows**：运行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package_release.ps1`，自动编译并生成带 release exe、`assets/` 与启动脚本的 `dist\steel-front-<tag>.zip`。
- **Linux**：运行 `./SteelFront.sh package`（调用 `scripts/package_release.sh`），生成 Linux 发行包压缩包。
- 玩家入口为 `SteelFront.bat`（Windows）或 `SteelFront.sh`（Linux）。

---

## 许可证 / Licence

本项目采用 **双许可（dual licensing）**：由你二选一，而不是"AGPL 上再叠条款"。

| 选项 | 许可 | 什么时候选 |
|---|---|---|
| **A. 自由软件** | **GNU AGPL-3.0-or-later**，完整原文就在 [`LICENSE`](./LICENSE) | 你能接受 AGPL §13（分发或以网络服务形式运行修改版时公开源码）。**任何规模的公司都免费、无需联系作者** |
| **B. 商业许可** | 作者另行签署的**书面商业许可**，条款见 [`LICENSE-COMMERCIAL.md`](./LICENSE-COMMERCIAL.md) | 你要闭源分发、把引擎嵌进专有产品，或无法接受 §13 |

**权威文件只有三份**（本 README 不复制其内容）：

| 文件 | 内容 | 什么时候必须读 |
|---|---|---|
| [`LICENSE`](./LICENSE) | **GNU AGPL-3.0 原文（661 行，未改动一字）** | 再分发、商用、或闭源使用前 |
| [`LICENSE-COMMERCIAL.md`](./LICENSE-COMMERCIAL.md) | 可选的商业许可（含季度营收 1000 万元以下的免费政策）+ 联系方式 | 无法接受 AGPL 时 |
| [`THIRD-PARTY-NOTICES.md`](./THIRD-PARTY-NOTICES.md) | 依赖与素材的**逐项来源核实**、§2b 下载资产清单、再分发清单 | **再分发仓库或构建产物之前必读** |

> 🔴 **2026-10-30 修正**：旧版 `LICENSE` 把"AGPL-3.0 + 附加商业条款（冲突时附加条款优先）"写在同一份文件里。
> 这个写法**在法律上不成立**：AGPL §7 只允许一份**封闭清单**里的附加条款（免责、保留声明、不得虚假来源、
> 名称/商标限制），§10 明确禁止对接收者施加进一步限制 —— 按营收触发的限制两者都不属于。
> 结果只有两种读法：要么附加条款**无效**（接收者直接按 AGPL 走），要么整份文件**根本不是 AGPL**
> （而"没有有效许可"= 默认保留所有权利，比前者更糟）。现在拆成"AGPL 原文 + 独立可选的商业许可"，
> 商业意图完全不变，但结构合法。

⚠️ **再分发前请读这两条**（逐项依据见 `THIRD-PARTY-NOTICES.md`，枪械另见
[`docs/WEAPON-LICENCE-AUDIT.md`](./docs/WEAPON-LICENCE-AUDIT.md)）：

1. ✅ **中文字模已解决**（2026-09-14）：不再来自专有字体，而是从 **Noto Sans SC（SIL OFL 1.1）**
   重新提取，并裁到源码真正用到的码点；OFL 全文随包发布
   （`assets/fonts/OFL-NotoSansCJK.txt`），提取器 `tools/extract_cjk_glyphs.py` 可复现，
   `cargo test` 会挡住"加了没有字模的字"。**唯一义务 = 保留 OFL 文本与出处**。
2. ⚠️ **枪械模型只完成了部分核实**（§4，2026-09-14：14 件里 4 件确认、2 件较可能，全部 CC BY）：
   **闭源/商业构建发布前，必须按 `docs/WEAPON-LICENCE-AUDIT.md` 逐件核实**；
   若某件不适合再分发，从 `assets/guns*/` 删除即可 —— 引擎对缺模型会回退 `ak12.glb`，**不会坏构建、只降画质**。

**关于下载的 CC0 资产**（`assets/props/` 里的 11 件、`assets/textures/` 里的 3 张与图集）：
它们来自 **Quaternius / Poly Haven / ambientCG**，全部 **CC0 1.0（公有领域）**，**不是**作者的版权、
也**不在** AGPL/商业许可范围内 —— 但 CC0 不要求任何义务，随便用、随便商用，逐件来源见
`THIRD-PARTY-NOTICES.md` §2b。

**再分发清单（照做即可）**：`LICENSE`（AGPL 全文，§4 要求随包给出全文而不是链接）+ `NOTICE`
+ `THIRD-PARTY-NOTICES.md` + `assets/fonts/OFL-NotoSansCJK.txt` 随包；依赖许可证文本按 §1 附带；
`assets/guns*/` 逐件过审；`assets/guns/` 与 `assets/rt/` 缺少时不会报错、只会静默回退 ——
打包后请实际清点一次。

---

This project is **dual-licensed** — pick one of two paths; it is not "AGPL plus
extra conditions bolted on".

| Option | License | When to pick it |
|---|---|---|
| **A. Free software** | **GNU AGPL-3.0-or-later** — the complete, unmodified text is [`LICENSE`](./LICENSE) | You can accept AGPL §13 (publish the corresponding source when you distribute, or when you run a modified version as a network service). Free at any company size, no contact needed. |
| **B. Commercial** | A separate written license from the copyright holder — terms in [`LICENSE-COMMERCIAL.md`](./LICENSE-COMMERCIAL.md) | You need closed-source distribution, or to embed the engine in a proprietary product. |

**Authoritative files — exactly three, deliberately not duplicated here**
(this README used to carry both a Chinese and an English copy that drifted):

| File | Contents |
|---|---|
| [`LICENSE`](./LICENSE) | The full GNU AGPL-3.0 text (661 lines, verbatim) |
| [`LICENSE-COMMERCIAL.md`](./LICENSE-COMMERCIAL.md) | Optional commercial license, including the free-above-CNY-10M-quarterly-revenue policy |
| [`THIRD-PARTY-NOTICES.md`](./THIRD-PARTY-NOTICES.md) | Per-item dependency/asset provenance, the CC0 inventory (§2b), redistribution checklist |

> 🔴 **Fixed on 2026-10-30.** The previous `LICENSE` mixed AGPL-3.0 with
> "additional commercial terms that prevail over the AGPL" in one document.
> That construction is not valid: AGPL §7 allows additional terms only from a
> closed list (disclaimers, notice preservation, no false claims of origin,
> name/trademark limits) and §10 forbids any further restriction on recipients;
> a revenue-triggered restriction is neither. Read one way the extra terms are
> void, read the other way the document is not AGPL at all — and an unlicensed
> work is "all rights reserved", which is worse for everyone. It is now the
> two-file dual-licensing structure above: identical business intent, valid
> legal shape.

**Downloaded CC0 assets** (11 props in `assets/props/`, 3 textures plus the prop
atlas in `assets/textures/`) come from **Quaternius / Poly Haven / ambientCG**
and are **CC0 1.0 (public domain)**: not the copyright holder's to re-license,
but also free of any obligation — commercial use included. Per-file provenance
is in `THIRD-PARTY-NOTICES.md` §2b.

⚠️ **Read these two before redistributing** (per-item evidence in `THIRD-PARTY-NOTICES.md`;
weapon models additionally in [`docs/WEAPON-LICENCE-AUDIT.md`](./docs/WEAPON-LICENCE-AUDIT.md)):

1. ✅ **CJK glyphs: resolved (2026-09-14).** They are no longer derived from a proprietary font but
   re-extracted from **Noto Sans SC (SIL OFL 1.1)**, trimmed to the code points the source actually
   uses. The full OFL text ships at `assets/fonts/OFL-NotoSansCJK.txt`, the extractor
   (`tools/extract_cjk_glyphs.py`) is reproducible, and `cargo test` rejects any character that has
   no glyph. **The only obligation is keeping the OFL text and the attribution** (nothing here is
   named "Noto", so the Reserved Font Name clause needs no action).
2. ⚠️ **Weapon models: only partially audited** (§4, 2026-09-14 — 4 of 14 positively identified,
   2 more plausible, all CC BY). **Verify each asset against `docs/WEAPON-LICENCE-AUDIT.md` before
   shipping a commercial or closed-source build.** If one cannot be redistributed, just delete it
   from `assets/guns*/`: the engine falls back to `ak12.glb`, so the build stays valid and only the
   visuals degrade.

**Redistribution checklist:** ship `LICENSE` (the full AGPL text — §4 requires
conveying a copy, a URL is not enough), `NOTICE`, `THIRD-PARTY-NOTICES.md` and
`assets/fonts/OFL-NotoSansCJK.txt`; attach the license text of each direct Rust dependency
(§1 lists them; the full graph is 314 crates, all permissive); clear `assets/guns*/` item by item;
and count `assets/guns/` + `assets/rt/` after packaging — their absence raises no error, it silently
falls back to procedural models.

---

<a id="english"></a>

# Steel Front · English

> Alternate-history 2020s large-scale battlefield FPS — a Rust + Vulkan engine written from
> scratch (ash / winit / glam), with zero third-party game-engine dependencies. Native Windows / Linux / Android support.

> **🌐 Language / 语言** ：[中文 ↑](#钢铁前线--steel-front) ｜ **📖 English（当前）**

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
| 2026-09-09 | Muzzle-orientation root cause fixed (the two ends of `detect_axes` were swapped); `assets/*.spv` rewriting itself dirty on every build diagnosed and fixed |
| 2026-09-12 | Soldier look locked in: head/helmet shrunk to human proportions, NPC rifle shortened to the real AK-12 length (0.94 m) |
| 2026-09-13 | **CJK glyph table re-sourced**: proprietary SimSun replaced by **Noto Sans SC (SIL OFL 1.1)**, trimmed to 1,580 code points / 167 KB, with a reproducible extractor; soldier GLB upload moved after construction (it can no longer silently fall back to boxes) |
| 2026-09-15 | **Bullet-hole decals** (impacts leave marks on obstacle surfaces); switching to a smaller weapon no longer rebuilds in-flight buffers (pressing "2" no longer kills the device) |
| 2026-09-16 | NPC grenades thrown from hand height (no more self-detonation at high frame rates); `RV3D_AI_DIAG=1` NPC diagnostics |
| 2026-09-19 | Shadow building-shell LOD (sw12 fps 47→80); GLB building collision top = visual height (same source in the vertical axis) |
| 2026-09-22 | **survive wave loop closed**: harness repositioning/re-aiming plus the root cause of Patrol circling forever (first real-machine clear of waves 1+2) |
| 2026-09-23 | Commit whitelist guard + history secret audit (chaining the pre-existing hook); weapon-buffer growth switched to "create first, destroy after" |
| 2026-09-25 | **Every Vulkan wait bounded** (acquire 1 s / fence 5 s + stall degradation) — the "discrete GPU hangs silently" defect; command buffers indexed by in-flight frame slot |
| 2026-09-26 | **Audit day**: shadows split into static/dynamic maps (**+23.5%**), frosted-glass menus, first-person weapon animation (sprint/reload/breathing), SVD-12M real model shipped, four battle-intel accounting fixes (roster closure / death tally / snapshot identity / regroup reachability), a dozen silent failures (perf_log / net / audio / assets / config), mutation fuzzing over GLB, TOML and the network codec |
| 2026-09-29 | **PT / raster same-source day**: fixed PT's 1.6× horizontal FOV stretch; PT's ground and markers now sample the **same** procedural textures as the raster; cured the shadow-interior checkerboard (PCF taps phase-locked to shadow texels → per-pixel rotated kernel); two masonry-skin scale fixes (metric courses plus a <1.5 m span gate, so bollards no longer look "painted with stripes"); 🔴 found that **PT marker boxes had been half-size since 09-17** (the "render box = collision AABB" convention change missed this consumer) |
| 2026-10-01 | **Parallel read-only agent audit day**: fixed the missing `!is_canopy` term — the later masonry-skin branch was **silently overwriting** the 2026-08-23 canopy value noise; 8 warehouse skylight glass panes that **rendered zero pixels** (strictly enclosed by the shell, and the main pass is fully opaque); the capture-point disc and pole were **2× oversized** (second missed consumer of the 09-17 convention change, now derived from the gameplay radius); two new guards (cross-file duplicated constants, strictly-enclosed geometry). 🔴 Plus two entries about myself: `assets/*.spv` is read from disk at runtime while cargo caches `build.rs` ⇒ **verify shader edits by spv hash, never by "it compiled"**, and a **quote I fabricated** to support one of my own corrections — retracted |
| 2026-10-07 | **Linux native platform & tooling closed loop**: Linux performance benchmark (`perf_run.sh`), two-process live networking verification (`net_pair.sh`, closing open item #13), swapchain recreation probe (`resize_probe.sh`), A/A noise floor probe (`aa_probe.sh`), automated screenshots (`RV3D_SHOT_AT`) and watchdog; Linux desktop integration (.desktop + SVG icon + installer) wired into `SteelFront.sh` |
| 2026-10-11 | **Android native mobile port shipped**: core engine moved into `lib.rs` and compiled as `cdylib` exporting `android_main`; introduced read-only asset abstraction `AssetSource` (Fs / Mem / native `AAssetManager`); enabled official `winit` `android-game-activity` handling Resumed/Suspended lifecycle; dual-stick on-screen touch controls and firing UI; sysfs CPU capacity clustering scheduler (1+3+4 core affinity binding); Vulkan instance dynamically falls back to `min(1.3, loader)` with FIFO presentation |
| 2026-10-14 | **Render scaling & pipeline decoupling (GSR2 phase ①)**: offscreen scene image + upsample blit decoupling internal rendering resolution from native display; HUD moved to independent overlay pass preserving native crispness; added `RV3D_NO_MESH=1` environment switch to force classic vertex pipeline for A/B evidence |
| 2026-10-17 | **Gameplay, ballistics, audio & networking audit closure**: raycast hits ordered by segment parameter $t$ (fixing obstacles occluding closer NPCs); fixed NPC dual-coordinate facing slip and cover seek self-freeze; fixed ADSR quadratic decay rate and wind ambient phase loss; UDP rendezvous relay `rdv` fortified with 120s TTL and 4096 entry ceiling, explicit invalid port rejection, and client parsing external address; all 35 firearms corrected for charging-handle axis, trigger/guard placement, muzzle-brake taper and bipod orientation, with a new `tools/gun_sight_alignment.py` iron-sight audit (**17 known misalignments baselined, not all fixed**); fixed inverted cylinder/cone lateral normals and all primitive winding; **inspect mode now disables back-face culling and enables depth testing** to eliminate interior bleed; guards corrected (`cjk_cover_check.py` now verifies the list is a subset of the table, `prod_panic_sweep.py` excludes 8 test-only files by path and reports **10 guarded production unwraps**) |

---

## Current Progress (as of 2026-10-17)

> 🔴 **Single source of truth**: engineering rules / open items / lessons live in `AGENTS.md`;
> per-round progress and handover live in `docs/PROGRESS.md`. This section is a one-screen
> status for outside readers only — **edit those two files**, do not grow a second list here
> (this README used to be 25 days behind for exactly that reason).

### Completed

**Engine and rendering**
- Pure Rust + Vulkan 1.3, **10 dependencies** on desktop; mesh-shader main path (MESH + FRAGMENT) with the
  classic vertex pipeline kept as a fallback (with `RV3D_NO_MESH=1` forced-fallback A/B testing).
- Internal rendering resolution & dynamic upsample blit (GSR2 phase ①); HUD overlay rendered independently at native display resolution.
- Three-level terrain LOD (257² / 129² / 65²) with inter-frame morphing; 65536-instance ground
  field; MSAA 4×; full mip chains.
- Shadows: **split static/dynamic shadow maps** (static every 30 frames, dynamic every 2);
  baked AO and static sky light; procedural textures.
- Self-contained HUD overlay (own pipeline and vertex buffer); frosted-glass menus;
  Noto Sans SC trimmed CJK glyph rasterisation.
- Inspect mode (`RV3D_INSPECT=1`) with depth testing and normal fixes, eliminating overlapping internal parts.

**Multi-platform and scheduling**
- **Cross-platform**: Windows native (DirectX/WinMM/Armoury Crate), Linux native (Wayland/KWin/ALSA/desktop integration), Android native (`cdylib` + `android-game-activity` + dual on-screen touch sticks + `AAssetManager`).
- **Hardware topology scheduling**: detects AMD CCX / Intel P/E-cores and ARM 1+3+4 mobile topologies, splitting into `scene_pool` and `ai_pool` with affinity binding, down-ticking distant units.

**Gameplay and systems**
- 35 modern firearms driven by the V3.0 data table (muzzle velocity, drop, spread, distance
  falloff, per-body-part damage, fire modes, ADS), with full iron-sight and geometry alignment across all weapons.
- Large battlefield: red 128 vs blue 127 + player; wave / pressure / capture-point rules;
  destructible obstacles and explosion shockwaves.
- AI: 3×3 hierarchy (battalion → company → platoon → section, with **every soldier carried by a
  company**), fire-and-maneuver, cover selection, and an **HTTP LLM battlefield commander**
  (one context window per side, heuristic fallback when unavailable).
- Multiplayer: server-authoritative + snapshot interpolation + reconnect + protocol handshake
  + `rdv` lightweight UDP rendezvous relay (with TTL and capacity ceiling).
- Data-driven levels: TOML maps (spawns / objectives / obstacles / rules), F5 hot reload,
  6 indexed maps.

**External asset pipeline**
- Abstract read-only asset source `AssetSource` (supporting `FsSource`, `MemSource`, and Android native `AAssetManager`).
- Zero-dependency OBJ and glTF GLB parsers (multi-mesh merge, componentType-aware accessors,
  interleaved `byteStride`, COLOR_0 vertex colour).
- Headless Blender loop: import → material/AO bake → node cleanup → GLB export → PNG preview.
- Procedural city (panel-block and plastered-block kits) + welded props (spatial 40m binning + frustum culling, fps 112→152).

**Verification status (measured 2026-10-17)**
- `cargo test --release` **756 passed / 0 failed**; `cargo build --release` **0 warnings**
  (🔴 the warning gate only accepts `cargo build --release`; `cargo test` under-reports).
- Every production `unwrap`/`expect` is guarded or an invariant (`tools/prod_panic_sweep.py` reports 10, each reviewed; it excludes 8 test-only files by path).
- Noto Sans SC trimmed CJK glyph coverage verified (`tools/cjk_cover_check.py` strictly verifies that all 1,591 CJK code points used across source code and comments exist in the glyph table).
- Smoke (`scripts/run_smoke_pm.ps1` / `scripts/smoke_linux.sh`) criterion = `vuid==0 and panics==0 and killed>=1`.
- Battle-intel invariants (tool `tools/battle_tally_check.py`): `own_deaths + Σstrengths == roster`
  and `camp score == the other camp's deaths` — checked line by line.

### Backlog (full list with leads: the open-items list in `AGENTS.md`)

- **Path-traced view composited over the raster frame** (currently a full-screen replacement);
  **soldier skeletal animation** (`docs/HANDOFF-soldier.md`).
- **Round-robin cost map with n≥5 pairs per arm** in a quiet GPU window (rule 45 in `AGENTS.md`).

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
├── main.rs            # Desktop thin binary entry (invokes steel_front::run_steel_front_desktop)
├── lib.rs             # Core engine / GameApp event loop / input / render loop / Android android_main
├── bin/
│   └── rdv.rs         # Lightweight UDP rendezvous relay for NAT punch & room resolution
├── ui.rs              # HUD, menus, settings, Noto Sans SC trimmed CJK glyph rasterisation
├── audio.rs           # Procedural audio synthesis/mixing (AudioSink abstraction)
├── audio_out.rs       # Native audio backends (Windows waveOut / Linux ALSA)
├── config.rs          # Config persistence (atomic write + fault-tolerant load)
├── net.rs             # UDP client/server snapshot sync & protocol
├── asset_source.rs    # Read-only asset source abstraction (FsSource / MemSource / AAssetManager)
└── engine/
    ├── renderer.rs    # Vulkan rendering root entry & types
    ├── renderer/      # 19 modularised render submodules (geometry, instances, pipelines, swapchain, PT, etc.)
    ├── game.rs        # Game logic root entry & types
    ├── game/          # 16 modularised game logic submodules (session, player, weapons, ballistics, AI, waves, etc.)
    ├── meshgen.rs     # Procedural mesh generation (rounded box/frustum/sphere/torus/normals)
    ├── weapons.rs     # Weapon framework (Firearm / ProjectileWeapon / WeaponRack)
    ├── weapon_data.rs # 35-firearm V3.0 data table (ALL_WEAPONS)
    ├── guns/          # Per-weapon procedural modelling pure functions
    ├── ai.rs          # Tactical AI (A* / state machines / cover selection)
    ├── ai_command.rs  # Company/platoon/section hierarchy (3x3, reports, commander)
    ├── camera.rs      # Three-mode camera (first-person / orbit / flight)
    ├── physics.rs     # Player & rigid-body physics with precise Y-axis support collision
    ├── map.rs         # Zero-dependency hand-written TOML parser / level loader
    ├── city.rs        # Hand-authored modern city layout (55m grid / zero coplanarity)
    ├── procedural.rs  # Procedural texture baking (city ground / skins)
    ├── cpu.rs         # CPU topology detection & thread affinity (Windows / Linux / Android)
    ├── lighting.rs    # Directional / point light & shadow uniforms / static sky light
    ├── cjk_glyphs.rs  # Noto Sans SC trimmed bitmap glyph table (1,595 codepoints)
    ├── simd.rs        # SIMD dispatch and explosion shockwave speedup
    ├── gpu_caps.rs    # GPU capability enumeration
    └── ...            # objective / window etc.
android/               # Android native Gradle project (game-activity / arm64-v8a)
desktop/               # Linux desktop icon and .desktop launcher
launcher/              # Zero-dependency Win32 GUI launcher (update / shortcut / wallpaper)
build.rs               # WGSL → SPIR-V compile at build time (naga 30)
assets/                # SPIR-V shaders, fallback textures, map TOMLs, GLB models
scripts/               # Cross-platform smoke / package / probe / debug scripts (PowerShell + Bash)
tools/                 # Automated audit guards (CJK coverage / panic sweep / battle tally / sight alignment)
```

---

## Documentation Index

| Document | Purpose | When to read it |
|---|---|---|
| [AGENTS.md](./AGENTS.md) | **Single source of truth for engineering constraints**: doctrines A–H, open items, lessons, acceptance red lines | Must-read before taking over; check before every change |
| [docs/PROGRESS.md](./docs/PROGRESS.md) | **Per-round progress and handover history** (each round with failing tests → fixes → criteria) | When looking up "what was done last / what to do next" |
| [docs/pitfalls.md](./docs/pitfalls.md) | **Repository of 162 hard-won pitfalls** (distilled from 283 fixes across 1,334 commits; 65% on silent failures and lying gates) | Must-read before writing gates, changing pipelines, or debugging elusive bugs |
| [docs/linux-native.md](./docs/linux-native.md) | Native Linux development and verification guide, Wayland & hybrid GPU debugging (commands in §10–§15) | Before developing or verifying on Linux |
| [docs/HANDOFF-mobile.md](./docs/HANDOFF-mobile.md) | Mobile (Android / iOS) porting progress, architecture, and handover notes | When developing or debugging the Android target |
| [Large-Battlefield Firearm Design V3.0](./docs/大战场枪械设计V3.0.txt) | Full data table and design basis for the 35 weapons | When changing weapon numbers or adding a firearm |
| [LICENSE](./LICENSE) | Licence and commercial grant terms | Before redistribution or commercial use |
| [THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md) | Per-item provenance check, redistribution list, two known risks | **Must read before redistributing this repo or binaries** |
| [docs/WEAPON-LICENCE-AUDIT.md](./docs/WEAPON-LICENCE-AUDIT.md) | Weapon model per-item licence check worksheet (14 slugs / verdict / leads / traps) | When auditing third-party model assets |
| `docs/progress/*.md` | Topic-split detailed progress logs (raster, ray tracing, geometry, characters & guns, AI & net & audio, engineering) | When investigating the evolution of a specific subsystem |
| `docs/perf-*` | Frame-rate and bottleneck benchmark archives | Before/after performance work |
| `docs/HANDOFF-*` | Historical handoff snapshots | To trace which round a decision was made in |

**Reading order**: start with `Large-Battlefield Firearm Design V3.0` and the doctrines in [AGENTS.md](./AGENTS.md) to build understanding; read [docs/pitfalls/lying-gates.md](./docs/pitfalls/lying-gates.md) before writing acceptance criteria; when touching rendering, read the matching verification logs under `docs/progress/` rather than editing code directly.

---

## Local Deployment

### Prerequisites

| Item | Requirement |
|---|---|
| OS | Windows 10 / 11 (x86_64) or native Linux (Wayland / X11). Mobile support for Android (arm64-v8a) |
| Rust toolchain | Stable `rustc` / `cargo` (2021/2024 edition); `cargo-ndk` for Android targets |
| GPU driver | Vulkan 1.3 capable (NVIDIA 512.xx+ / AMD 22.5+ / Intel current) |
| Disk | ~2 GB including `target/` build output |
| Memory | 12 GB or more advised; **at 12 GB or less only one `cargo` may run at a time** — concurrent builds stall out of memory |

### Dependencies

**All runtime dependencies are built-in OS components; nothing third-party is downloaded:**

- `ash` / `winit` / `glam` — Rust crates fetched by cargo at build time (10 direct dependencies on desktop).
- **Vulkan loader** (`vulkan-1.dll` on Windows, `libvulkan.so.1` on Linux) — supplied by the GPU driver or system.
- Audio: `winmm` (`waveOut` FFI) on Windows, `libasound` (ALSA) on Linux.
- Android target: hooks into native Activity lifecycle via `winit`'s `android-game-activity`, loading assets via `AAssetManager`.
- No C/C++/C# runtime, no Python runtime dependency, no external DLLs.

**Build-time dependencies:**

- `naga` (build-dependency only, WGSL → SPIR-V) and `rspirv`.
- Optional: `glslangValidator` — only for path-tracing GLSL, see `scripts/compile_pt.ps1` or `compile_pt.sh`.
- Optional: Blender — only when regenerating 3D assets; invoked headless, no GUI required.

### Steps

**Windows**:
```powershell
# 1) Clone and build
git clone https://github.com/Jerry-Hang/steel-front.git
cd steel-front
cargo build --release

# 2) Run (working directory must be repository root or have assets/ adjacent)
$env:RV3D_AUTOSTART = '1'
.\target\release\steel-front.exe
```

**Native Linux**:
```bash
# 1) Clone and build (see docs/linux-native.md)
git clone https://github.com/Jerry-Hang/steel-front.git
cd steel-front
cargo build --release

# 2) Run (wrapper script configures Wayland/KWin environment)
./SteelFront.sh
```

**Android**:
```bash
# Requires Android NDK, build cdylib with cargo-ndk
cargo ndk -t arm64-v8a build --release
```

### Verification and Self-Checks

```bash
# Unit tests (pure logic, never touches the GPU). The 0-warning gate is `cargo build --release`.
cargo test --release
cargo build --release

# Smoke verification (PostMessage / headless event injection, never steals focus or grabs the cursor)
# Windows:
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run_smoke_pm.ps1
# Linux:
./scripts/smoke_linux.sh
# Criterion = vuid==0 and panics==0 and killed>=1 (no fps threshold)

# Production panic check
python3 tools/prod_panic_sweep.py

# CJK glyph coverage check (verifies all source CJK chars exist in the glyph table)
python3 tools/cjk_cover_check.py

# Battle + intel invariants (own_deaths + Sum(strengths) == roster, camp score == enemy deaths)
python3 tools/battle_tally_check.py logs/llmbattle.log.err
```

⚠ `scripts\run_gameplay_smoke.ps1` is a **deprecated historical record** (SendInput injection cannot work on this machine); do not follow it.

### Importing External 3D Assets

```powershell
# Weapons: triage → normalise (apply transforms / join to one mesh / bake textures into
# vertex colour / decimate / export dense buffers) → install under each weapon's key
& "blender.exe" --background --factory-startup --python tools\blender\prep_guns.py -- --in "D:\my\models"
python tools\install_guns.py

# World props: change the generator → regenerate → weld (vertex count IS the frame rate, rule D)
& "blender.exe" --background --python tools\blender\build_city_kit.py -- <out_dir>
& "blender.exe" --background --python tools\blender\weld_props.py -- <in.glb> <out.glb>
& "blender.exe" --background --python tools\blender\preview_glb.py -- <in.glb> <out_prefix>
```

### Packaging a Release

The repository provides automated cross-platform packaging scripts:

- **Windows**: run `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package_release.ps1` to compile and generate `dist\steel-front-<tag>.zip` with the release executable, `assets/`, and launch scripts.
- **Linux**: run `./SteelFront.sh package` (invokes `scripts/package_release.sh`) to generate a packaged archive.
- Player entry points are `SteelFront.bat` (Windows) and `SteelFront.sh` (Linux).

---

