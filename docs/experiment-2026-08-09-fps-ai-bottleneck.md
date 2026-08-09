# 钢铁前线 · 2026-08-09 性能与 AI 实验复盘报告

> 这是一次"问题驱动的实验"的完整复盘：用户连续抛出 FPS/瓶颈/指令集/平台适配/微架构猜想等一串问题，
> 全部落地为代码与实测数据后，整理成单文件存档。目的是给未来的我和未来的 AI 一份"别再犯的傻问题"清单。

## 0. TL;DR

- **1280×800 下的 ~350fps 帧率天花板 = WSL2 dzn/WSLg 转译层的 present 队列**，不是 CPU、GPU、内存或显存带宽的问题。
- 实测证据：`present_us≈1.1ms` 占 frame_us 的 55–70%，game update 仅 6–18µs，GPU Engine SUM 仅 38–42%。
- 把分辨率提到 1920×1080 后 GPU 立刻饱和（fps 358→111），说明低分辨率下负载太小喂不饱显卡。
- **WSL2 的 Vulkan 路径无法调用 RT Core / Tensor Core（协作矩阵）/ DLSS**（Mesa dzn 未实现 DXR 映射）；要上光追需迁 Windows 原生 Vulkan 或走 CUDA 直通。
- SIMD 五级选路（AVX-512 > AVX2 > AVX > SSE4.2 > 标量）与 AVX-512 启用策略已实装，`renderer.rs` 内已有标注注释。
- AI 战术逻辑（角色分工/协同冲锋/躲避/偷袭绕背）与 64v64 压力模式均已落地。
- 本实验最大的"傻问题"：**Codex 环境没有 DeepSeek 模型**，且多智能体并发上限由 harness 决定，不由 CPU 核数决定。

---

## 1. 实验背景

用户一进来提出三件事，随后逐步展开成一场持续两天的性能与 AI 实验：

1. 多智能体（subagent）为什么只开 3 个？想开 6–12 个，并指定"DeepSeek V4 Flash"模型。
2. 1280×800 只有 ~180fps，显卡利用率不到 20%，要求认真对待。
3. 授权"代码规范且无隐性问题就无脑推送"。

随后追加的问题链：

- 移除 300 帧上限，测渲染瓶颈到底在哪。
- 350fps 时 GPU 45–50%、CPU 各核 <75%、内存 12/16GB 未满、GDDR7 带宽充足，为什么跑不满？
- 要求写一档"性能日志监测"，逐段探测帧率链路。
- WSL2 外接的是 NVIDIA RTX 5060 Laptop，不是 AMD 610M 核显，注意甄别。
- CPU 亲和：AI/地图生成丢第二 CCD，主线程绑第一 CCD；Intel 大小核策略。
- 指令集：Zen4/5 上 AVX-512，AVX2 保留作回退，再退 AVX/SSE4.2。
- renderer.rs 到底有没有 AVX-512？没有就加，有就用注释标明。
- 苹果/ARM：Metal 后端、NEON、大小核调度要不要做？
- 下一步：AI 调优、64v64 大战场压力测试、让 AI 不再像方块。
- 微架构猜想：Zen4 靠 2×256 单元拼 512，能不能发两条 256 指令单周期并行，代替两周期的 512？

---

## 2. 问题清单与逐条解答

### Q1. 为什么只开 3 个 AI 分身？想开 6–12 个，用 DeepSeek V4 Flash

**事实**：本环境的 subagent 工具有并发上限，由 harness 配置决定，与 CPU 核数/内存无关（7945HX 再强也改变不了这一点）。本会话实际可见 5 个子代理（Lorentz / Darwin / Ohm / Newton / Helmholtz）。

**我的看法**：多开确实有价值（并行压测、并行审查），但"数量"不是目标——3 个能干完的活开 12 个只会互相踩文件、浪费上下文。按需开、给明确边界任务即可。

