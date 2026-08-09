# 钢铁前线 · 2560×1600 + 64v64 全压力基准报告

> 2026-08-09 · AMD Ryzen 9 8940HX（16C/32T）+ RTX 5060 Laptop 8GB GDDR7 + WSL2/dzn（Vulkan-on-D3D12 转译）
> 配套原始日志与复现脚本见同目录（`game-*.log` / `hw-*.log` / `win-gpu-*.log` / `bench2560.sh` / `hw_mon.py` / `look_bot.py`）

## 0. 摘要（TL;DR）

- **2560×1600（笔记本原生分辨率）+ 64v64（128 NPC 大战场）实测 fps p50≈264**（avg 266 / max 316 / min 212）。
- **分辨率不是瓶颈**：1280×800 与 2560×1600 的 fps 几乎一致（260 vs 264），GPU 利用率同为 ~27%。
- 瓶颈仍然是 WSL2 dzn 转译层的 present 队列：`present_us p50≈1.12ms`，占 frame_us 的 ~61%。
- 硬件余量巨大：GPU 功耗仅 ~21W、温度 62°C、显存 2.16/8.15GB、CPU 平均负载 ~5%、WSL 内存 1.5/12.66GB。
- 附带抓到一个基准方法论事故：首轮 bot 后坐力把视角压到 pitch=-89°，触发**已知的低头剔除 bug**（visible=0），帧数虚高，已用固定视角 bot 重跑修正。

---

## 1. 测试方法

- 分辨率：`~/.steel_front.cfg` 写入 `resolution=2560x1600` / 对照 `1280x800`（跑完自动恢复）。
- 压力：`RV3D_STRESS_AI=1` → 红蓝各 64 NPC（128 个同时活动，互射 + 玩家可参战）。
- 视角控制：固定视角 bot（yaw=0 / pitch=-10°，不射击、不移动，死亡按 R 重开）——**杜绝后坐力把 pitch 压到 -89° 触发低头剔除 bug 污染数据**（首轮就是这么污染的，见 §5）。
- 三路采样：
  - 游戏内 1Hz 性能日志（renderer 行：fps/frame_us/cull_us/wait_fence/record/submit/present；game 行：phys/ai/audio/net；cam 行：cycle/update/render）。
  - WSL 侧 1s 采样：`/proc/stat` 每核利用率、`/proc/meminfo`、`nvidia-smi`（util/vram/temp/power）、Windows 可用内存。
  - Windows 侧 1s 采样：`Get-Counter '\GPU Engine(*)\Utilization Percentage'`（区分 vmwp/dwm 进程）。
- 每档时长 45s。

## 2. 实测数据（2560×1600 + 64v64）

### 帧率链路（游戏内探针，1Hz）

| 阶段 | p50 | avg | p95 | max |
|---|---:|---:|---:|---:|
| fps | 264.3 | 266.1 | 308.7 | 315.9 |
| frame_us | 1834 | 1959 | 3220 | 3614 |
| **present_us** | **1116** | 1195 | 1727 | 2233 |
| wait_fence_us | 186 | 214 | 420 | 542 |
| record_us | 211 | 271 | 518 | 1270 |
| submit_us | 123 | 153 | 311 | 470 |
| cull_us（AVX-512 16 实例/批） | 64 | 106 | 288 | 563 |
| update_us | 704 | 697 | 1098 | 1346 |
| ai_us（128 NPC 决策） | ~700 | — | — | 1229 |

present 占 frame 的 **60.9%**（p50），与 1280×800 对照的 62.3% 一致——present 队列是唯一显著瓶颈。

### 硬件负载（1s 采样）

| 指标 | p50 | avg | max |
|---|---:|---:|---:|
| CPU 全核平均 | 5.5% | 5.2% | 5.7% |
| CPU 单核峰值 | 45.5% | 43.1% | 46.7% |
| WSL 内存占用 | 1.5GB / 12.66GB | — | 1.6GB |
| GPU 利用率（nvidia-smi） | 27% | 27.3% | 30% |
| 显存占用 | 2.16GB / 8.15GB | — | 2.19GB |
| GPU 温度 | 62°C | 62.2°C | 63°C |
| GPU 功耗 | 20.8W | 21.2W | 24.6W |
| Windows GPU Engine SUM | 30.4% | 29.0% | 41.1% |
| vmwp 进程（WSL 渲染） | 25.2% | 24.1% | 36.1% |

## 3. 分辨率对照：1280×800 vs 2560×1600（同为 64v64 + 固定视角，各 45s）

