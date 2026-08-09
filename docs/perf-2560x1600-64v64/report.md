# 钢铁前线 · 2560×1600 + 64v64 全压力基准报告（2026-08-09 修正版）

> 2026-08-09 · AMD Ryzen 9 8940HX（16C/32T）+ RTX 5060 Laptop 8GB GDDR7 + WSL2/dzn（Vulkan-on-D3D12 转译）
> 配套原始日志与复现脚本见同目录（`game-*.log` / `hw-*.log` / `win-gpu-*.log` / `bench2560.sh` / `hw_mon.py` / `key_bot.py`）

## 0. 摘要（TL;DR）

- **真·2560×1600（笔记本原生）+ 64v64（128 NPC）实测 fps p50≈42**（avg 41.7 / max 46 / min 34）。
- 对照 1280×800 同视角同负载：fps p50≈194。**分辨率×4 → 帧率 -78%**。
- 瓶颈两级跳变：1280×800 时 present 转译层占 frame 64.6%（唯一墙）；2560×1600 时
  **CPU 主线程单核 86–94% 成为新墙**（cull+record+submit+update ≈ 3.4ms/帧），present 4.5ms（59%）次之。
- **GPU 依然远未吃饱**：2560×1600 下利用率仅 ~34%（与 1280 的 36% 相当），功耗 ~27W、显存 2.27/8.15GB。
- 前置修复（本报告数据的前提）：① 分辨率回退 bug（2560×1600 不在 RESOLUTIONS → 实际渲染 1280×800，
  旧存档的"2560 数据"全部作废）；② 鼠标 Y 方向反转（玩家"低头"实为看天 → 近档实例场"消失"，
  即所谓"低头剔除 bug"的真正根因，剔除数学本身经实测正确）。

## 1. 前情：旧存档为什么作废（重要教训）

上一版报告（`game-2560x1600-bot-confounded.log` 与旧 `game-2560x1600.log`）声称"2560×1600 与 1280×800
帧率持平（264 vs 260）"，**结论完全错误**，两个原因叠加：

1. **分辨率回退 bug**：`RESOLUTIONS` 只有 4 档（最高 1920×1080），配置 `resolution=2560x1600` 匹配失败后
   `unwrap_or(0)` 回退 1280×720，`apply_resolution` 再校正为 1280×800。旧"2560×1600"日志里
   `窗口创建成功: 1280x800` 即为铁证 —— 两轮基准其实都在跑 1280×800。
2. **相机视角失控 + 鼠标反转**：基准 bot 的后坐力把 pitch 压到 ±89°；且当时鼠标方向是反的
   （`pitch -= dy*sens`，拖下=看天），"低头"实为看天空，`visible=0` 帧数虚高。

修复后（`ui.rs` 加入 2560×1600、`camera.rs` 鼠标方向标准化、`RV3D_BENCH_*` 基准挂钩锁定视角）
重跑得到本报告的权威数据。

## 2. 测试方法

- 配置 `resolution=2560x1600` / 对照 `1280x800`（跑完自动恢复，日志确认 `窗口创建成功` 尺寸）。
- 压力：`RV3D_STRESS_AI=1`（红蓝各 64 NPC）。
- 相机锁定：`RV3D_BENCH_YAW=0 RV3D_BENCH_PITCH=-10`（每帧强制，消除鼠标/后坐力干扰；
  可见实例数两档完全一致：17966/65536，near=3016 far=14950）。
- 驱动：`key_bot.py` 仅 Space 开局 / 死亡 R 重开，不碰鼠标。
- 采样：游戏内 1Hz 探针 + WSL CPU/内存/nvidia-smi + Windows GPU Engine，每档 45s。

## 3. 实测数据

### 帧率链路（探针 p50）

| 阶段 | 1280×800 | 2560×1600 |
|---|---:|---:|
| fps | 194.3 | **42.0** |
| frame_us | 2737 | 7629 |
| present_us（占 frame） | 1767（64.6%） | 4500（59.0%） |
| wait_fence_us | 211 | 530 |
| record_us | 253 | 929 |
| submit_us | 151 | 486 |
| cull_us（含 marker/NPC 上传） | 322 | 947 |
| update_us（AI 为主） | 769 | 1029 |
| cycle_us（完整帧周期） | 4218 | 13620 |

