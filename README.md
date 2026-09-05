# 钢铁前线 · Steel Front

> 架空历史 · 2020 年代 · 大规模战场 FPS  |  Rust + Vulkan 自研引擎（ash / winit / glam），零第三方游戏依赖

---

## 🌐 语言 Language

**📖 中文（当前）** · [English](#steel-front-english)

---

## 一句话

从零构建的现代大战场射击游戏引擎与玩法原型：程序化渲染、程序化建模、程序化音效、程序化地图；同时支持**外部资产导入**（glTF GLB 模型 + Blender 无头批处理管线），用户可用 Blender 或直接下载的模型经脚本规范化后接入。无 Unity/Unreal 黑盒。

---

## 项目状态（2026-09-05 快照）

| 维度 | 状态 |
|---|---|
| 单元测试 | `cargo test --release` **全绿 462 个 `#[test]`**；存量警告 50 条（含 3 条本轮新增的"尚未接线"死代码，刻意不用 `allow` 压掉，理由见[验证纪律](#验证纪律)） |
| 渲染主路径 | **VK_EXT_mesh_shader 网格着色器**（RTX 5060 真机启用）。⚠ 曾于 09-02 被硬编码关闭、**09-05 恢复**——当时的"mesh 地面全黑"实为一个与管线无关的描述符绑定缺失，详见[渲染里程碑](#渲染与画面里程碑) |
| 传统顶点管线 | **已冻结**：仅作为缺 `VK_EXT_mesh_shader`（WSLg / dzn / 老卡）时的兼容回退，不接受功能开发、不做双份维护 |
| 深度遮挡 | 主管线**已开启深度测试**（此前为 false，即整个世界无遮挡、楼穿楼）；第一人称枪模拆到独立 `gun_pipeline`（不测也不写深度）以保持恒可见 |
| 世界建模 | **GLB 道具已上屏**：24 件原创资产套件（5 种建筑变体 / 树 / 集装箱 / 泽西护栏 / HESCO / 铁丝网 / 电线杆 / 残骸车 / 5 层预制板楼 / 据点底盘与阵营旗），全城 288 处摆放、其中 80 处建筑；碰撞由网格实测包围盒自动推导 |
| 武器系统 | 35 把现代枪械（V3.0 数据表：初速/下坠/散射/衰减/部位倍率/开火模式/ADS）；**13 把已换用外部规范化 GLB 模型**，其余回退 AK-12 GLB 或程序化枪模 |
| 性能 | 2560×1600 中画质、128 敌目标下实测 **≈152 fps**（道具分桶视锥剔除前 112，无道具基线 187）；剔除经逐像素差分验证**未丢失任何可见几何** |
| 大战场 | 默认红 128 vs 蓝 127+玩家（256 人）；`RV3D_STRESS_AI=0` 恢复波次模式 |
| 地图 | 手工绘制现代城市（55m 街区网格、沿街围合 + 内院）：写字楼/仓库/公园/商铺/哨卡/停车场/围墙；关卡亦可由 TOML 描述（`RV3D_MAP` / `RV3D_MAPS`，F5 热重载） |
| 联机 | **局域网/同机双人可玩**：服务器权威 + 快照插值 + 断线重连 + 协议版本握手 + **NAT 中继**（rdv.exe 房间名直连） |
| 外部资产 | OBJ / glTF GLB 导入管线；**Blender 无头处理链**（`gen_props.py` 原创建模、`prep_guns.py` 规范化外部模型、`install_guns.py` 按武器 key 安装）；`glb_survey.py` 做资产体检 |
| 音频 | winmm waveOut 原生 FFI 发声（无设备静默降级） |
| AI | 三三制指挥体系 + 火-机动交替 + 连级战位铺开 + LLM 战时指挥官（llama.cpp 零依赖接入）；线程按 CCX/能效核分层绑定 |
| 路径追踪 | ⚠ **当前停用**（`pt_enable=false`）。管线本身已打通（NEE 太阳阴影射线 + 漫反射弹跳 + spp 时域累积降噪），但**启动即 `0xC0000005` 崩溃**，已实机复现确认；根因排查详见[路径追踪](#路径追踪rt-参照视图)一节与 `AGENTS.md` |
| 许可 | **AGPL-3.0 + 附加商业条款**（开源永久免费；闭源在季度营收 < 1000 万元时自动免费）。详见 [LICENSE](./LICENSE) |
| 当前阶段 | 建模资产化迁移期 + 渲染正确性收口期 + RT 参照期 |
| 设计文档 | [大战场枪械设计V3.0](./docs/大战场枪械设计V3.0.txt) · [AGENTS.md](./AGENTS.md)（AI 交接与迭代留痕） |

---

## 渲染与画面里程碑

这一节记录"为什么现在是这样"，避免后来者把已修的当成没修、把误记的当成真因。

### 地面大面积纯黑 ≠ 没有几何（2026-09-03 定位，09-04 修复）

片元着色器无条件引用 `@group(0) @binding(9) ground_detail_tex`，而描述符集布局历史上只声明到 binding 8、写入函数也只写 1/3/5/6/7/8。**未绑定描述符采样恒返回 0**，而地面分支是乘性的（`mixed *= mix(1.0, g * GROUND_DETAIL_GAIN, gdetail)`）→ 相机周边整圈地面被**乘成纯黑**。

- 与光照、阴影、纹理内容**全部无关**，所以历史上"三重排除"（关阴影 / dump 纹理 / 开关光照）全部通过，并被误判为"没有三角形覆盖"。
- 交换链 clear color 是浅蓝 `(0.24,0.36,0.60)`，**黑色绝不可能是"露出清屏色"**——这一点当初就能直接否掉那个假设。
- 因为两条管线**共用同一个片元着色器**，同一缺陷也解释了"mesh 路径地面全黑"。**那不是两个 bug，是一个**，mesh 着色器本身无责。
- 修复：以 `R8G8B8A8_UNORM`（线性，**不能走 sRGB view**，否则 128 解成 0.214、全场暗一半）创建并绑定该纹理；纹素改半值编码以匹配着色器 ×2 增益；`GROUND_DETAIL_SIZE` 与着色器的每纹素米常数严格对齐。两侧各加一道门（`flags.w` + `g<=0` 回退），使"绑定再次丢失"退化为"没有细节层"而不是黑地。

### 主管线曾完全没有深度遮挡（2026-09-04 修复）

`init_pipeline` 的 `depth_test_enable` 是 `false`，而 mesh 管线是 `true`——即唯一在跑的 legacy 路径**没有深度遮挡**，楼与楼只按绘制顺序互相穿透。枪模对"不测深度"的依赖是真实的（第一人称武器必须恒可见），但不该让整个世界陪它放弃遮挡。**修法**：新增 `gun_pipeline`（不测深度、也不写深度），主管线打开深度测试。

### 实例 buffer 的元素数曾是三份抄写（2026-09-04 修复）

`buffer_elems` + 主管线描述符 `range` + 阴影管线描述符 `range` 是三份互相抄写的硬编码，其中**阴影那份连枪模槽都没覆盖**。只扩 buffer 不扩 range 会让 shader 对新增槽位**越界读 storage buffer**——驱动不崩、不报 VUID、不写日志，只返回全零 → 模型矩阵变零矩阵 → 顶点全塌一点 → **几何彻底消失且毫无提示**。现已收敛为 `INSTANCE_BUFFER_ELEMS = 最高槽位 + 1` 单一定义 + 编译期断言。

### 道具分桶视锥剔除（2026-09-05）

道具几何一次全量提交 80 万顶点 / 36 万三角，实测占整帧约 40%（fps 187 → 112），且 `cull_us` 仅 10µs、`wait_fence_us` 高达 5.9ms ⇒ **GPU 受限**。解法是按 40m 格分桶、逐桶做球-视锥测试，只提交可见桶的索引段（共用同一份 VBO/IBO，不重传顶点）。结果 fps 112 → **152**，且逐像素差分证明**未丢失任何可见几何**。

### 验证纪律

本项目对"改完必须自己看图"有一条硬要求，并且刻意补了一层：**凡是只有肉眼能看见的结论，必须同时有一个非视觉的佐证。**

- `tools/shot_diff.ps1`：分区像素差分，把"看起来没变"变成"全图 0.00、36 格 0 显著"这类硬数字。
- 运行时几何探针（如 `gun-orient`）：把朝向/深度这类三维判断变成可输出的数字。

这条纪律的来源是一次真实失误：本仓库曾把"枪模朝向错误"当作缺陷排查了四轮实验（换绕序、关背面剔除、换已知良好实例槽、片元插纯红探针），最后由探针证明**朝向一直是对的，是读图读错了**——0.6m 锚距下看 0.675m 长枪，近端投影远大于远端，肉眼看就是"横躺"。反面教训同样记录在案：直指地面全黑 bug 的 `dead_code` 警告，曾被前人的脚本主动 `#[allow(dead_code)]` 压掉。**所以本项目不用 `allow` 掩盖死代码警告。**

---

## 当前进度（2026-09-05）

### 已完成（近期主线）

**建模从"全程序化"迁移到"资产化"**
- `tools/blender/gen_props.py`：headless `bpy` 原创建模套件生成器（24 件），含**顶点色预览渲染自检**（两个 3/4 视角 + 正交俯视 + 小件特写），建模错误在进引擎前就能看见。
- 约定固定：1 单位 = 1 米、原点在底面中心、Blender 内 +Z 上、导出 `export_yup=True` 转 glTF Y-up、单 mesh / 单 primitive / 节点不带变换。
- `engine::props`：`PropSet` 目录扫描加载（按名排序保证下标稳定）、`PropPlacement` 位姿、**旋转盒 AABB 闭式解**（`|cos|·hw + |sin|·hd`，任意朝向都精确，无需量化到 90°）、`merge_binned` CPU 烘焙位姿。
- **刻意不做 `props.toml` 清单**：尺寸与碰撞一律从已加载网格实测推导。清单要重复记一遍数据，就多一条会脱同步的路。
- 街区接入：`building()` 与 `row_houses()` 改为放 GLB + **保留碰撞核但标为不可见**（`Shape::None`），物理/弹道/AI 视线一行未动；资产缺失时自动退回程序化盒。
- 不变式守卫：`invisible_cores_must_be_covered_by_a_prop` 确保每个隐形碰撞核都有网格盖住，**杜绝"无形墙"**。

**外部枪械模型接入**
- 用户下载的 14 把 GLB 经 `glb_survey.py` 体检：**14/14 都不能直接加载**（全部交错缓冲、全部带节点变换、全部无顶点色、8 把带贴图、最大一把超枪模 buffer 上限）。
- `tools/blender/prep_guns.py`：应用变换 → join 单 mesh → **把 baseColorTexture 烘进顶点色**（绕开"不读贴图"）→ 按文件实测面数反向决定减面比 → 密集缓冲导出。**730MB → 6MB**，顶点数控制在枪模 buffer 上限的 36% 以内（避免触发销毁在飞 buffer 的 device-lost 老雷）。
- `tools/install_guns.py`：按**武器 key** 安装到 `assets/guns/`，映射固化了两条易错领域知识（PP-9 是 Bizon 的原设计代号、PP-19-01 才是 Vityaz），并对无法清理的资产显式记录跳过原因。

**GLB 加载器正确性**（四个静默错读，全部无报错只出坏几何）
- `accessor.normalized` 完全不换算 → u16 顶点色会以 0..65535 当 albedo；补齐 BYTE/UBYTE/SHORT/USHORT 四类整型并按标志除以对应最大值。
- VEC3 颜色按 stride 4 寻址 → 逐顶点错位、越界后静默退回材质基色（**这使既有 `ak12_baked.glb` 的烘焙顶点色一直是错的**，实机表现为枪模呈平灰剪影；修好后细节立刻可见）。
- 不读 `accessor.byteOffset` → 多个 accessor 挤同一 bufferView 时唯一的区分手段。`ak12.glb` 的 `NORMAL` 因此**一直被读成 `POSITION`**、第二个 mesh 被读成第一个 mesh 前 988 顶点的副本。
- 缺失的 `NORMAL`/`TEXCOORD_0`/`indices` 用 `unwrap_or(0.0)` 别名到 accessor 0 → 没有 UV 的网格会拿到"位置当 UV"。改为返回空并由下游按长度兜底。
- 各配一条**无磁盘依赖的合成 GLB 回归测试**，另加一条对真实资产套件的端到端范围校验。

**联机、AI、玩法**（此前已完成，仍为现状）
- 会话协议版本握手；NAT 中继 `rdv.exe`（REG 房间名 + 打洞）；`联机主机.bat` / `联机加入.bat` 双击即用。
- 火-机动交替（攻击站 3.4s → 掩体/侧移 9m 换位）；连级目标横向铺开；NPC 出障碍推开。
- 枪摆动改为连续状态量驱动（指数包络后坐 + Hermite 平滑步态 + 屏幕等幅归一化），消除旧实现的阶跃残影与"开镜甩枪"。
- 线程分层调度：AI 按近/远组分双池、按 CCX/能效核绑定；地图生成换核执行。

### 进行中 / 待办

- **路径追踪启动崩溃**：`0xC0000005`，源码与最后可用版本逐字节相同仍复现、重启无效；`Cargo.lock` 依赖不匹配假设已被证伪。待专项排查（怀疑方向：AS 构建显存/尺寸、每帧 dispatch 与场景重建竞争、push constant 布局）。
- **玩家可能站进 GLB 楼体**：为保证"GLB 不小于碰撞盒"（宁可撞在看得见的墙上）取的 `scale = max(...)` 的已知代价，需一次实测校准。
- 地面场 GPU 侧剔除：mesh 路径把 65536 个地面 workgroup 静态全量上传、不做 CPU 剔除，是剩余的帧率天花板。
- `merge()` 已被 `merge_binned` 取代，成为死代码待删（连带 5 个测试改用超大 cell）。
- 存量警告 50 条待专项清理；`scripts/allowall.py` 等 `dead_code` 压制脚本待删除。
- PBR 贴图采样（金属度/环境反射）、GLB 嵌入贴图解析（`images`/`bufferViews` 的 `byteStride` 仍未处理）。
- `svd_63` 源文件是含两把重叠枪身的产品宣传图，需人工删一把后装为 `svd12`。

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

> ⚠ **工作目录必须是仓库根**：`assets/*.spv` 与 `assets/props/` 按**进程 cwd 相对路径**加载，从别处启动会报"打开着色器文件失败 (os error 3)"。

### 重新生成世界道具资产

```powershell
# 导出全部 GLB 到 assets/props/
& "D:\...\blender.exe" --background --python tools\blender\gen_props.py
# 出顶点色预览图自查（两个 3/4 视角 + 正交俯视 + 小件特写）
& "D:\...\blender.exe" --background --python tools\blender\gen_props.py -- --preview screenshots/kit.png
```

### 接入外部枪械模型

```powershell
# 1) 体检：看能不能被本引擎吃下（byteStride / 节点变换 / 贴图 / 面数预算）
python tools\glb_survey.py "D:\你的模型目录"
# 2) 规范化：应用变换 + join + 贴图烘顶点色 + 减面 + 密集缓冲导出
& "D:\...\blender.exe" --background --factory-startup --python tools\blender\prep_guns.py -- --in D:\你的模型目录
# 3) 按武器 key 安装到 assets/guns/
python tools\install_guns.py
```

---

## 硬件推荐配置

| 档位 | 处理器 | 显卡（Vulkan 1.3 驱动） | 内存 | 说明 |
|---|---|---|---|---|
| 1080P 最低可玩 | 6 核（i5-10400 / R5 3600 级） | RX 6500 XT / A380 级 | 8GB | 走传统顶点管线回退；波次流畅，128v128 掉帧 |
| 1080P 主流 | 8 核（i5-12400F / R5 5600 级） | RTX 2060 SUPER / RX 6600 级 | 16GB | 网格着色器主路径；128v128 顺畅（AI 分池需 8 线程+） |
| 2K 高画质 | 8 核+（i7-12700K / R7 5800X 级） | RTX 3060 Ti / RX 6800 级 | 16GB+ | 全部特效 + 128v128 |
| 4K 高画质 | 多核（i7-13700K / R9 7900X 级） | RTX 4070 及以上 | 32GB | 建议独显直连（IMMEDIATE 呈现最稳） |

> **CPU 是硬门槛**：128v128 + 并行 AI（scene_pool/ai_pool 双线程池）+ 物理/音频合成——核心数比单核频率更重要（线程按 CCX/能效核自动绑定）。
> **内存提示**：开发机为 12GB 内存时，**同一时刻只能跑一个 `cargo`**，并行构建会触发 OOM 式挂起。
> **开发验证环境**：RTX 5060 Laptop（8GB）+ Ryzen 16C/32T + 2560×1600@144Hz；当前实测 2560×1600 中画质 ≈152 fps（含 GLB 道具与剔除）。

---

## 操作说明

| 按键 | 功能 |
|---|---|
| W/A/S/D | 移动 |
| 鼠标左键 | 射击（Playing）；非第一人称窗口拖拽旋转 |
| 鼠标右键 | 开镜（ADS） |
| Tab | 相机循环（Orbit→Flight→FirstPerson）。⚠ 玩法态实测不切换，环绕取证请用 `RV3D_CAM` |
| 1/2 或 /命令窗口 | 切换武器 |
| G | 手榴弹 |
| R | 进入操控 / 死亡后复活 |
| Enter | 结算后重开 |
| ESC | 菜单 |
| Q/E | 升降（飞行/轨道） |
| B | 开火模式（单发/三连/连发） |
| N | 补给 / 下一关 |
| F5 | 关卡 TOML 热重载 |

> **鼠标捕获**：进入玩法后引擎**自行抓取光标**（不需要点击），抓取期间系统光标被锁。自动化测试务必用带 `finally` 强杀的封装（见 `scripts/cap_safe.ps1`），不要裸启动游戏。

---

## 诊断开关（`RV3D_*`）

| 变量 | 作用 |
|---|---|
| `RV3D_AUTOSTART` | 跳过菜单直接进玩法（自动化采图用） |
| `RV3D_STRESS_AI` | 大战场压力模式 |
| `RV3D_NO_PROPS` | `1` = 不上传道具几何。**性能 A/B 基线**：先量"该优化什么"再动手 |
| `RV3D_NO_SHADOW` / `RV3D_NO_GROUND_TEX` | 分别关阴影采样 / 烘焙地面纹理 |
| `RV3D_SKIN_TEX` | marker/NPC 程序化皮肤纹理（缺省关，保持冒烟基线） |
| `RV3D_GUN_SWAY` | 枪摆动总增益（`0` 完全关闭，做 A/B 判定） |
| `RV3D_SWITCH_WEAPON` | 启动后自动切到第 N 把武器（验模型） |
| `RV3D_CAM` / `RV3D_BENCH_YAW` / `RV3D_BENCH_PITCH` | 强制相机位姿，用于可复现截图 |
| `RV3D_MAP` / `RV3D_MAPS` | 指定单张 TOML 关卡 / 关卡索引（F5 热重载） |
| `RV3D_PT_LIVE` / `RV3D_PT_VIEW` / `RV3D_PT_BENCH` / `RV3D_PT_SPP` | 路径追踪相关，见下节 |

---

## 联机说明

- **RV3D_NET**：`server`（主机）/ `client`（加入），默认不启用；
- **RV3D_NET_ADDR**：默认 `127.0.0.1:27015`；局域网用主机 IP（客户端）；主机可用 `0.0.0.0:27015` 监听全部网卡；
- **RV3D_NET_RDV + RV3D_NET_NAME**（可选，异地）：先跑 `rdv.exe <bind>`，主机/加入方用同一房间名即可互连（自动打洞 + 地址发现）；
- `release_dist\game\` 内有 `联机主机.bat` / `联机加入.bat` 双击即用。

---

## Vulkan 特性说明（1.3）

- 按 **Vulkan 1.3** 编写与运行（ash 0.38 全量 1.3 头；实例/设备 1.3）；
- **VK_EXT_mesh_shader** 主路径（GPU 逐实例剔除 + 顶点生成），无扩展回退传统顶点管线；
- **VK_KHR_ray_query / ray_tracing_pipeline** 已启用（RT 参照视图，当前因崩溃停用）；
- 经典 vkRenderPass（1.0 核心子集，1.3 设备上合法）；后续升级：dynamic rendering（VK_KHR_dynamic_rendering 已入 1.3 核心）；
- 特性全表见 `docs/` 下验证文档。

### 关于引擎的着色约束（做资产前必读）

- **法线不会上传到 GPU**：顶点格式是 `pos(3) + color(3) + uv(2)` = 32 字节，着色法线全部由屏幕空间导数重建 ⇒ **只能纯平着色**。平滑着色的高模会棱面毕现；AO / 烘焙光照必须进**顶点色**。
- 由此推论：**绕序错了的面会直接黑掉而不报错**。外部建模进入引擎时统一换一次面（`props::merge` 侧），因为加载器不做 `meshgen.rs` 那套索引交换。
- 材质身份目前仍由**顶点色通道比例**嗅探（`is_glass = b > r*1.4`、`is_canopy = g>r && g>b*1.4`）；外部建模资产用 `Shape::Authored`（`tint.w = 6.0` → `flat_flag = 1.25`）退出这套程序化立面加工，否则会在建模立面上再画一层按 3.15m 间距的错位窗带。

---

## 路径追踪（RT 参照视图）

```powershell
# 单次参考帧 -> screenshots/pt_ref.bmp（自带取景，验证命中/材质/接触阴影）
$env:RV3D_PT_VIEW = '1'; cargo run --release

# 游戏内实时 PT 全景（会替换光栅画面，属调试/烘焙参照视图）
$env:RV3D_AUTOSTART = '1'; $env:RV3D_PT_LIVE = '1'; cargo run --release

# RT core 求交吞吐基准
$env:RV3D_PT_BENCH = '1'; cargo run --release
```

- `RV3D_PT_LIVE`：`0`=强制关 / `1`=强制开 / 未设=跟随 `config.pt_enable`；
- 着色器源码 `assets/rt/pt_panorama.glsl`，改动后跑 `powershell -ExecutionPolicy Bypass -File scripts/compile_pt.ps1`（glslang 编译 + 严格 spirv-val），再 `cargo build`；**改 glsl 不删 spv 会命中磁盘缓存而不生效**；
- `RV3D_PT_SPP`：累积样本数上限；**达标后自动停止 PT 派发**，帧率回到无 PT 水平；
- 降噪 = 时域累积：RGBA32F 累加线性 HDR 样本，对运行均值做 ACES + sRGB；相机/光照/场景任一变化自动清空重开。

> ⚠ **当前状态：停用**。`pt_enable` 在 `src/config.rs` 中为 `false`，置 `true` 后游戏**启动即 `0xC0000005` 崩溃**（日志末行停在 `PT-SCENE: 盒 512 个`）。已实机复现确认，且以下假设**均已被证据排除**，不要重复尝试：
> - ~~依赖版本不匹配、恢复 `Cargo.lock`~~ —— 全历史仅 5 次改动且最后一次在可用构建的上游，工作区 lock/toml 与可用版本逐字节相同；
> - ~~驱动状态、需要重启~~ —— 重启后仍复现；
> - ~~源码改动引入~~ —— 崩溃发生于代码已还原至干净基线的提交上；
> - ~~`RV3D_PT_LIVE=1` 可以打开 PT~~ —— `main.rs` 的门是 `if config.pt_enable { init_pt_resident() }`，而 **`config.rs` 的 parser 至今不读 `pt_enable` 键**，所以 cfg 文件与环境变量都开不了它，必须改代码重编。

---

## 许可

本项目采用 **AGPL-3.0 + 附加商业使用条款**，完整条款见 [LICENSE](./LICENSE)。要点：

| 使用方式 | 条件 | 费用 |
|---|---|---|
| **开源使用** | 完全遵守 AGPL-3.0（源码公开，修改亦以 AGPL-3.0 发布） | **永久免费**，与商业规模/收入无关 |
| **闭源使用（小规模）** | 闭源分发或提供闭源网络服务，且最近一个完整季度总营业收入 **< 人民币 1000 万元** | **自动免费**，无需联系 |
| **闭源使用（大规模）** | 闭源且最近一个完整季度总营业收入 **≥ 人民币 1000 万元** | **须购买书面商业授权**，否则视为侵权 |

> ⚠ **第三方素材不在本许可覆盖范围内**：`assets/guns/` 与 `assets/guns_ext/` 中由外部站点下载的枪械模型各自带独立来源许可，引入前须逐个确认是否允许再分发与商用。`assets/props/` 由本项目 headless Blender 脚本原创生成，适用上述许可。

---

## 文档索引

- [AGENTS.md](./AGENTS.md) —— **最新迭代留痕与 AI 交接日志**（本项目唯一的正式 AI 交接载体）；
- [LICENSE](./LICENSE) —— 许可与商业授权条款；
- [大战场枪械设计V3.0](./docs/大战场枪械设计V3.0.txt)；
- 渲染/光照/性能验证与历史交接：`docs/` 目录（`experiment-*` / `perf-*` / `HANDOFF-*`）；
- 工具链：`tools/blender/`（建模与规范化）、`tools/glb_survey.py`（资产体检）、`tools/glb_probe.py`（GLB 结构）、`tools/install_guns.py`（按 key 安装）、`tools/shot_diff.ps1`（截图差分）、`scripts/cap_safe.ps1`（带强杀的采图封装）。

---

# Steel Front — English

<a id="steel-front-english"></a>

**Alternate-history 2020s large-scale battlefield FPS** · self-built engine (ash / winit / glam), zero third-party game dependencies. Procedural rendering / modeling / audio / map, **plus an external-asset pipeline** (glTF GLB + headless Blender normalization — models you make or download can be brought in after a scripted cleanup pass).

## Status (2026-09-05)

- **Tests**: `cargo test --release` all green — **462 tests**.
- **Rendering**: `VK_EXT_mesh_shader` is the main path again (verified on RTX 5060). It had been hardcoded off on 09-02 because of "all-black ground on the mesh path"; that turned out to be an unbound descriptor (`binding 9`) in the fragment shader **shared by both pipelines**, fixed on 09-04. The classic vertex pipeline is now **frozen** as a fallback for GPUs without the extension.
- **Depth**: the main pipeline now has depth testing enabled (it was off — the world had no occlusion at all). The first-person weapon moved to its own pipeline that neither tests nor writes depth, so it stays visible without costing the world its occlusion.
- **World art**: 24 original GLB props (5 building variants, trees, containers, Jersey barriers, HESCO, fences, poles, a wrecked car, a 5-storey panel block, capture-point base + flag) placed across the city — 288 placements, 80 of them buildings. Collision is derived from measured mesh bounds, no manifest to drift.
- **Weapons**: 35 modern firearms (V3.0 data table); **13 now use external normalised GLB models**.
- **Performance**: ≈152 fps at 2560×1600 medium with props, after per-bin frustum culling (was 112; no-props baseline is 187). Pixel-diff verified that culling drops no visible geometry.
- **Battlefield**: 128v128 stress mode (`RV3D_STRESS_AI=1`) or wave mode.
- **Multiplayer**: LAN / same-machine 2-player — server-authoritative snapshots, reconnect, protocol version handshake, **NAT rendezvous** (`rdv.exe` room-name direct connect).
- **AI**: section-level command hierarchy + fire-and-maneuver + company objective spread + optional LLM commander (llama.cpp, zero-dep); threads pinned to CCX / efficiency clusters.
- **Path tracing**: pipeline works but is **currently disabled** — enabling it crashes at startup with `0xC0000005`; reproduced and narrowed (dependency-mismatch and driver-state theories both disproved).
- **License**: AGPL-3.0 plus additional commercial terms — see [LICENSE](./LICENSE).

## Quick Start

```powershell
$env:RV3D_AUTOSTART = '1'      # skip menu
cargo run --release            # wave mode
$env:RV3D_STRESS_AI = '1'      # 128v128 pressure mode
$env:RV3D_INSPECT = '1'        # weapon inspect (view imported models)
```

> ⚠ Run from the repository root: `assets/*.spv` and `assets/props/` are loaded relative to the **process working directory**, not the executable.

## Networking

- `RV3D_NET=server|client`, `RV3D_NET_ADDR=host:port` (default `127.0.0.1:27015`);
- Optional rendezvous: run `rdv.exe <bind>`, set `RV3D_NET_RDV` + a shared `RV3D_NET_NAME` for cross-network play.

## Hardware

- 1080p entry: 6-core CPU + RX 6500 XT / A380-class GPU (classic pipeline fallback).
- 1080p main: 8-core + RTX 2060 SUPER / RX 6600-class (mesh-shader path; ≥8 threads for parallel AI).
- 4K: 8+ cores / RTX 4070+ / 32GB — dGPU-direct recommended (IMMEDIATE present is most stable).
- **CPU is the hard gate** for 128v128: core count matters more than single-core speed.
- **RAM**: on a 12 GB machine only **one `cargo` may run at a time**; concurrent builds stall.

## Importing External Models

```powershell
python tools\glb_survey.py "path\to\models"      # can this engine eat them?
# normalize: apply transforms, join, bake textures to vertex colour, decimate, dense buffers
& "blender.exe" --background --factory-startup --python tools\blender\prep_guns.py -- --in path\to\models
python tools\install_guns.py                     # install under assets/guns/<weapon key>.glb
```

### Shading constraints that affect authoring (read before making assets)

- **Normals never reach the GPU**: the vertex format is `pos(3) + color(3) + uv(2)` = 32 bytes and shading normals are reconstructed from screen-space derivatives ⇒ **flat shading only**. Bake AO and lighting into **vertex colours**; smooth-shaded high-poly imports will look faceted.
- Consequently a wrongly-wound face renders black **without any error**. External meshes get one deliberate index flip at the merge boundary.
- Material identity is sniffed from vertex-colour ratios. Authored meshes opt out via `Shape::Authored` so the engine does not paint procedural window bands over modelled facades.

## Verification discipline

Every change must be looked at with our own eyes **and** backed by a non-visual measurement: `tools/shot_diff.ps1` (per-region pixel diff) and runtime geometry probes. This exists because a real investigation chased a "wrong gun orientation" for four experiments before a probe proved the model was correct all along — perspective at a 0.6 m anchor made a 0.675 m rifle look broadside. Conversely, the `dead_code` warning that pointed straight at the black-ground bug had been silenced by a helper script, **so this project does not use `#[allow(dead_code)]` to hide warnings**.

## License

**AGPL-3.0 with additional commercial terms** — see [LICENSE](./LICENSE). In short: open-source use is free forever regardless of scale; closed-source use is automatically free while your most recent complete quarter's gross revenue is under **CNY 10,000,000**; above that, a written commercial licence is required. **Third-party downloaded models are not covered** by this licence and carry their own source terms.

---

*更多细节（中文）见上文。*
