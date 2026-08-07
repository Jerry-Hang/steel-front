# 钢铁前线 (Steel Front) — 会话交接与进度总结

> 本文档是 AI 会话交接材料：记录已完成功能、当前工作状态、待办任务与关键约定。
> 新会话请先读本文 + `git log --oneline` + `git status`，再接续工作。

## 一、项目概述

二战题材 FPS 游戏引擎，Rust + Vulkan（winit 0.30），零第三方游戏依赖。
`src/engine/game.rs` 为运行时中枢，main.rs / renderer.rs 只做最小调用。

## 二、已完成功能（按提交顺序）

### 阶段 1：模块接线（8 个 commit，master 曾到 37675c1）
- feat(wiring): physics / weapons / ai / ui / lighting / audio / network
  - network 仅 `RV3D_NET=1` 启用同进程环回演示，含环回单测
- chore: annotate reserved dead-code APIs（dead-code 警告 39 → 0，预留 API 均带理由注释）
- 验收：98 测试全绿；15s 冒烟 VUID=0、fps 248–300、terrain LOD high→medium→low 正常

### 阶段 2：gameplay（6 个 commit，a→b→c→e→d→f，到 9fd83d8）
- a 鼠标视角：光标捕获 + yaw/pitch（ef3ca4a）
- b 敌人波次：NPC 血量、击杀计分（ee983d5）
- c 玩家受伤与死亡状态（0f049ce）
- e 游戏状态机：菜单/游戏中/结算 + R 重开（4aea8ae）
- d 真实 HUD：血条/弹药/分数/波次/准星/开始与结算画面（4deda87）
- f 波次递进缩放与清波奖励（9fd83d8）
- 验收：111 测试全绿、0 警告

## 三、当前工作状态（重要）

- HEAD = 0707075（chore(scripts)），docs 交接提交紧随其后；工作区干净，push 已完成。
- 最终 20s gameplay 冒烟**已通过**（方案 A，游戏侧 + harness 适配）：
  `kills=1`、4 发命中、VUID=0、fps 249–299、yaw/pitch 变化、无 panic。
- 落地 commit：
  - `feat(game)`：演示刚体挪到场地角落（距原点 >150m，不再拦截过原点射线）
  - `feat(game)`：NPC 进 Attack 站定时打位置日志 `npc: #id stand (x,y,z)`
  - `chore(scripts)`：冒烟改为 读站定日志 → 滚轮拉近 dist=1.5 → 对跖点瞄准 → 点射 6 发

### 冒烟关键机制（勿回退）
1. Orbit 射线必过原点：相机站在目标 NPC 对跖点（`direction = -C/|C|`）；
2. NPC 攻击态站定（距相机 <12m）；只选 `|C| ≤ 10.4` 的对侧 NPC——
   拉近到 dist=1.5 后对跖点距离 = |C|+1.5 < 12，目标全程保持站定；
3. 演示刚体在角落，12m 射线路径上无拦截体；点射 6 发 × 25 伤害 ≥ 100 hp。

## 四、待办任务（Next Steps）

- 全部完成：20s 冒烟通过、scripts/ 已迁入并提交、push 已执行。
- 后续若重跑：`bash scripts/run_gameplay_smoke.sh`（需沙箱外 X/Vulkan 权限）。

## 五、关键约定与环境备忘

- commit 规范：`feat(game): ...` / `feat(wiring): ...` / `chore:` / `docs:`
- 每步 `cargo build --release && cargo test` 全绿再继续；不新增第三方依赖
- 沙箱限制：绑 UDP socket / 连 X socket 需提权运行（沙箱外测试才准）
- X 注入坑：XTEST 卡键是 server 级，脚本开头先释放键 + pkill 清场
- HUD 用 5x7 位图字体，仅 ASCII，界面文案全英文
- 会话过长会压缩降智，长任务及时开新会话并先读本文件

## 六、验收数据快照

| 指标 | 数值 |
|---|---|
| 测试 | 111 passed, 0 failed |
| 警告 | 0 |
| 冒烟（飞行） | VUID=0, fps 228–300 |
| gameplay 冒烟 | ALL-OK：kills=1、hit_events=4、VUID=0、fps 249–299、无 panic |