### 硬件负载（1s 采样，p50）

| 指标 | 1280×800 | 2560×1600 |
|---|---:|---:|
| CPU 全核平均 | 4.8% | 6.2% |
| **CPU 单核峰值** | 44.2% | **86.2%（max 94%）** |
| GPU 利用率（nvidia-smi） | 36% | 34% |
| 功耗 | 29.0W | 26.8W |
| 显存 | 2.17/8.15GB | 2.27/8.15GB |
| Windows GPU Engine SUM | 38.1% | 36.1% |

## 4. 结论

1. **分辨率从 1280×800 → 2560×1600（4× 像素）帧率 -78%（194→42）**，与旧存档"持平"结论正相反。
2. **瓶颈转移**：
   - 1280×800：present 转译层占 64.6%，CPU 单核仅 44% —— present 是唯一墙；
   - 2560×1600：CPU 主线程单核 86–94%（cull 947µs + record 929µs + submit 486µs + update 1029µs），
     present 4.5ms 次之 —— **CPU 单线程链路成为新墙**。
3. **GPU 在两种分辨率下都只有 1/3 左右利用率**：wait_fence 仅 530µs（GPU 实际忙碌），
   显卡 93% 时间在等 CPU 与 present 队列 —— 5060 远未饱和，负载再大也有余量。
4. **这回答了你最初的困惑**：1280×800 卡 350fps 是 present 天花板；2560×1600 + 64v64 掉到 42fps
   是 CPU 主线程（剔除+提交+AI 单线程）打满 —— 两头都不是 GPU 算力问题。
5. 下一步优化方向明确：把剔除/提交从主线程拆出去（多线程渲染提交）或并行 AI；
   否则原生分辨率 + 大战场就是 42fps 的墙。

## 5. 修复清单（本次一并完成）

- `ui.rs`：RESOLUTIONS 增加 2560×1600（5 档），设置面板可切到原生分辨率。
- `camera.rs`：`look()`/`orbit()` 鼠标方向标准化（拖下=低头）；后坐力 `pitch -= recoil_pitch*dt`（kick 正=枪口上扬）。
- `main.rs`：新增基准挂钩 `RV3D_BENCH_YAW` / `RV3D_BENCH_PITCH`（度），每帧强制相机朝向。
- `scripts/gameplay_smoke.py`：瞄准注入方向同步新约定（冒烟 ALL-OK、kills=1、VUID=0）。
- 附带结论：**"低头剔除 bug"不是剔除 bug** —— 剔除数学（Gribb–Hartmann）经 Python 复刻与游戏内
  `pitch=+89 → visible=4` 实测逐位一致；真实原因是鼠标反转导致玩家"低头"时相机看天。
  方向修复后"低头看地"表现正常（本报告 17966 可见实例即为锁定 pitch=-10 的正常视野）。

## 6. 存档清单

| 文件 | 说明 |
|---|---|
| `report.md` | 本报告（修正版） |
| `game-2560x1600.log` / `hw-2560x1600.log` / `win-gpu-2560x1600.log` | 真 2560×1600 权威数据 |
| `game-1280x800-control.log` / `hw-1280x800-control.log` / `win-gpu-1280x800-control.log` | 同视角 1280×800 对照 |
| `game-2560x1600-bot-confounded.log` | 旧"2560"（实为 1280×800 + 相机失控）作废数据，教学用 |
| `bench2560.sh` / `hw_mon.py` / `key_bot.py` / `look_bot.py` | 基准与驱动脚本 |

## 7. 复现方法

```bash
export RV3D_BENCH_YAW=0 RV3D_BENCH_PITCH=-10
BENCH_SECS=45 RES=2560x1600 BOT_CMD=/tmp/key_bot.py /bin/bash docs/perf-2560x1600-64v64/bench2560.sh
BENCH_SECS=45 RES=1280x800 BOT_CMD=/tmp/key_bot.py /bin/bash docs/perf-2560x1600-64v64/bench2560.sh
```

日志输出到 `/tmp/perf_<res>.log` / `/tmp/hw_<res>.log` / `/tmp/win_gpu_<res>.log`。