**关于模型**：本环境（Codex CLI）的可用模型只有 OpenAI 系（gpt-5.6-sol / gpt-5.6-terra / gpt-5.6-luna / gpt-5.5 / gpt-5.2）。**不存在"DeepSeek V4 Flash"模型，Codex 也没有接入 DeepSeek 的接口**，指定第三方模型是无效操作。同时也不用担心"ChatGPT 欠费"——环境内模型走 Codex 订阅，没有第三方按次计费。这是本次实验最该记住的一条。

### Q2. 1280×800 只有 180fps，显卡利用率不到 20%

**事实**：移除 300 帧上限（`MAX_FPS=0`、`FRAME_BUDGET=0`）后，1280×800 实测 270–433fps（6→48 NPC）。

**我的看法**：当时 180fps 恰好是被 300 上限内的 V-Sync 门控或节流压住的假象之一；无上限后立刻暴露真实链路。低负载场景下 GPU 利用率低是正常的——场景太小，显卡"无事可做"。

### Q3. 350fps 时 GPU 45–50%、CPU 各核 <75%、内存/带宽都够，卡在哪？

**事实（分阶段探针实测）**：

| 链路阶段 | 实测耗时 |
|---|---|
| present（dzn/WSLg 转译层 present 队列） | ≈1.1ms（占 frame_us 55–70%） |
| 事件循环 | ≈0.8ms |
| 实际渲染 | ≈0.5ms |
| game update（物理+武器+AI+音频+网络） | 仅 6–18µs |

- GPU Engine SUM ≈ 38–42%（Windows 侧 `Get-Counter '\GPU Engine(*)\Utilization Percentage'` 实测），与"CPU/GPU 均未跑满"一致。
- MAILBOX 与 IMMEDIATE present 模式无差异（`RV3D_PRESENT_MODE=immediate|mailbox|fifo` 可覆盖验证）。
- **1920×1080 对照**：fps 358→111，`wait_fence_us` 219→659 → 分辨率提高后 GPU 才饱和。

**结论**：1280×800 下的瓶颈是 WSL2 转译层的 present 排队，不是 CPU 算力、不是 GPU 算力、不是内存、不是显存带宽。想喂满 5060 就把分辨率/实例数提上去（这是 dzn 转译的固有开销，代码层面能优化的空间很小）。

### Q4. WSL2 显卡甄别：5060 Laptop vs 610M 核显

**事实**：dzn 枚举唯一设备 = **NVIDIA RTX 5060 Laptop**（LUID `0x00010bed`，vmwp 进程 engtype_3d，实测显存占用 ~1.5GB）；AMD 610M 核显（LUID `0x000122d1`）空闲 0MB。后续光追/DXR 一律走 NVIDIA 路径。

**我的看法**：WSL2 混合显卡下 dzn 默认暴露的是直通给 WSL 的那张卡，用户看到"AMD"多半是误读了任务管理器里的核显条目。甄别逻辑已固化到能力探测里，后续开发不要再混淆。

### Q5. CPU 亲和：AI/地图生成丢第二 CCD

**事实**：`src/engine/cpu.rs` 用 `sched_setaffinity`（FFI、零第三方依赖、仅 Linux 编译）做主线程亲和绑定，默认绑首簇（CCD0）。

**我的看法**：方向对，但**现阶段 AI/地图生成仍是单线程**——AI 每帧强耦合玩家状态与协同决策，跨线程需要双缓冲+同步，会破坏冒烟测试的确定性。等 AI 拆成独立 tick 再上第二 CCD，现在强行分线程是负优化。

### Q6. Intel 大小核：E 核 ≤8 只接轻任务，>8 才接 AI/地图生成

**事实**：已实现于 `cpu.rs`：Intel 混合架构主线程绑 P-core 组；E-core 数量来自 CPUID leaf 0x1A hybrid（AMD 恒 0）；E-core ≤8 只接音频等轻任务，>8 时 E-core 组才承担 AI/地图生成。AMD 侧第二簇 = 后半 CCD1。

**我的看法**：同意。E 核的"尴尬"在于它是给笔记本省电的，不是给游戏算 AI 的；小 E 核组接 AI 反而拖慢决策延迟。

