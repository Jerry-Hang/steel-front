# SIMD 指令级 A/B 实测（2026-08-13）

## 目的
验证现有 SIMD 优化（AVX-512 / AVX2 / AVX / SSE4.2 / 标量 五档）是否有效、效率提升多少。
新增 `RV3D_FORCE_SIMD=avx512|avx2|avx|sse4.2|scalar` 环境变量可强制锁定任一档位（三处共用：
`simd::shockwave_pressure`、`renderer::cull_spheres_dispatch`、`morph_heights_dispatch`；仍要求硬件支持，
非法值/不支持档位告警并回退自动选路）。

## 测试环境
- CPU：AMD Zen4（Ryzen 8940HX 类，16C/32T，`avx2=true avx512=true`），WSL2 Ubuntu
- 隔离微基准：`cargo test --release shockwave_path_microbench` / `simd_cull_microbench`
  （65536 元素 × 200 轮，单线程 `--test-threads=1`，无渲染并发，`black_box` 防优化）
- 游戏内对照：`RV3D_FORCE_SIMD=<档> RV3D_EXPLOSION_SIM=1 RV3D_STRESS_AI=1 RV3D_BENCH_PITCH=-10`
  60s/档，1280×800，记录 `simd:` 突发加速比 + `cull_us/fps/cycle_us/ai_us` + 进程 CPU 占用采样

## 隔离微基准（权威数据）

### ① 视锥剔除（65536 实例，`cull_spheres_*`）
| 档位 | us/轮 | 加速比 vs 标量 |
|---|---|---|
| scalar | 798 | 1.00× |
| sse4.2 | 249 | 3.20× |
| avx512 | 53 | **15.06×** |
| avx2 | 49 | **16.29×** |
| avx | 50 | **15.96×** |

→ **剔除 SIMD 极有效**：宽 SIMD 档约 15–16×，SSE4.2 也有 3.2×（2008 年后全平台兜底）。

### ② 爆炸冲击波压力场（65536 点，`shockwave_pressure_*`，AoS `[f32;3]` 12B 步长）
| 档位 | us/轮 | 加速比 vs 标量 | 取数策略 |
|---|---|---|---|
| scalar | 55 | 1.00× | 直接标量 load |
| sse4.2 | 67 | 0.82× | 标量 load 转置 + 4 宽向量（无 gather） |
| avx | 33 | **1.67×** | 标量 load 转置 + 8 宽向量（无 gather） |
| avx2 | 66 | 0.83× | `vpgatherdd` 硬件 gather（8 宽） |
| avx512 | 60 | 0.92× | `vpgatherdd` 硬件 gather（16 宽） |

→ **gather 型内核负收益**：Zen4 上 `vpgatherdd` 是微码实现，取数开销淹没向量运算收益，
AVX2/AVX-512 反而比标量慢；唯一有效的是无 gather 的 AVX 路径（1.67×）。SSE4.2 同为转置策略
但 4 宽不足以抵消转置开销。游戏内突发（含渲染并发噪声 ±30%）方向一致：avx 1.40×、
avx2 0.70×、avx512 0.82×、sse4.2 0.79×。

### ③ 地形 LOD morph 高度（65536 点）
| 档位 | us/轮 | 加速比 |
|---|---|---|
| 全部档位 | 6 | 1.00× |

→ 中性：6µs 属内存带宽瓶颈，SIMD 无差别（本就不是优化点）。

## 游戏内对照（60s/档，128 NPC 压力模式，固定视角）
| 档位 | fps p50 | cull_us p50 | cycle_us p50 | CPU 占用% |
|---|---|---|---|---|
| avx512 | 181 | 649 | 4794 | 73.2 |
| avx2 | 182 | 642 | 3566 | 73.9 |
| avx | 194 | 590 | 3458 | 73.9 |
| sse4.2 | 187 | 674 | 3538 | 80.6 |
| scalar | 195 | 594 | 3259 | 81.1 |

- `cull_us` 是"剔除+压缩+GPU 上传"全程：上传 ~500µs/帧（HOST_VISIBLE 写 17761 实例）占大头，
  剔除算力（SIMD ~50µs → 并行 ~10µs；标量 ~800µs → 并行 ~90µs）被掩盖，五档持平属预期。
- fps 五档 181–195 均受 present（dzn/WSLg ~2ms）主导，CPU 侧差异不体现为帧率差异。
- `ai_us` 不可比（各轮 NPC 存活数随机 8–102，污染 AI 耗时对比）；CPU 占用均 ~73–81%（dzn 呈现线程）。
- 全部 5 档 `bitwise_eq=true`、VUID=0、逐位一致无回归（272 tests 全绿）。

## 结论
1. **剔除 SIMD 是当前最大的有效优化**：约 15–16×（宽 SIMD）/ 3.2×（SSE4.2），
   128 NPC 压力场景下单线程剔除时间从 ~800µs 降到 ~50µs。
2. **冲击波/爆炸 AoS gather 内核是负优化**：AVX2/AVX-512 硬件 gather 在 Zen4 慢于标量，
   当前默认选路（avx512）跑的是最差档之一；无 gather 的 AVX 路径才是正解（1.67×）。
3. **后续建议（低风险高收益）**：把 `shockwave_pressure_avx2/avx512` 改为与 `_avx` 相同的
   "标量 load 转置 + 宽向量"策略（去掉 `vpgatherdd`），预计从 ~0.85× 提升到 ~1.7×；
   该改动不影响剔除（剔除走 SoA 无 gather，已是最优）。

## 复现
- 微基准：`cargo test --release shockwave_path_microbench -- --nocapture --test-threads=1`
  `cargo test --release simd_cull_microbench -- --nocapture --test-threads=1`
- 游戏内对照：`/bin/bash /tmp/run_simd_bench.sh <档位> 60`（日志 `/tmp/simd_bench_<档>.log`）