| 指标 | 1280×800 | 2560×1600 | 差异 |
|---|---:|---:|---|
| fps p50 / avg / max | 260.0 / 259.3 / 310.7 | 264.3 / 266.1 / 315.9 | ≈ 0（+1.5%） |
| frame_us p50 | 1872 | 1834 | ≈ 0 |
| present_us p50（占 frame） | 1167（62.3%） | 1116（60.9%） | ≈ 0 |
| GPU 利用率 p50 | 27% | 27% | ≈ 0 |
| GPU 功耗 p50 | 21.6W | 20.8W | ≈ 0 |
| CPU 平均 / 单核峰值 | 4.8% / 42.9% | 5.2% / 45.5% | ≈ 0 |
| 显存 p50 | 2.17GB | 2.16GB | ≈ 0 |

**结论：2560×1600 是 1280×800 的 4 倍像素量，但帧率/功耗/利用率全部持平。** 说明当前代码在 WSL2 下：

1. 渲染负载（实例场 + 128 NPC + HUD）远小于 5060 的能力，像素量翻 4 倍都喂不满显卡；
2. 帧率天花板完全由 dzn present 队列（~1.1ms）决定，与分辨率无关；
3. 想榨干这块卡，要么迁 Windows 原生 Vulkan（去掉转译层），要么把场景负载再提 3–4 倍（更多实例/特效/后处理）。

## 4. 64v64 AI 压力评估

- 128 NPC 同时活动：`ai_us p50≈700µs/帧`，全部在**主线程单核**上（CPU 亲和绑定 CCD0）。
- NPC 交火真实发生：60s 首轮实测 NPC 从 128 降到 68；玩家不干预也会打完大半场。
- 玩家视角下 visible 实例数峰值 ~1.8 万（实例场 + NPC 积木人 + HUD 全在画）。
- **CPU 无瓶颈**：全核平均仅 ~5%。即使 NPC 翻倍（AI ~1.4ms/帧），离 3ms 帧预算仍有距离；真正的瓶颈始终是 present。

## 5. 实验教训（重要，别再犯）

### 5.1 基准必须控制相机视角

首轮 2560×1600 跑出 fps p50≈269 的"漂亮数据"，随后对照 1280×800（fps p50≈227）反而更慢，看似"分辨率越高越快"的悖论。查日志发现：

- 首轮 bot 持续点射，后坐力把 pitch 压到 **-89°（看地）**：60/68 秒 pitch < -20°，其中多数 -89°；
- 触发 AGENTS.md 已知的**低头剔除 bug**（pitch < -30° 时近档实例场被视锥剔除全灭）：61/68 秒 `visible=0`；
- 画面几乎什么都没画，帧数自然虚高；对照轮恰好视角朝战场（visible≈1.7 万），所以更慢。

修正：固定视角 bot（yaw=0/pitch=-10°、不射击）重跑，得到 §3 的干净结论。原始污染数据保留在 `game-2560x1600-bot-confounded.log` 供对照。

### 5.2 低头剔除 bug 被现场复现

`visible=0` 的时间占比（61/68 秒）就是该 bug 的直接证据。修复方向（AGENTS.md 已知问题）：排查 `extract_frustum_planes` / near plane 对视锥近裁剪面的处理，pitch < -30° 时不应把近档实例场整体剔除。

### 5.3 WSL2 下测不出 GPU 真实上限

present 队列 ~1.1ms 是 dzn/WSLg 转译层的硬成本。要量 GPU 上限（RTX 5060 全速能跑多少帧/瓦），必须 Windows 原生 Vulkan；WSL2 里 GPU 利用率 27% 就是"正常且健康"的状态，不是代码问题。

## 6. 存档清单

| 文件 | 说明 |
|---|---|
| `report.md` | 本报告 |
| `game-2560x1600.log` / `hw-2560x1600.log` / `win-gpu-2560x1600.log` | 2560×1600 主测原始日志 |
| `game-1280x800-control.log` / `hw-1280x800-control.log` / `win-gpu-1280x800-control.log` | 1280×800 对照原始日志 |
| `game-2560x1600-bot-confounded.log` | 首轮被相机视角污染的作废数据（教学用） |
| `bench2560.sh` | 基准运行脚本（`RES=` 指定分辨率、`BENCH_SECS=` 时长、`BOT_CMD=` 驱动脚本） |
| `hw_mon.py` | WSL 硬件采样（CPU/内存/nvidia-smi/Windows 内存） |
| `look_bot.py` | 固定视角驱动（yaw=0/pitch=-10°、死亡重开） |

## 7. 复现方法

```bash
# 2560×1600 + 64v64，45s
BENCH_SECS=45 RES=2560x1600 BOT_CMD=/tmp/look_bot.py /bin/bash docs/perf-2560x1600-64v64/bench2560.sh
# 1280×800 对照
BENCH_SECS=45 RES=1280x800 BOT_CMD=/tmp/look_bot.py /bin/bash docs/perf-2560x1600-64v64/bench2560.sh
```

日志输出到 `/tmp/perf_<res>.log` / `/tmp/hw_<res>.log` / `/tmp/win_gpu_<res>.log`，统计脚本 `analyze2560.py` 见 /tmp（仅会话内保留）。
