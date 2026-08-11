# 硬件要求与 Vulkan 特性说明（2026-08-11）

> 本文档回答三个问题：① 游戏实际使用了哪些 Vulkan API 特性；② 显卡需要支持 Vulkan 哪个版本；③ 是否使用网格着色器。最后给出游玩硬件标准（最低 / 推荐 / 最高三档）。
>
> **结论速览：传统 VERTEX+FRAGMENT 渲染管线，不使用网格着色器；实例声明 Vulkan 1.3，实际只用 Vulkan 1.0 核心特性 + `VK_KHR_swapchain`。显卡门槛：支持 Vulkan 1.3 的驱动（2016 年后桌面独显基本全满足）。**

## 一、实际使用的 Vulkan 特性

来源：`src/engine/renderer.rs`（初始化与绘制路径，游戏真实依赖）+ `src/engine/gpu_caps.rs`（硬件探测，仅日志不依赖）。

| 类别 | 内容 | 说明 |
|---|---|---|
| 实例 API 版本 | `VkApplicationInfo::apiVersion = VK_API_VERSION_1_3` | 游戏声明的最高 API 版本 |
| 实例扩展 | `VK_KHR_surface` + 平台 surface 扩展（`ash_window` 枚举：Xlib / XCB / Wayland / Win32 等） | 窗口呈现必需 |
| 实例扩展（可选） | `VK_EXT_debug_utils` | 仅验证层可用时启用 |
| 实例层（可选） | `VK_LAYER_KHRONOS_validation` | 有则启用，无则跳过 |
| 设备扩展 | `VK_KHR_swapchain` | 唯一必需设备扩展 |
| 设备特性 | `samplerAnisotropy`（物理设备支持才启用） | 各向异性过滤，可选 |
| 渲染管线 | 传统图形管线：`VERTEX` + `FRAGMENT` 两个 shader 阶段 | **非网格着色器** |
| 渲染通道 | 经典 `vk::RenderPass` + `vkCmdBeginRenderPass` | 未用 `VK_KHR_dynamic_rendering` |
| 纹理 | 1 张 2D 程序化纹理 `R8G8B8A8_SRGB` + 完整 mip 链 + LINEAR 过滤 | 地形 / 地面 / 障碍共用 |
| 描述符 | 相机 UBO + 灯光 UBO + 纹理采样 + 实例 storage buffer | 全部为 Vulkan 1.0 核心能力 |
| 交换链 | 双/三缓冲，PRESENT_MODE 支持 IMMEDIATE / MAILBOX / FIFO（`RV3D_PRESENT_MODE` 可覆盖） | |
| 顶点输入 | 顶点缓冲 + 实例化绘制（65536 实例场）、地形 3 级 LOD 索引缓冲、HUD 覆盖层 | 核心 1.0 |

**明确未使用**（`gpu_caps.rs` 只探测、不依赖，未来路线见 `docs/windows-native-vulkan-plan-2026-08-09.md`）：

- 网格着色器：`VK_EXT_mesh_shader` / `VK_NV_mesh_shader` —— 不使用，绘制全部走传统 VERTEX+FRAGMENT
- 光追：`VK_KHR_ray_tracing_pipeline` / `VK_KHR_acceleration_structure` / `VK_KHR_ray_query` —— 不使用
- 协作矩阵：`VK_KHR_cooperative_matrix` / `VK_NV_cooperative_matrix` —— 不使用
- DLSS / 超分私有扩展（`VK_NVX_*` / `VK_NV_cuda_kernel`）—— 不使用
- 几何 / 细分着色器、渲染路径无 compute 管线（爆炸/冲击波 SIMD 走 CPU）

## 二、显卡 Vulkan 版本要求

