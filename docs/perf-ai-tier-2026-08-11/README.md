# AI 线程分层基准（2026-08-11，1280x800 + 64v64 压力模式）

> 目的：量化线程优化第 2-3 步（近/远分层双池 + 远组降频）在压力模式下的实际效果。
> 环境：RTX 5060 Laptop + Ryzen 9 8940HX（WSL2/WSLg/dzn），release 构建。
> 方法：`RV3D_STRESS_AI=64`（128 NPC）+ `RV3D_BENCH_YAW=-90 / RV3D_BENCH_PITCH=-10`
> 固定视角（免 bot 干扰）+ `key_bot.py` Space 开局，每轮 30s；游戏日志逐秒采样
> `renderer fps=` / `game ai_us=` / `ai: npcs= near= far=`。

## 结论速览

- **分层生效**：128 NPC 中 near p50≈41-42（走 scene_pool=CCD0/P 核）、far p50≈85（走
  ai_pool=CCD1/E 核）——约 2/3 AI 负载从主线程所在簇剥离，主簇只留玩家交互 NPC。
- **AI 非瓶颈**：`ai_us` p50≈385µs（128 NPC 全量状态机 + A* + O(n²) 目标预选），
  仅占 16.6ms 帧预算的 2.3%；fps p50≈274（1280x800，受 dzn present 限制）。
- **降频收益≈0（压力模式）**：A/B（`RV3D_AI_DECIMATE=off` 对照）ai_us p50 385→403µs、
  ai_avg 413→411µs，几乎无差。原因：压力模式两军 300m 接火，远 NPC 几乎全部处于
  Chase/Attack（红线：攻击态/接火必须每帧步进）→ 降频触发面极小。降频保留为
  **防御性优化**：普通波次 / 大战场脱战 NPC 场景生效，开销为零，无害。
- **无回归**：VUID=0、panic=0；fps 与历史 128 NPC 基准（~250-280）持平。

## 数据表（30s，release）

| 指标 | 降频开 | 降频关（A/B 对照） |
|---|---|---|
| fps avg / p50 / p95 | 275 / 274 / 289 | 274 / 274 / 292 |
| ai_us avg / p50 / p95 / max | 413 / 385 / 669 / 744 | 411 / 403 / 635 / 698 |
| npc / near / far (p50) | 128 / 41 / 85 | 128 / 42 / 85 |
| VUID / panic | 0 / 0 | 0 / 0 |

## 复现

```bash
BENCH_SECS=30 BENCH_LOG=/tmp/stress_bench_dm.log RV3D_AI_DECIMATE_OPT= /bin/bash /tmp/run_stress_bench.sh
BENCH_SECS=30 BENCH_LOG=/tmp/stress_bench_nodm.log RV3D_AI_DECIMATE_OPT=off /bin/bash /tmp/run_stress_bench.sh
python3 - <<'PY' ... # 分析脚本见会话记录
PY
```

## 后续

- 分层收益的进一步量化：需要与"全 ai_pool（第 2 步前）"同环境 A/B（主簇 CPU 占用
  分布对比），当前 ai_us 数字只能证明 AI 非瓶颈，主簇减负需 Windows 侧 CPU 采样验证。
- 降频若要在压力模式获得收益，需放宽红线（如远组 NPC 状态机降频但攻击结算保持），
  与用户"接火必须每帧"决策冲突，暂不执行。
