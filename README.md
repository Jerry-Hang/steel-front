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

- HEAD = 276b395（docs 交接提交）；工作区唯一未提交改动：
  - `M src/engine/game.rs`：spawn_wave 出生点日志（冒烟辅助，是否保留待定）
- 最终 20s gameplay 冒烟**尚未通过**：kills=0。根因已定位（见下），方案待确认。

### 冒烟卡点根因（已定位，勿重复排查）
1. Orbit 模式射线必过原点，只能打中原点对面窄走廊；
2. NPC 走 A* 路径，攻击态才站定，停车点不在固定射线上；
3. 真实 bug：原点附近 3 个演示 AABB + 2 个球体拦截投射物（"吃子弹的箱子"）。

### 待确认方案
- 游戏侧（各一个 commit）：演示刚体挪到场地角落（feat(fix)）；NPC 进 Attack 站定时打位置日志；
- 冒烟脚本改为：等 NPC 站定 → 读日志 → 对跖点瞄准 → 点射 4–6 发；
- 或纯 harness 侧（飞行模式 + 反馈瞄准，不动游戏代码）。

## 四、待办任务（Next Steps）

1. 确认并执行上述冒烟方案，通过最终 20s 冒烟（断言：kill 日志、yaw/pitch 变化、
   wave= 序列、VUID=0、fps≥200、不崩溃）；
2. 提交 game.rs 的 spawn 日志改动（若保留）；
3. 冒烟脚本（/tmp/gameplay_smoke.py、/tmp/run_gameplay_smoke.sh、/tmp/release_keys.py）
   迁入仓库 scripts/ 并提交——/tmp 重启即失；
4. push 到 GitHub：WSL 直连不通，**在 Windows PowerShell 窗口执行 git push**；
5. 验收约束：dead-code 保持 0、测试全绿、不破坏现有 111 个测试。

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
| gameplay 冒烟 | FOCUS/yaw/pitch/掉血 OK，kills=0（待修） |
