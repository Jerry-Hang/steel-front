# 钢铁前线 · Steel Front

> 架空历史 · 2020 年代 · 大规模战场 FPS  |  Rust + Vulkan 自研引擎（ash / winit / glam），零第三方游戏依赖

---

## 🌐 语言 Language

**📖 中文（当前）** · [English](#steel-front-english)

---

## 一句话

从零构建的现代大战场射击游戏引擎与玩法原型：程序化渲染、程序化建模、程序化音效、程序化地图；同时支持**外部资产导入**（glTF GLB 模型 + Windows 原生贴图解码），用户可用 Blender 制作模型直接接入。无 Unity/Unreal 黑盒。

---

## 项目状态（2026-08-31 快照）

| 维度 | 状态 |
|---|---|
| 单元测试 | `cargo test --release` 全绿（405 个 #[test]）；存量警告 46 条待专项清理 |
| 渲染主路径 | **VK_EXT_mesh_shader 网格着色器**（RTX 5060 真机启用），无扩展显卡回退传统顶点管线 |
| 武器系统 | 35 把现代枪械（V3.0 数据表：初速/下坠/散射/衰减/部位倍率/开火模式/ADS）；**AK-12M 可由外部 GLB 模型替代**（用户自备模型即插即用） |
| 大战场 | 默认红 128 vs 蓝 127+玩家（256 人）；`RV3D_STRESS_AI=0` 恢复波次模式 |
| 地图 | 手工绘制现代城市（55m 街区网格）：写字楼/仓库/公园/商铺/哨卡/停车场/围墙 |
| 联机 | **局域网/同机双人可玩**：服务器权威 + 快照插值 + 断线重连 + 协议版本握手 + **NAT 中继**（rdv.exe 房间名直连） |
| 外部资产 | **OBJ/glTF GLB 导入管线 + Windows GDI+ 贴图解码**；Blender 无头脚本化处理（修姿态/烘 AO/导出），AK-12 GLB（63283 顶点）已实装 |
| 音频 | winmm waveOut 原生 FFI 发声（无设备静默降级） |
| AI | 三三制指挥体系 + 火-机动交替 + 连级战位铺开 + LLM 战时指挥官（llama.cpp 零依赖接入） |
| 路径追踪 | **已打通并接入真实战场**：`VK_KHR_ray_query` 全景 PT（NEE 太阳阴影射线 + 漫反射弹跳），场景盒体与光栅化 `WorldMarker` 同源、材质同色，可作光照烘焙真值；着色器由 glslang 编译（`assets/rt/pt_panorama.glsl`），严格 `spirv-val` 通过。当前 1 spp 有颗粒噪声，默认关闭（`RV3D_PT_LIVE=0`） |
| 当前阶段 | 玩法迭代期 + 外部资产管线接入期 + RT 参照期 |
| 设计文档 | [大战场枪械设计V3.0](./docs/大战场枪械设计V3.0.txt) |

---

## 当前进度（2026-08-28）

### 已完成（近两周主线）

**外部资产导入管线（用户决策：取消全程序化限制）**
- 零依赖：OBJ 解析器（v/vt/vn/f 三角化）、glTF GLB 解析器（多 mesh 合并、componentType 感知 accessor、COLOR_0 顶点色、材质基色）、Windows GDI+ PNG/JPEG 解码（BitmapData 布局正确、BGRA→RGBA）
- 自动归一化：包围盒居中 + 长轴对齐 + 0.94m 级缩放适配枪械；共享 fp_gun_matrix（view_inv × anchor × 缩放）实现第一人称跟随
- **Blender 无头控制**（`blender --background --python 脚本.py`）：导入→材质/AO 烘焙（vertex_color_dirt）→节点净化→导出 GLB；渲染-视觉自检闭环（渲染 PNG → 看图 → 改脚本）
- **AK-12 模型实装**（Sketchfab，63283 顶点）：第一/三人称显示 + 开火（Score 460/击杀 46/128 实测），AO 细节（导轨/通风槽/防滑纹）全枪可见
- 已知进行项：枪模最终颜色校准（渲染布局/着色器微调，数据全程验证正确——见 HANDOFF-2026-08-28）

**联机完整化**
- 会话协议版本握手（Join 携带 SESSION_VERSION，不匹配 → Refuse，客户端停重试）
- NAT 中继 `rdv.exe`（REG 房间名：端口 / WHO → 声明端口直连 + 打洞）；服务器/客户端 `RV3D_NET_RDV`+`RV3D_NET_NAME` 全链路实测
- 快照 `firing` 指示 → 客户端枪口焰同步；`联机主机.bat` / `联机加入.bat` 双击即用

**AI 战斗深化**
- 火-机动交替（攻击站打 3.4s → 掩体点/垂直侧移 9m 换位循环）；连级目标横向铺开（-55/0/+55m）；NPC 移动出障碍推开（穿墙修复）

**美术立体化**
- 树冠二十面体（去纸帽子）、爆炸/自发光球状化（去方块冲击波）、围墙深色压顶、NPC 实时 Blinn-Phong 光照

**渲染与稳定性**
- Vulkan 1.3（mesh shader 主路径 + 传统回退）；IMMEDIATE 呈现 + 帧率上限（MUX 独显直连最稳）；自发光槽位区间修复（枪槽不再误判）

### 进行中 / 待办
- 枪模最终颜色校准（布局修复已完成，着色器 fade 细节收官）
- 场景道具路径（assets/props.toml + 世界空间网格管线）；PBR 贴图采样（金属度/环境反射）
- GLB 嵌入贴图（images/bufferView）→ 贴图采样；多材质节点矩阵解析

---

## 快速开始

```powershell
# 单机（波次模式）
$env:RV3D_AUTOSTART = '1'
cargo run --release

# 128v128 大战场
$env:RV3D_STRESS_AI = '1'           # 默认注入 128 对敌

# 双人联机（同机/局域网）
# 主机：
$env:RV3D_NET = 'server'; $env:RV3D_NET_ADDR = '0.0.0.0:27015'; start target\release\steel-front.exe
# 加入方（同机：127.0.0.1；局域网：主机 IP）
$env:RV3D_NET = 'client'; $env:RV3D_NET_ADDR = '主机IP:27015'; start target\release\steel-front.exe

# 中继联机（异地）：先跑 rdv.exe，主机/加入方再设 RV3D_NET_RDV + RV3D_NET_NAME

# 枪械检视（查看导入模型）
$env:RV3D_INSPECT = '1'
```

### 外部资产接入（简版）
1. 模型放入 `assets/guns/`（或 props 目录），GLB/OBJ 均可；
2. 想修姿态/烘 AO：我可用 Blender 无头脚本一键处理（见 `scripts/blender_bake.py`）；
3. 游戏启动自动加载（优先 `*_baked.glb`，缺失回退原始 + 程序化枪模）。

---

## 硬件推荐配置

| 档位 | 处理器 | 显卡（Vulkan 1.3 驱动） | 内存 | 说明 |
|---|---|---|---|---|
| 1080P 最低可玩 | 6 核（i5-10400 / R5 3600 级） | RX 6500 XT / A380 级 | 8GB | 传统顶点管线回退；波次流畅，128v128 掉帧 |
| 1080P 主流 | 8 核（i5-12400F / R5 5600 级） | RTX 2060 SUPER / RX 6600 级 | 16GB | 网格着色器主路径；128v128 顺畅（AI 分池需 8 线程+） |
| 2K 高画质 | 8 核+（i7-12700K / R7 5800X 级） | RTX 3060 Ti / RX 6800 级 | 16GB+ | 全部特效 + 128v128 |
| 4K 高画质 | 多核（i7-13700K / R9 7900X 级） | RTX 4070 及以上 | 32GB | 建议独显直连（IMMEDIATE 呈现最稳） |

> **CPU 是硬门槛**：128v128 + 并行 AI（scene_pool/ai_pool 双线程池）+ 物理/音频合成——核心数比单核频率更重要（线程按 CCX/能效核自动绑定）。
> **开发验证环境**：RTX 5060 Laptop（8GB）+ Ryzen 16C/32T + 2560×1600@144Hz；128v128 压力模式 116–240 fps（LLM 采集模式 90）。

---

## 操作说明

| 按键 | 功能 |
|---|---|
| W/A/S/D | 移动 |
| 鼠标左键 | 射击（Playing）；非第一人称窗口拖拽旋转 |
| 鼠标右键 | 开镜（ADS） |
| Tab | 相机循环（Orbit→Flight→FirstPerson） |
| 1/2 或 /命令窗口 | 切换武器 |
| G | 手榴弹 |
| R | 换弹 |
| Enter | 结算后重开 |
| ESC | 菜单 |
| Q/E | 升降（飞行/轨道） |
| B | 开火模式（单发/三连/连发） |
| N | 补给 / 下一关 |
| F5 | 关卡 TOML 热重载 |

---

## 联机说明

- **RV3D_NET**：`server`（主机）/ `client`（加入），默认不启用；
- **RV3D_NET_ADDR**：默认 `127.0.0.1:27015`；局域网用主机 IP（客户端）；主机可用 `0.0.0.0:27015` 监听全部网卡；
- **RV3D_NET_RDV + RV3D_NET_NAME**（可选，异地）：先跑 `rdv.exe <bind>`，主机/加入方用同一房间名即可互连（自动打洞+地址发现）；
- `release_dist\game\` 内有 `联机主机.bat` / `联机加入.bat` 双击即用。

---

## Vulkan 特性说明（1.3）

- 按 **Vulkan 1.3** 编写与运行（ash 0.38 全量 1.3 头；实例/设备 1.3）；
- **VK_EXT_mesh_shader** 主路径（GPU 逐实例剔除 + 顶点生成），无扩展回退传统顶点管线；
- 经典 vkRenderPass（1.0 核心子集，1.3 设备上合法）；后续升级：dynamic rendering（VK_KHR_dynamic_rendering 已入 1.3 核心）；
- 特性全表见 `docs/` 下验证文档。

---

## 文档索引

- [交接日志（AI 会话）](./docs/HANDOFF-2026-08-28.md) —— 面向下一个 AI 接手；**最新迭代留痕以 [AGENTS.md](./AGENTS.md) 为准**；
- [大战场枪械设计V3.0](./docs/大战场枪械设计V3.0.txt)；
- 渲染/光照/性能验证：`docs/` 目录（experiment-*/perf-*/HANDOFF-*）。

## 路径追踪（RT 参照视图）

```powershell
# 单次参考帧 -> screenshots/pt_ref.bmp（自带取景，验证命中/材质/接触阴影）
$env:RV3D_PT_VIEW = '1'; cargo run --release

# 游戏内实时 PT 全景（512² 上屏，会替换光栅画面，属调试/烘焙参照视图）
$env:RV3D_AUTOSTART = '1'; $env:RV3D_PT_LIVE = '1'; cargo run --release

# RT core 求交吞吐基准
$env:RV3D_PT_BENCH = '1'; cargo run --release
```

- `RV3D_PT_LIVE`：`0`=强制关 / `1`=强制开 / 未设=跟随 `config.pt_enable`；
- 着色器源码 `assets/rt/pt_panorama.glsl`，改动后跑 `powershell -ExecutionPolicy Bypass -File scripts/compile_pt.ps1`（glslang 编译 + 严格 spirv-val），再 `cargo build`；
- 已知现状：1 spp 有颗粒噪声（降噪为下一步），盒体上限 512（超出静默截断），PT 画面当前整体替换而非叠加。

---

# Steel Front — English

<a id="steel-front-english"></a>

**Alternate-history 2020s large-scale battlefield FPS** · self-built engine (ash / winit / glam), zero third-party game dependencies. Procedural rendering / modeling / audio / map, **plus external asset import** (glTF GLB models + Windows-native texture decode — Blender-made assets plug right in).

## Status (2026-08-28)

- **Tests**: `cargo test` all green (404 tests, dead-code=0).
- **Rendering**: VK_EXT_mesh_shader main path (verified on RTX 5060); classic pipeline fallback.
- **Weapons**: 35 modern firearms (V3.0 data table); AK-12M replaceable with an external GLB model.
- **Battlefield**: 128v128 stress mode (RV3D_STRESS_AI=1) or wave mode.
- **Multiplayer**: LAN/same-machine 2-player — server-authoritative snapshots, reconnect, protocol version handshake, **NAT rendezvous** (`rdv.exe` room-name direct connect).
- **External assets**: OBJ/glTF import + GDI+ PNG/JPEG decode; Blender headless pipeline (fix pose / bake AO / export); AK-12 GLB (63,283 verts) in-game.
- **AI**: 3×3 command hierarchy + fire-and-maneuver + company objective spread + optional LLM commander (llama.cpp, zero-dep).

## Quick Start

```powershell
$env:RV3D_AUTOSTART = '1'      # skip menu
cargo run --release            # wave mode
$env:RV3D_STRESS_AI = '1'      # 128v128 pressure mode
$env:RV3D_INSPECT = '1'        # weapon inspect (view imported models)
```

## Networking

- `RV3D_NET=server|client`, `RV3D_NET_ADDR=host:port` (default `127.0.0.1:27015`);
- Optional rendezvous: run `rdv.exe <bind>`, set `RV3D_NET_RDV` + shared `RV3D_NET_NAME` for cross-network play.

## Hardware

- 1080p entry: 6-core CPU + RX 6500 XT/A380-class GPU (classic pipeline fallback).
- 1080p main: 8-core + RTX 2060 SUPER/RX 6600-class (mesh-shader path; ≥8 threads for parallel AI).
- 4K: 8+ cores / RTX 4070+ / 32GB — recommend dGPU-direct (IMMEDIATE present most stable).
- **CPU is the hard gate** for 128v128: core count > single-core speed (threads pinned to CCX/efficiency clusters).

## Asset Import (brief)

1. Drop model into `assets/guns/` (GLB/OBJ);
2. Optional Blender headless pass (`scripts/blender_bake.py`) to normalize pose / bake AO;
3. Game auto-loads (prefers `*_baked.glb`, falls back to raw or procedural).

---

*更多细节（中文）见上文。*