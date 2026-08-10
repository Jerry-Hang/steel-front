# Windows 原生 Vulkan 迁移规划 — 2026-08-09

> 背景结论（已实测）：WSL2 dzn（Vulkan-on-D3D12 转译）下 `present_us≈3ms` 占帧 55–70%，
> GPU util 仅 ~30%，MAILBOX/IMMEDIATE 无差异；dzn 下 RT/DLSS/协作矩阵扩展全 false。
> 唯一根治路径 = Windows 原生 Vulkan（NVIDIA RTX 5060 Laptop 驱动全支持）。

## 1. 迁移动机与量化目标
- 现状：present_us 3ms（dzn 转译固有，CPU 优化已到平台极限）；GPU util 30%
- 目标：present_us 3ms→<1ms（帧率瓶颈解除）；GPU util 显著提升；解锁 RT/超分/协作矩阵实验
- 附加收益：WSL 内被禁的能力（VK_KHR_ray_tracing_pipeline / acceleration_structure / ray_query、
  cooperative matrix、DLSS 私有扩展）在 Windows 原生全部可用

## 2. 工具链方案
- Rust stable MSVC + Windows Vulkan SDK + ash（现 0.38）；winit 0.30 Win32 后端，surface 走 ash-window（已依赖）
- WGSL→SPIR-V：build.rs 现用 naga 生成 assets/*.spv，产物跨平台可直接复用，build.rs 无需平台分支
- 测试：`cargo test` 原样可跑（纯逻辑）；net.rs 是纯 std UDP 可原样跑；audio.rs 需确认后端（ALSA→wasapi 待评估）
- 冒烟：现有 `scripts/run_gameplay_smoke.sh` 是 bash + Xvfb，Windows 需新写 PowerShell 等价脚本，
  或阶段 A 期间继续在 WSL 跑测试 + Windows 跑游戏对照日志

## 3. 按文件改动地图
- `src/main.rs`：winit 事件循环无大改；surface 创建按平台分支
- `src/engine/renderer.rs`：实例化/物理设备/设备扩展列表按 `target_os` 分支（Windows 追加 RT/协作矩阵扩展）；
  swapchain/present 模式直接 MAILBOX；swapchain 重建逻辑复用现有；其余渲染路径（剔除/morph/实例场）零改动
- `src/engine/gpu_caps.rs`：探测代码复用，Windows 下期望结论反转（RT/DLSS 变 true），日志对照
- `src/audio.rs`：Windows 播放后端（wasapi FFI 或先保持 headless 混音，评估后定）
- `Cargo.toml`：不新增第三方依赖；如需 windows API 尽量走 FFI 或 std
- 双路径策略：Linux+dzn 现状勿回退，`cfg(target_os = "windows")` 隔离新路径

## 4. 分阶段验收
- 阶段 A（先做，1–2 天）：Windows 原生 swapchain+present 跑通，复用性能日志三行
  （renderer/game/cam），对照 present_us / GPU util；验收：present_us 显著下降、帧率不再受转译层限制
- 阶段 B（~1 周）：RT opt-in——ray query/加速结构做阴影/AO 示例；`cfg(target_os="windows")` 隔离，
  保持 WSL 路径可编译
- 阶段 C（评估）：DLSS（VK_NVX_*/VK_NV_cuda_kernel）与协作矩阵；备选 CUDA 直通
  （`/usr/lib/wsl/lib/libcuda.so` 已实测存在，Tensor Core 可编程）

## 5. 风险与明确边界
- WSL 内无法验证：真机运行、驱动行为、RT 正确性、DLSS 画质 → 阶段 A 必须 Windows 真机执行
- 回归策略：每阶段 `cargo test` 全绿 + 冒烟等价；Windows 冒烟脚本单独立项
- 本机 12GB 内存约束不变：一次只跑一个 cargo；Windows 侧开发建议在真机/另配环境进行