### Q7. AVX-512（Zen4/5）+ AVX2 回退 + AVX/SSE4.2 兜底

**事实**：已实现五级选路（`renderer.rs`）：

`avx512f`（16 实例/批，512 位 16×f32）> `avx2`（8 实例/批）> `avx`（8）> `sse4.2`（4）> 标量兜底；
非 x86_64 平台走标量，AArch64 走 NEON。各级路径与标量逐位一致（非 FMA，有单测断言）。

**我的看法**：完全同意保留 AVX2 回退。AVX-512 收益集中在 Zen4/5；对不支持它的机器逐级回退是唯一稳妥做法。实测 `cull_us` 357–502 → 72–286。

### Q8. renderer.rs 到底有没有 AVX-512？

**事实**：有，且已有注释标明。关键位置：

- `renderer.rs:2523` `#[target_feature(enable = "avx512f")]` + 2522 注释"AVX-512 批量视锥剔除：16 实例/批"
- `renderer.rs:2964-2972` 选路注释："★ AVX-512 加速已启用（本机 Zen4 实测走 16 实例/批路径）…x86_64 分级选路"
- `renderer.rs:5002` `std::is_x86_feature_detected!("avx512f")` + AVX-512 与标量逐位一致断言

**我的看法**：之前会话已经加好了；这次复审确认存在并补充了注释说明，代码逻辑未动。

### Q9. Intel 11/12/13/14 代 AVX-512：检测到也默认关闭

**事实**：`cpu::avx512_enabled()` 已实现：

- AMD Zen4/Zen5（7000/9000 系）→ 启用（本机实测 `avx512f=true`，走 16 实例/批路径）
- Intel 11 代（Rocket Lake 0xA7 / Tiger Lake 0x8C/0x8D）→ 默认关闭（AVX-512 能效/降频差，游戏负收益）
- Intel 12 代起（model ≥ 0x97，13/14 代同）→ 防御性关闭（出厂已熔丝禁用；E-core 无 AVX-512，防止指令异常透传）
- `RV3D_DISABLE_AVX512=1` 可强制关闭

**我的看法**：同意，且理由充分。11 代（Rocket Lake）的 AVX-512 功耗/频率代价在游戏负载上是负收益；12 代起 Intel 出厂就把 AVX-512 熔丝禁用了，且混合架构下任务可能被调度到不支持 AVX-512 的 E-core 上直接 `SIGILL`/崩溃——防御性关闭是对的。

### Q10. 苹果/ARM：Metal 后端、NEON、大小核调度

**事实**（决策已记录 AGENTS.md）：

- 暂不做原生 Metal 后端：macOS/iOS 走 MoltenVK 零改动，等 iOS 商业化再评估。
- AArch64 NEON 剔除已实现：`cull_spheres_neon`（4 实例/批，vld1q/vmulq/vaddq/vcgeq），含 aarch64 门控等价断言。
- `RawCString` 别名统一字符串指针（x86_64=`*const i8`，AArch64=`*const u8`）。
- Apple Silicon 不手工绑核（macOS 无 sched_setaffinity；让系统 QoS 调度）。

**我的看法**：NEON 现在做是对的（低成本、收益直接）；Metal 后端是另一个数量级的工作量，游戏内容稳定前不碰；"船大难掉头"的担忧成立，但掉头成本最高的其实是着色器层，而 Vulkan→Metal 的着色器转换有 MoltenVK 兜底。

### Q11. 下一步该搞什么：AI 调优、64v64、AI 可视化

**事实**（全部落地）：

- `4588af9` 战术 AI：角色分工（突击/包抄/压制/掩体跃进）、低血撤退、玩家面朝偷袭绕背、左右包抄+同步冲锋（滞回 50%开/60%关）、锯齿机动躲避（追击态）+ 受击/子弹威胁侧向弹开、感知注入状态机。
- `5c27ecf` 64v64 压力模式 + 并行 AI + NPC 互射/补员 + `RV3D_NPC_SCALE` 数量缩放。
- `8b31ba6` NPC 积木人可视化 + shader 纯色阵营渲染——解决"分不清哪个是 AI、哪个是障碍物"。

