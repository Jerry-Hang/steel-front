# AGENTS.md — Steel Front 项目记忆

## 项目
二战题材 FPS，Rust + Vulkan（winit 0.30），零第三方游戏依赖，纯 bin crate。
- 入口：`src/main.rs`（GameApp + winit 事件循环）
- 运行时中枢：`src/engine/game.rs`（每帧 `update(dt, camera)` 编排物理/武器/AI/UI/音频/网络）
- 渲染：`src/engine/renderer.rs`（地形 LOD + 65536 实例场 + HUD 覆盖层；改 pipeline/shader/swapchain 风险高，须先跑冒烟验 VUID）
- 地形高度纯函数在 renderer.rs（`terrain_height` / `terrain_height_at`），中央 60×60 压平 y=0
- 配置持久化：`src/config.rs` → `$HOME/.steel_front.cfg`（原子写 + 容错加载，测试不写盘）
- 测试：`cargo test`（纯逻辑，不碰 GPU）；冒烟 `bash scripts/run_gameplay_smoke.sh`（需 X/Vulkan，20s，断言 kills>0）
- 验收约束：dead-code=0（0 警告）、测试全绿、不新增第三方依赖、commit 规范 `feat(game)/chore/docs`
- 内存约束：12GB，一次只跑一个 cargo，禁止并行构建

## 当前进度快照（2026-08-08，wsl --shutdown 前固化）

### 已完成（全部在本地 master，**均未 push**）
- Wave 2：`d5a4240` feat(game) / `5bd5f57` chore(scripts) / `0b7f5e6` docs
- Wave 3：`e593272` chore(wip) checkpoint / `bd2bb1f` feat(config) / `1010447` chore(game) / `f7e5f01` docs
- `.wslconfig` 已配置 `[wsl2] networkingMode=mirrored + dnsTunneling + firewall + autoProxy`，待 `wsl --shutdown` 生效
- 验收快照：176 tests passed、0 警告、20s 冒烟 ALL-OK（kills=1、VUID=0、fps 214.8–292.7）

### 重启后待办（按顺序）
1. 验证网络（mirrored+autoProxy 生效）：
   - `ip addr show eth0`（应不再是 172.x NAT 网段）
   - `env | grep -i proxy`（应有 http_proxy/https_proxy 自动注入）
   - `curl -I https://github.com`（应 HTTP/2 200）
   - `cd ~/Rust/Rust_Vulkan_3D && git fetch origin`
2. 若 curl 不通 → 检查 Clash 是否开 "Allow LAN"（监听 0.0.0.0）
3. `git log --oneline origin/master..HEAD` 确认领先的提交数
4. `git push origin master`（推 Wave 2+3 共 7 个提交）
5. 验证：`git fetch origin && git rev-parse HEAD` 与 `origin/master` 一致
6. 可选：README.md 更新 push 状态

### 重要约束
- git 操作只在 WSL 内跑，禁止 `\\wsl$` + Windows git（ref 缓存幻觉）
- 一个功能一个 commit，别出 mega-commit
- git identity: Evernight <3520143257@qq.com>
- 冒烟关键机制（勿回退）：程序化障碍环带 58–130m（game.rs `MAP_RING_INNER`），
  中央安全区保证 NPC 攻击态站定与弹道无阻挡；NPC 站定日志 `npc: #id stand (x,y,z)` 是冒烟瞄准依据
