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

### 阶段 2.5：20s gameplay 冒烟收口（方案 A，到 3e6a20f）
- 演示刚体挪到场地角落（距原点 >150m，不再拦截过原点射线）
- NPC 进 Attack 站定时打位置日志 `npc: #id stand (x,y,z)`
- 冒烟改为：读站定日志 → 对跖点瞄准 → 点射 6 发；20s 内 kills=1
- 验收：111 测试全绿、0 警告、冒烟 ALL-OK；scripts/ 迁入仓库并 push

### 阶段 3：Wave 2 六项功能（本会话，基于 3e6a20f）
- 真实玩家移动：WASD + PlayerBody 碰撞推回 + 地形贴地，FirstPerson 相机
- 武器手感：Firearm 弹匣/备弹/换弹进度、后坐力 kick、命中 hit marker 与音效
- AI 增强：就近掩体 `find_cover_points`、包抄 `should_flank`/`flank_goal`、波次难度曲线 `wave_profile`
- 音效接入：SfxBank（Gunshot/Footstep/Hit/Reload/UiBlip/Ambient）经 mixer 播放
- UI polish（部分）：ESC 设置面板（音量/灵敏度 + 键位列表）、HUD 备弹显示、hit marker
- 验收：160 测试全绿、0 警告、20s gameplay 冒烟 ALL-OK

## 三、当前工作状态（重要）

- Wave 2 六项功能已实施并通过验收（160 测试全绿、0 警告、20s gameplay 冒烟 ALL-OK）。
- 落地范围（基于 3e6a20f 的新 commit，见 git log）：
  - `src/engine/game.rs`：FPS 玩家移动（WASD + PlayerBody 碰撞 + 贴地）、Firearm
    （弹匣 30/备弹 120/换弹 2s/后坐力）、SFX 播放链路、AI 掩体/包抄、波次难度曲线、
    设置面板与 hit marker 同步、7 个新集成测试
  - `src/main.rs`：FirstPerson 输入分支（拖拽转视角 + 回中）、WASD 转发、R 换弹、
    ESC 设置面板、Tab 循环选中项、滚轮调音量/灵敏度、N 补给、后坐力加回相机
  - `src/ui.rs`：设置面板渲染、HUD 备弹 `AMMO x/y +z`、hit marker、清理已接线注解
  - `scripts/gameplay_smoke.py`：FPS 拖拽视角瞄准（相对注入 + 回中 + 点射）
- 未实施（下一阶段）：程序化地图生成/多关卡切换；主菜单美化与键位重绑定。

### 阶段 4：Wave 3 六项功能收尾（第 5 项全量 + 第 6 项完成）
- 程序化地图 `generate_level_map(seed)`（LCG 确定性、零依赖；障碍环带 58–130m，
  中央安全区保冒烟站定与弹道）+ 多关卡（每关 3 波，清完升关重建地图、
  难度按累计有效波次递进）+ NPC 出生点避障外推
- 世界障碍 marker 渲染：复用现有实例 pipeline，独立槽位 65537+ 与 2 个额外 draw call，
  零 pipeline/shader/swapchain 改动（Vulkan 校验零 VUID）
- 主菜单美化（装饰条/闪烁 PRESS ANY KEY/版本行）、键位绑定（设置面板 9 项循环，
  Enter 进入绑定 / 非 ESC 键完成 / ESC 取消，重复键互斥复位）、HUD `LEVEL` 显示
- 配置持久化 `src/config.rs`：`$HOME/.steel_front.cfg` 保存键位/音量/灵敏度
  （原子写 + 容错加载，测试环境不写盘）
- 键盘改键位驱动：移动/换弹/开火/菜单可重绑定；ESC 保留为系统键（关面板/退出）
- 验收：176 测试全绿、0 警告、20s gameplay 冒烟 ALL-OK

### 冒烟关键机制（勿回退，当前为 FPS 拖拽版）
1. 玩家在原点不动（FPS 相机），冒烟读 `npc: #id stand` 日志选距原点最近的站定 NPC，
   按 `(x, y+0.8-EYE, z)` 算 yaw/pitch，按住左键拖拽视角瞄准后点射 6 发；
2. NPC 攻击态无就近掩体时原地站定（`|C| ≤ 16m` 内，掩体搜索半径 40m）；
3. 障碍全在 58m 环带外，12–16m 弹道路径无拦截体；点射 6 发 × 25 伤害 ≥ 100 hp。

## 四、待办任务（Next Steps）

- 本批完成：Wave 2 六项功能（1–4 全量、6 部分）实施并提交。
- 本批完成：Wave 3（第 5 项程序化地图/多关卡全量、第 6 项主菜单美化/键位绑定完成）已提交。
- 后续候选：音效/难度曲线精调、更多地图风格（不同安全环半径/障碍密度）、
  新关卡特殊波次（Boss/援军）、主菜单选项（分辨率/画质）。
- 重跑冒烟：`bash scripts/run_gameplay_smoke.sh`（需沙箱外 X/Vulkan 权限）。

## 五、关键约定与环境备忘

- commit 规范：`feat(game): ...` / `feat(wiring): ...` / `chore:` / `docs:`
- 每步 `cargo build --release && cargo test` 全绿再继续；不新增第三方依赖
- 沙箱限制：绑 UDP socket / 连 X socket 需提权运行（沙箱外测试才准）
- X 注入坑：XTEST 卡键是 server 级，脚本开头先释放键 + pkill 清场
- HUD 用 5x7 位图字体，仅 ASCII，界面文案全英文
- 配置持久化在 `$HOME/.steel_front.cfg`（不进仓库目录）；`cargo test` 下不写盘
- 会话过长会压缩降智，长任务及时开新会话并先读本文件

## 六、验收数据快照

| 指标 | 数值 |
|---|---|
| 测试 | 176 passed, 0 failed |
| 警告 | 0 |
| 冒烟（飞行） | VUID=0, fps 228–300 |
| gameplay 冒烟 | ALL-OK：kills=1、hit_events=4、VUID=0、fps 214.8–292.7、无 panic |