### Q12. Zen4 双 256 单元：发两条 256 指令代替一条 512 是不是更快？

**我的看法（这条最有意思）**：方向对，但结论是**不能更快，别这么干**。

- Zen4 的 512 位浮点运算**本来就是前端解码后拆成 2 个 256 位 μop 执行的**（物理执行单元是 256 位）。所谓"512 跑两周期" = 两条 256 μop 各占一周期。
- 手动发两条 256 指令：指令数翻倍 → 解码带宽、乱序窗口、前端带宽被占更多；寄存器压力更大。吞吐与硬件自己拆完全一样，白亏指令带宽。
- 你的直觉"两条 256 并行单周期"恰恰就是 Zen4 执行 AVX-512 的方式——**硬件已经替你干了**。
- AVX-512 在本项目的真实收益不是"单周期浮点"，而是：单条指令 16×f32（剔除 16 实例/批 vs AVX2 的 8）、k-mask 比较掩码（免打包/解包）、嵌入广播、指令数更少。这些正是实测 `cull_us` 显著下降的原因。
- 所以：保留 AVX-512 路径、让编译器与硬件融合；**不要**在应用层把 512 拆成 2×256。

---

## 3. 实测数据汇总

| 指标 | 数值 | 说明 |
|---|---|---|
| 无上限 fps（1280×800，6→48 NPC） | 270–433 | `MAX_FPS=0` + `FRAME_BUDGET=0` |
| AVX2 剔除 `cull_us` | 357–502 → 72–286 | SIMD 化前后 |
| frame_us p50 | ≈1.4–2.0ms | 瓶颈在 present/GPU 而非 CPU |
| 350fps 时 `present_us` | ≈1.1ms（占 frame 55–70%） | dzn/WSLg 转译层固有；MAILBOX/IMMEDIATE 无差异 |
| 事件循环 / 实际渲染 | ≈0.8ms / ≈0.5ms | 探针分解 |
| game update（phys/weapon/ai/audio/net） | 6–18µs | CPU 侧几乎空闲 |
| GPU Engine SUM | 38–42% | Windows 侧 Get-Counter 实测 |
| 1920×1080 对照 | fps 358→111；`wait_fence_us` 219→659 | 分辨率提高后 GPU 才饱和 |
| GPU 甄别 | RTX 5060 Laptop（LUID 0x00010bed，显存 ~1.5GB）；610M 空闲 0MB | dzn 枚举唯一设备 |
| RT/TensorCore/DLSS 探针 | `VK_KHR_ray_tracing_pipeline`/`ray_query`/协作矩阵/DLSS 扩展全 false | WSL2 dzn 无 DXR 映射 |
| 本机 AVX-512 | `avx512f=true`，走 16 实例/批剔除路径 | Zen4 8940HX |

---

## 4. 落库实现清单（commit 索引）

**AI 战术与压力测试**

- `4588af9` feat(game): 战术AI——角色分工/协同冲锋/躲避/偷袭绕路
- `5c27ecf` feat(game): 64v64 压力模式 + 并行 AI + NPC 互射/补员
- `8b31ba6` feat(game): NPC 积木人可视化 + shader 纯色阵营渲染

**性能探针与瓶颈定位**

- `213e67d` feat(game): 移除 300 帧上限 + `RV3D_NPC_SCALE` 数量压测缩放
- `4082bde` feat(game): 视锥剔除 AVX2 SIMD 化（SoA 球心 + 8 实例/批，标量逐位一致）
- `d460954` feat(perf): 分阶段性能探针（wait_fence/acquire/present/phys/ai/audio/net）+ `RV3D_PRESENT_MODE` 覆盖
- `f34af25` docs(AGENTS.md): 性能探针与瓶颈结论快照（NVIDIA 5060 甄别/present 阻塞/分辨率对照）

**CPU 拓扑 / 指令集 / 平台**