- 实例创建声明 `apiVersion = 1.3`。按 Vulkan 规范，驱动支持版本低于应用声明版本时 `vkCreateInstance` 可能返回 `VK_ERROR_INCOMPATIBLE_DRIVER`，因此**最低门槛按 Vulkan 1.3 驱动计算**。
- 实际功能需求只有 Vulkan 1.0 核心 + `VK_KHR_swapchain` + 可选 `samplerAnisotropy`。若未来要兼容更老的驱动，把 `api_version` 降到 1.0/1.1 即可，渲染代码无需任何改动（没有用到 1.1+ 的任何能力）。
- 本机实测（WSLg/dzn 转译层）：RTX 5060 Laptop 以 "Microsoft Direct3D12 (NVIDIA GeForce RTX 5060 Laptop GPU)" 枚举，报告 47 个设备扩展，`VK_KHR_dynamic_rendering` / `VK_KHR_buffer_device_address` = true，光追 / 网格着色器 / 协作矩阵 = false —— 游戏照常运行（1280×800 ≈ 250–430 fps）。
- **结论：Vulkan 1.3 驱动的 2016 年后桌面独显 / 2019 年后核显均可运行**（NVIDIA GTX 900/1000 系起、AMD RX 400/500 系起、Intel Gen9 起，需新版驱动：NVIDIA 545+ / Mesa 23.2+）。

## 三、游玩硬件标准

### 最低要求（能玩，低画质可接受）

| 项目 | 要求 |
|---|---|
| GPU | 支持 Vulkan 1.3 的独显/核显（约 2016 年后桌面独显、2019 年后核显，需新版驱动） |
| 显存 | ≥ 2 GB |
| CPU | 4 核 8 线程（AI 决策可回退串行；剔除有 SSE4.2 路径） |
| 内存 | 8 GB（游戏进程实测约 1.5 GB） |
| 目标 | 1280×720 @ 30–60 fps，NPC ≤ 24 |

### 推荐要求（流畅高帧）

| 项目 | 要求 |
|---|---|
| GPU | 中端独显：NVIDIA GTX 1660 / RTX 20 系及以上、AMD RX 5000/6000 系及以上、Intel Arc |
| 显存 | ≥ 4 GB |
| CPU | 8 核 16 线程（AI 亲和线程池 + AVX2/AVX-512 剔除、AMD 双 CCD / Intel P+E 绑核生效） |
| 内存 | 16 GB |
| 目标 | 1280×800 / 1920×1080 @ 144+ fps；64v64 压力模式流畅 |

### 最高要求（当前引擎上限）

| 项目 | 要求 |
|---|---|
| GPU | NVIDIA RTX 40/50 系或 AMD RX 7000/9000 系（AVX-512 剔除选路；未来光追/DXR 走 NVIDIA，WSL2 下需 Windows 原生 Vulkan 或 CUDA 直通） |
| 显存 | ≥ 8 GB |
| CPU | 16 核 32 线程（主簇渲染 + 次簇 AI 并行池全用上） |
| 内存 | 32 GB |
| 显示 | 2560×1600 @ 240 Hz 及以上 |

> 注：当前引擎瓶颈不在 GPU——2560×1600 + 128 NPC 时 GPU util 仅 ~27–30%，帧时大头是 dzn/WSLg 呈现层（present_us ≈1.1–3 ms）。顶配硬件在 Windows 原生 Vulkan 呈现路径落地前，帧率提升有限（见 `docs/windows-native-vulkan-plan-2026-08-09.md`）。

## 四、实测性能参考（2026-08-09/10，RTX 5060 Laptop + Ryzen 9 8940HX，WSLg/dzn）

- 1280×800 无帧率上限：270–433 fps（6→48 NPC）；`present_us ≈ 1.1 ms` 占帧时 55–70% → 瓶颈在 dzn/WSLg 呈现层
- 1920×1080：358→111 fps（分辨率提升后 GPU 才饱和）
- 2560×1600 + 64v64（128 NPC）：p50 ≈ 264 fps，GPU util ~27–30%，显存 2.16/8.15 GB，CPU 单核峰值 ~47%
- WSL2 的 Vulkan 路径（dzn）无法访问 RT Core / Tensor Core / DLSS；Windows 原生 NVIDIA 驱动全支持