- `8bd7354` feat(perf): CPU 拓扑检测与主线程 CCD 亲和绑定 + AVX-512 剔除路径（16 实例/批）
- `e9f8ffb` feat(perf): 剔除选路补全 AVX 与 SSE4.2 回退档（五级 SIMD，非 x86 标量兜底）
- `6703088` feat(perf): AVX-512 启用策略——Intel 11 代关闭、12 代起防御性关闭，AMD Zen4/5 启用
- `5bf7c77` feat(perf): AArch64 NEON 剔除路径（Apple Silicon/Android 通用）+ CPU 平台隔离
- `0d7bedb` docs(AGENTS.md): 记录跨平台/指令集决策（NEON 剔除、RawCString、sched_setaffinity Linux 隔离）
- `26f8882` feat(perf): GPU 硬件能力探测——光追/Tensor Core/DLSS 可用性判定日志
- `e4fe4ac` docs(AGENTS.md): 记录压力模式/并行 AI/NPC 可视化与 flat_flag 渲染约定

---

## 5. 结论

1. **1280×800 的帧率天花板是 WSL2 转译层的 present 队列**，不是 CPU/GPU/内存/带宽。探针数据（present_us 占 55–70%、game update 仅 6–18µs、GPU SUM 38–42%）与 1920×1080 对照组（fps 111、GPU 饱和）互相印证。
2. **WSL2 下光追/Tensor Core/DLSS 的 Vulkan 路径不可用**；要调用硬件 RT 必须迁 Windows 原生 Vulkan，或走 CUDA 直通（`/usr/lib/wsl/lib/libcuda.so` 实测存在）。
3. **SIMD 分级选路、CPU 拓扑亲和、AVX-512 启用策略是正确投资**，实测有量化收益（cull_us 357→72）。
4. **AI 战术与 64v64 压力模式已成形**，下一步是继续堆战术深度（掩体网络、小队队形）并保持冒烟确定性。
5. **Zen4 的"2×256 拆两条"猜想**：硬件本来就是这么执行的，应用层手动拆是负优化。

---

## 6. 给未来的我和 AI：别再犯的傻问题

1. **模型**：Codex 环境只有 OpenAI 系模型，没有 DeepSeek，也没有第三方扣费——别再指定"DeepSeek V4 Flash"，也别担心"欠费百万"。
2. **并发**：subagent 上限由 harness 决定，不由 CPU 核数决定；按任务开、给边界，不要迷信数量。
3. **硬件**：写进报告前先 `lscpu` 核实（本实验用户首条消息写 7945HX，实测是 8940HX）。
4. **WSL2 性能**：低分辨率低负载下 present 转译层就是天花板，GPU 利用率上不去是正常的；先看探针（`present_us`/`wait_fence_us`）再下结论，别只看 fps 和利用率。
5. **"CPU/GPU 都没跑满"**：先怀疑负载太小，用更高分辨率/更多实例复测，再谈优化。
6. **AVX-512**：AMD Zen4/5 放心开；Intel 11 代能效差、12 代起熔丝禁用 + E-core 无支持，防御性关闭是对的；**别手动把 512 拆成 2×256**。
7. **光追**：WSL2 dzn 无 DXR 映射；要 RT Core/DLSS 就去 Windows 原生，别再在 WSL2 里期待 Vulkan 光追。
8. **内存**：12GB 约束下一次只跑一个 cargo，禁止并行构建（AGENTS.md 铁律）。
9. **提交**：一个功能一个 commit；改 renderer/pipeline 先跑 20s 冒烟验 VUID；dead-code=0、测试全绿再推。

---

## 附录：环境快照（2026-08-09 实测）

- CPU：AMD Ryzen 9 8940HX（Zen4, Dragon Range），16C/32T，1 NUMA 节点（CPU 0-31）
- 图形：WSL2 + dzn（Vulkan-on-D3D12 转译），默认 1280×800
- GPU：NVIDIA RTX 5060 Laptop（LUID 0x00010bed，显存占用 ~1.5GB）；AMD 610M 核显空闲
- 验收基线（2026-08-08 快照）：176 tests passed、0 警告、20s 冒烟 ALL-OK（kills=1、VUID=0、fps 214.8–292.7）
