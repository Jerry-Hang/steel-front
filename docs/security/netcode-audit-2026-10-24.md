# 联机（UDP）安全审计 — steel-front / `src/net.rs`

- **日期**：2026-10-24
- **审计对象**：`src/net.rs`（3057 行，协议魔数 `0x53`）+ 网络状态的消费端 `src/engine/game/net.rs`、`src/engine/game/weapons.rs`、`src/engine/game/projectiles.rs`、`src/lib.rs`
- **范围**：局域网/内网联机（`RV3D_NET=server|client`）。**静态审计为主**；
- **执行约束（严格遵守）**：**未运行 `cargo build` / `cargo test`**（Lead 可能在用 cargo，12 GB 内存只允许一个 cargo）、**未修改 `src/`**、未动 `assets/`、未 commit。所有现场验证都在**回环**上完成，工具不向任何非回环地址发包。
- **一句话结论**：协议的**编解码层是干净的**（边界检查、NaN 拒绝、长度/尾随校验都到位，未发现可达 panic）；问题全在**信任模型**——`Join` 之后既没有认证、也没有会话令牌、没有速率限制，于是"客户端权威包速率"直接变成服务端的移动速度与射速乘数，同时 1 个 17 B 的 Join 就能换来持续整帧快照（实测 203 KB/s）。

---

## 0. 摘要

| 编号 | 级别 | 一句话 | 是否需要伪造源地址 |
|---|---|---|---|
| S1 | **Critical** | `Input` 无 seq 去重、无每帧限额、每包推进一次 ⇒ 速度×N、穿墙、射速×N、无限弹药、刷分刷胜利 | 否（Join 即可） |
| S2 | **Critical** | 会话零认证：只按 UDP 源地址认人，无 nonce/token ⇒ 冒充服务器（伪造快照可**瞬移本机玩家**）、冒充客户端（1 包踢人） | 是 |
| S3 | **High** | 加入即广播：17 B Join → 持续整帧快照（实测 203 KB/s，峰值 13 MB/s）；64 个 socket 占满名额 | 否 |
| S4 | **High** | 客户端实体表/插值表无上限，`Position` 的 `player_id` 完全不校验 ⇒ 内存 + 渲染实例爆炸 | 是 |
| S5 | **High** | 服务端单帧收包无上限 + `fire_remote` 不扣弹无上限 ⇒ 单客户端打满服务端帧时间 | 否 |
| S6 | **High** | "服务器已满"也走 `Refuse` ⇒ 客户端把**瞬时**状态记成**永久**拒绝，永不重连（重启进程才恢复） | 否 |
| S7 | Medium | 日志注入：`rule_kind` / `Refuse.reason` 原样进日志（含换行），可伪造日志行 | 是 |
| S8 | Medium | 远端弹 `from_player=false` ⇒ 打死玩家不可能、打死**己方蓝 NPC** 可以；且击杀一律记成房主的 `DamageSource::Player` | 否 |
| S9 | Low | `net_look` 无写入点：`lib.rs:1730` 那段"客户端输入驱动房主相机"是死分支（若接线即 Critical） | — |
| S10 | Low | `payload.len() as u16` 无断言；`+ NET_PLAYER_BASE` 与 `wrapping_add` 两种写法混用 | — |
| S11 | Low | 客户端未 `UdpSocket::connect()`，只做用户态源地址比对；服务器换端口/端口重绑定即"永久断线" | — |
| S12 | Low | `MAX_SNAPSHOT_NPCS` 只封顶容量提示与编码条数，**解码仍按报文 count 收**；编码容量注释 24B/条 ≠ 实际 26B/条 | — |
| S13 | Info | 出生点硬编码 `[-110.0, 60.0]`，无占位/安全区校验，多人同点重叠 | — |
| S14 | Medium | 中继注册仍是 fire-and-forget：`BUSY`/`FULL`/`BAD port` 收不到，日志仍报"已注册"；解析函数只有测试在调 | — |

**证据分级**（借用 `dsh-purge/lib/redteam/evidence-ledger.mjs` 的 `confidence` 口径）：
`observed` = 本会话实测；`derived` = 静态推导 + 行号可复核；`inferred` = 推测。
本报告：编解码/布局类为 **observed**（本机工具实测），信任模型类为 **derived**（逐行可查，但**未在真实战场上抓包验证**——本轮禁止 `cargo build`，游戏进程未启动）。

---

## 1. 方法论：从红队预设里提炼的工作流

先读了这些文件（不是凭空发挥）：

| 文件 | 取用的东西 |
|---|---|
| `~/.dsh/.agent-presets/redteam/preset.yml` | 角色编制：主会话（指挥）+ 5 个执行角色、最多 3 并发 |
| `~/.dsh/.agent-presets/redteam/agent.cordis.yml` | **指挥只做四件事**（计划/派活/核对回报/汇报）；"必须用原生 subagent"；子代理是叶子；技能与资源预检先行 |
| `~/.dsh/profiles/web/node_modules/dsh-purge/lib/redteam/store-core.js` | 五阶段推进（①信息收集 → ②互联网资产权限 → ③边界突破 → ④内网资产权限 → ⑤靶标权限）；**先查库去重**；角色边界不交叉 |
| `.../lib/redteam/evidence-ledger.mjs` | `confidence ∈ {claimed, observed, derived, inferred}`；`claimed`/`inferred` **必须核实**才能升级为 `observed` |
| `.../lib/redteam/schema.js` | `stage(code)` 只有 5 个合法值；证据/得分/步骤都要带 stage |
| `.../lib/redteam/score-rules.js`（`STAGE_LABEL`） | 阶段标签与"按服务封顶"的计分口径（不重复凑数） |
| `.../docs/redteam-upstream-DISCLOSURE` | "编排/取证/报告层"定位：包本身不含 exploit，价值在**驱动什么、记录什么** |
| 本仓 `AGENTS.md` 教训 46 | 审计工具必须有第三种结局：`0=扫过无命中 / 1=有命中 / 2=没跑成`（扫到 0 个 ≠ 通过） |

这次把它落成 6 个可执行阶段（**对源码审计的等价物**）：

1. **侦察**：定位信任边界（谁读网络数据）、所有 `pub fn` 入口、所有 `unsafe`/`unwrap`（本模块 0 处 `unwrap`）、所有日志插值点。
2. **建模**：画"谁能发什么消息 / 谁按什么规则信任它"的表（§3）。资产清单是**消息类型 × 方向 × 接收方校验**，而不是 IP 列表。
3. **去重（对应"先查库"）**：先读 `src/net.rs` 的 40 余条单测，把"已有判据覆盖的结论"排除掉，只对**无判据覆盖**的路径下结论（§8 附录 B）。
4. **验证**：协议层用独立实现 + 回环实测（`tools/security/netcode_probe.py`），得到字节数与放大比这类**可复算**的数字，而不是"看起来能打"。
5. **报告**：每条发现必须给 `文件:行号` + 置信度 + 复现思路 + 判据（未来能变红的测试名）。
6. **加固排序**：按"改动行数 ÷ 消除的风险"排，而不是按级别排。

> 与红队的差别（诚实说明）：红队第 ②③④⑤ 阶段是**外部资产权限**，这里没有外部资产；等价物是"把信任边界当成靶标逐个突破"。因此本报告**不产出得分/攻击链资产库**，只产出发现 + 判据。

---

## 2. 范围、约束与验证手段

**在范围内**：`net.rs` 全部编解码与状态机；服务端对 `Input` 的应用（`game/net.rs`）；快照在客户端的落地（`handle_message` → `entities`/`interp_delay` → `lib.rs` 渲染）；`fire_remote` 的伤害结算；`RV3D_NET` 接线。

**不在范围内**：`llm_cmd.rs` 的 HTTP 出站、中继 `src/bin/rdv.rs` 的服务端实现（只看了协议对端 `rdv_register`/`rdv_resolve` 的用法）、Vulkan 渲染本身。

**验证工具**：`tools/security/netcode_probe.py`（新增，stdlib only）

| 子命令 | 做什么 | 判据（能变红） |
|---|---|---|
| `layout` | 独立实现一遍 `encode`/`decode` 布局，核对常量与实测字节数；跑 5 条拒绝路径 + NaN | 任一不符 → exit 1 |
| `budget` | 用真实常量算放大比 / 每包乘数 / 弹丸堆积 | — |
| `artifacts` | 生成 7 个伪造报文样本（`docs/security/artifacts/`） | — |
| `mock reflect` | 回环复现 `register`+`broadcast`，**实测**放大比 | 放大比 < 2× → exit 1 |
| `mock move` | 回环复现"每包推进一次"，**实测**速度/射速乘数 | 乘数 ≠ 包数 → exit 1 |

工具对**非回环目标直接拒绝**（exit 3，不是"跑了但没结果"）；`--seconds` 上限 10 s。

**关键实测数字**（`layout` 全绿 22/22；以下为原始输出的摘录）：

```
join('steel') 总长      实测 17B          # 5B 头 + 4 + 1 + 5 + 2
snapshot(n=128) 载荷    实测 3358B        # 30 + 128×26 ⇒ 整包 3363B
snapshot(n=1024) 载荷   实测 26654B       # 整包 26659B（≤ MAX_DATAGRAM 65507）
join 往返 / 拒绝：坏魔数 / 坏版本 / 未知类型 / 截断 / 尾随 / NaN 视角 —— 全部 OK

mock reflect（128 NPC，60 Hz，2 s，回环）:
  收到触发流量 17B  →  广播 121 帧 × 3363B = 406923B
  出向 203 KB/s ；名额上限 64 ⇒ 峰值 13.0 MB/s

mock move（10 包/帧，60 Hz，1 s，回环）:
  600 个 Input / 60 帧 ⇒ 位移 46.0 m ⇒ 46.0 m/s（基线 4.6 m/s，乘数 10.0×）
  单步 0.767 m（穿墙阈值 0.75 m）；开火 75 发（合法上限 8.3 发/s）
```

---

## 3. 威胁模型

**部署形态**：默认 `RV3D_NET_ADDR=127.0.0.1:27015`（仅回环，`lib.rs:4961`）；要联机必须显式改成 `0.0.0.0:<port>` 或局域网 IP ⇒ **一旦开联机，攻击面就是整个二层广播域**。无加密、无认证、明文。

| 角色 | 能力假设 | 备注 |
|---|---|---|
| A. 任意同网段主机（**无特权**） | 能 Join、能发任意构造的报文、能开任意多个 UDP socket | **S1/S3(b)/S5/S6/S8 只需这个能力** |
| B. 同网段 + 能伪造源地址（root/raw socket 或 ARP 污染） | 加上"以任意 `IP:port` 身份发包"、能收到发往被冒充地址的包 | S2/S4/S7 需要 |
| C. 恶意/被控的"房主"（server 端） | 客户端完全信任它 | 客户端侧只有 S4/S7 可利用 |
| D. 中间人（能双向嗅探/改写） | = B，且能读明文序号 ⇒ 可持续压制合法快照 | 见 S2 的"序号超前压制" |

**信任边界（谁信谁）**：

```
客户端 ──(Join/Input/Leave，无令牌)──▶ 服务端   服务端只按 UDP 源地址认人（net.rs:886 HashMap<SocketAddr,u32>）
服务端 ──(Snapshot/ObjectiveState/Join/Refuse，无令牌)──▶ 客户端   客户端只比 `from == self.server`（net.rs:1215）
```

两条边都**只校验地址，不校验身份**，且双方都**没有挑战应答**（对比 STUN 的 `MESSAGE-INTEGRITY` / QUIC 的 address-validation token）。这是 S2 的根因，也是 S3 反射放大的前提。

---

## 4. 攻击面清单（带行号）

| # | 面 | 位置 | 接收方校验 | 结论 |
|---|---|---|---|---|
| 1 | 报文头/长度 | `net.rs:583-601` | magic + version + type + `payload_len` + 尾随 | **充分**（无缺陷） |
| 2 | 逐字段读取 | `net.rs:408-463`（`Reader`） | `get()`/边界检查，`f32` 拒 NaN/inf | **充分** |
| 3 | 快照 count | `net.rs:650-661`，`snapshot_capacity_hint` `net.rs:79-81` | 容量封顶 1024；条数按实际载荷逐条读 | 容量侧已修；**语义不一致**（S12） |
| 4 | 目标状态 count | `net.rs:693-702` | 同上（`min(MAX_OBJECTIVE_POINTS)`） | 充分 |
| 5 | 魔数/版本 | `net.rs:587-592`；会话版本 `net.rs:1049` | 两道闸门 | 充分（但两个版本号并存，S10） |
| 6 | **Join → 注册** | `net.rs:924-950`，`1048-1066` | 只按源地址；`MAX_CLIENTS=64`；id 上界 | **无认证、无挑战**（S2/S3） |
| 7 | **Input 应用** | `game/net.rs:143`（入队）、`197-201`（逐包应用）、`340-402` | `seq` 被 `..` 丢弃；无速率限制 | **Critical**（S1） |
| 8 | **快照信任** | `net.rs:1262-1357`；`game/net.rs:318-327` | 只比源地址 + 序号单调 | 可伪造 ⇒ 瞬移（S2） |
| 9 | 实体表增长 | `net.rs:1323-1357`、`1423-1438` | 只按 2 s 老化 | 无上限（S4） |
| 10 | `Position` 分支 | `net.rs:1239-1261` | **不校验 `player_id` 是否认识** | 无上限（S4） |
| 11 | 单帧收包预算 | `game/net.rs:135` | `while let Ok(Some(..))` 无上限 | 帧时间可被打满（S5） |
| 12 | 投射物 | `weapons.rs:205-222`、`552-610` | 无上限、无弹药、无归属 | 放大（S5/S8） |
| 13 | 离场/踢人 | `game/net.rs:146-150` → `net.rs:962-965` | 只按 `from` | 1 包踢人（S2） |
| 14 | 重连状态机 | `net.rs:1391-1409`、`1518-1559` | `refused_reason` 粘性 | 永久锁死（S6） |
| 15 | 日志插值 | `game/net.rs:112-121`、`net.rs:1402-1406` | 原样 `{}` | 日志注入（S7） |
| 16 | 分片/MTU | 模块头 `net.rs:64-67`（明确不做分片） | — | 无重组面；有可用性问题 |
| 17 | 中继注册/解析 | `net.rs:287-362`、`net.rs:915-921` | `rdv_resolve` 有 `from == raddr`（`net.rs:338`）；**`Server::rdv_register` 压根不读回复** | 解析侧充分；**注册侧死路径**（S14） |
| 18 | 杂包跳过 | `net.rs:370-401`（`MAX_JUNK_SKIP=32`） | 有上限 | 充分 |

---

## 5. 发现

> 编号按**发现顺序**，不按级别排（级别见每条标题；总览见 §0）。每条给出：级别 / 证据（行号 + 置信度）/ 复现思路 / 影响 / 修复（具体到函数或校验）。

### S1 —【Critical】`Input` 无去重、无速率限制：客户端包速率 = 服务端速度/射速乘数

**证据**（`derived`，行号逐条可查）：
- `game/net.rs:143`：`NetworkMessage::Input { input, .. } => inputs.push((from, input))` —— **`seq` 与 `time` 被 `..` 丢掉**，协议里唯一的去重手段从未被使用。
- `game/net.rs:197-201`：对队列里**每一个** Input 调一次 `apply_net_player_input`。
- `game/net.rs:350`：`let speed = 4.6 * dt;` —— `dt` 是**帧时**，不是"距上次该玩家输入的间隔"⇒ **每包都按整帧推进**。
- `game/net.rs:371-380`：位移直接 `p.pos = [rx, p.pos[1], rz]`，`resolve_circle_static(.., 0.45)` 只做**落点**外推（`ai_util.rs:672-706`）⇒ 单步 > 半径+薄墙半厚即**穿墙**。
- `game/net.rs:385-401`：`p.fire_accum += dt` 也是**每包一次**，`>= 0.12` 就开火；`fire_remote`（`weapons.rs:205-222`）**不读也不扣任何弹药**、无每帧上限。
- `game/net.rs:135`：服务端单帧把所有排队包收完（无上限）。

**实测**（`mock move`，回环，10 包/帧）：`46.0 m/s`（基线 4.6，×10）、`75 发/s`（基线 8.3，×9）、单步 `0.767 m > 0.75 m` 穿墙阈值。`budget` 给的表：100 包/帧 ⇒ 460 m/s、833 发/s、约 1667 个并发弹丸。

**复现思路**：
1. 正常 `Join`（`artifacts/join_amplifier.bin` 就是合法 Join，17 B）。
2. 每帧发 10~100 个 `Input`（`artifacts/input_flood_frame.bin` 是一帧 10 个的样本，260 B），`forward=1, fire=1`。
3. 观察点：服务端 `net: server 远端玩家 #N 加入` 后，客户端被 `game/net.rs:318-327` 的 `dx²+dz² > 9.0` 硬对齐**反复拽回**（正常现象），但攻击者自己**每帧位移 0.77~7.7 m**、射速 83~833 发/s；`net_snap_seq` 每 tick +1 不变，说明不是"帧变快"，是**单帧内乘倍**。

**影响**：服务端权威被彻底旁路 —— 瞬移/穿墙（`speed` 乘 N）、超射速（`fire_accum` 乘 N）、无限弹药（`fire_remote` 无弹药概念）。因为 `damage_npc` 对**任何**击杀都按 `DamageSource::Player` 结算（`projectiles.rs:360`），击杀还会 `score += KILL_SCORE`（`weapons.rs:839`）并 `objective.progress(1)`（`weapons.rs:860`）⇒ **攻击者直接推进房主的"歼灭敌人"胜利条件**（刷分刷胜利）。这是"任何能 Join 的人"就能做的事，**不需要伪造源地址**。

**修复**（具体到函数）：
1. `Server` 增加 `last_input_seq: HashMap<SocketAddr,u32>`；`step_net_server` 收包时用现成的 `seq_is_newer`（`net.rs:877`）**滤掉重复/乱序**，且**每客户端每帧只应用一条**（取最新）。
2. `apply_net_player_input` 的 `dt` 改成 `min(now - p.last_rx, MAX_INPUT_DT)`（`p.last_rx` 已有，`NetPlayer.last_rx`），而不是 `self.last_dt`。
3. `fire_accum` 同步改用真实时间差；`fire_remote` 增加 `MAX_PROJECTILES`（例如 256）上限，满了丢最旧。
4. 判据（新增，必须能红）：`one_input_packet_per_frame_is_the_speed_baseline`（10 包/帧不得让位移 > 1.05×）、`duplicate_input_seq_does_not_move_the_player`。

---

### S2 —【Critical】会话零认证：只按 UDP 源地址认人（冒充服务器 / 冒充客户端 / 1 包踢人 / 瞬移本机玩家）

**证据**（`derived`）：
- `net.rs:886`：`clients: HashMap<SocketAddr, u32>` —— 身份 == 源地址。
- `net.rs:1048-1066`：`handle_join` 直接 `register(from)`，**无挑战、无 nonce、无 token**。
- `net.rs:1206-1223`：客户端 `recv` 只在用户态比较 `from == self.server`；`Client::connect`（`net.rs:1130-1172`）**没有** `UdpSocket::connect()`（内核不过滤）。
- `net.rs:1233-1238`：客户端接受**任意** `Join{player_id}` 写入自己的 id，不校验回显/版本。
- `game/net.rs:318-327`：`entity_state_at(NET_PLAYER_BASE + own, ..)` 距离 > 3 m 即**硬对齐本机玩家坐标** ⇒ 一条伪造快照即可把玩家**瞬移到任意坐标**（`artifacts/snapshot_teleport.bin`，61 B）。
- `game/net.rs:146-150` + `net.rs:962-965`：`Leave` → `unregister(from)` ⇒ **1 个 10 B 的包踢人**（`artifacts/leave_kick.bin`）。
- `net.rs:1264-1265`：快照只按序号单调接受 ⇒ 伪造者只要 `seq = 合法值+1` 就能**压制**随后到达的真快照（攻击者可持续领先）。

**复现思路**（需要 B 类能力：伪造源地址）：
1. 嗅探到房主 `H:P` 与客户端 `C:Q`。
2. 以 `H:P` 为源地址向 `C:Q` 发 `snapshot_teleport.bin`：`player_id` 填受害者自己的 id（`Join` 回执里有），`npcs` 里放 `id=100000+id, pos=(500,0,500)` ⇒ 受害者被瞬移；后续每次真快照都会被"序号已超前"挡掉，直到真快照序号追上。
3. 以 `C:Q` 为源地址向 `H:P` 发 `leave_kick.bin` ⇒ 注册表项消失，所有人画面里该玩家蒸发，受害者 3 s 后按超时重连（拿新 id，战绩清零——`reset_connection` 不保留会话）。

**影响**：客户端对"世界状态"的信任完全可被同网段第三方替换（位置/血量/阵营/开火指示全可伪造，`lib.rs:3267-3295` 无条件渲染）；玩家位置可被远程操控；比赛可被单包打断。**这是"内网可被同网段攻击者利用"最直接的路径**。

**修复**：
1. `Join` 改三步握手：`handle_join` 先回 `Challenge{nonce: [u8;16]}`（新消息类型，或复用 `Refuse` 加前缀），客户端 `Client` 存 `nonce` 并在 `Join` 里回显；**服务端只在 nonce 对上时 `register`** ⇒ 伪造源地址收不到 `Challenge`，S3 的反射也随之失效。
2. 注册成功后下发 32 B `session_token`；`Input`/`Leave`/`Snapshot` 都带它，服务端用 `HMAC`-风格常数比较校验（`std` 足够：`u128` 异或聚合比较）。
3. 客户端对 `Snapshot` 加"会话前置条件"：`self.player_id.is_some() || self.has_snapshot` 之外，还要 `token` 匹配；`Join` 回执里的 `name/version` 必须回显一致。
4. 玩家位置校正加合理性夹紧：单次校正距离 ≤ 8 m、且必须 `terrain_height_at` 附近（`game/net.rs:319` 处）。
5. 判据：`snapshot_from_unauthenticated_source_is_rejected`、`join_requires_nonce_echo`、`teleport_correction_is_clamped`。

---

### S3 —【High】加入即广播：17 B 换 203 KB/s（一个包 = 197× 放大），64 个 socket 即可锁死房间

**证据**（`observed`，回环实测）：
- `net.rs:924-950`：`register` 只看源地址、上限 64。
- `game/net.rs:137-142`：收到 `Join` 就 `handle_join` ⇒ 立即登记。
- `game/net.rs:218-255`：**每 tick** 组一整帧快照 `server.broadcast(&snapshot, None)`，不去重、不按需。
- `net.rs:1035-1044`：`broadcast` 逐个 `send_to`；`net.rs:981-996`：只有 `SERVER_TIMEOUT=5 s` 才摘。

**实测**：17 B `Join` → 121 帧 × 3363 B = 406,923 B / 2 s ⇒ **203 KB/s 持续**；`MAX_CLIENTS=64` 时峰值 **13.0 MB/s**（`MAX_SNAPSHOT_NPCS=1024` 时 1.6 MB/s / 单受害者、102 MB/s / 64 名额）。维持成本：每 5 s 补 1 个 Join（或 26 B 的 `Input`，因为 `net.rs:1004` 对**任何解开的包**都刷新 `last_seen`）。

**两条臂**：
- **(a) 反射/放大**（需伪造源地址）：把 Join 的源地址伪造成受害者 ⇒ 服务器替攻击者向受害者灌 203 KB/s。
- **(b) 名额耗尽（不需要伪造！）**：本机开 64 个 UDP socket，各发 1 个 Join ⇒ `register` 满 ⇒ 之后所有真实玩家的 `Join` 都收到 `Refuse{"服务器已满"}`。**配合 S6，这不是 5 s 的抖动，而是永久锁死**（客户端把"满"记成永久拒绝，只能重启进程）。成本：64 × 17 B + 每 4 s 各 26 B ≈ 416 B/s。

**修复**：
1. 广播**只发给"已确认会话"**（S2 的 token/challenge 通过后），并加 `MIN_JOIN_INTERVAL`。
2. 快照按订阅需求/可见性裁剪（`MAX_SNAPSHOT_NPCS` 已是 1024，实际战场 128 ⇒ 可先按 256 封顶）。
3. `register` 满时**不**回不可重试的 `Refuse`（见 S6），并优先淘汰"只 Join 过、从未发过 Input"的条目（现在这两类无法区分，因为 `last_seen` 对任何包都刷新）。
4. 判据：`unverified_join_does_not_receive_snapshots`、`join_flood_cannot_evict_an_active_client`。

---

### S4 —【High】客户端实体表/插值表无上限；`Position` 的 `player_id` 完全不校验

**证据**（`derived`）：
- `net.rs:1323-1357`：快照里每个 NPC 都 `entities.insert(npc.id, ..)`，**没有条数上限**；一条 65,507 B 的报文可携带约 2,500 个新 id。
- `net.rs:1423-1438`：只按 `now - last_seen <= 2.0` 老化 ⇒ 持续换 id 即可维持任意大的表。
- `net.rs:1239-1261`：`Position` 对**未知** `player_id` 直接 `remote_players.insert(player_id, ..)`；`prune_stale_entities` 只在 `removed > 0` 时才连带清理 `remote_players`（`1432-1436`）⇒ 只发 `Position` 就能让这张表**只增不减**。
- `lib.rs:3266-3295`：客户端把 `entities` **全部**转成 `NpcVisual`（`id >= NET_PLAYER_BASE || id == 0 || hp > 0.0`）⇒ 每帧实例数 = 表大小；`lib.rs:3397-3412` 的开火引擎还会再遍历一遍。

**复现思路**：以 `H:P` 为源，向 `C:Q` 连续发 60 Hz 的快照，每帧换 2,500 个新 id（≈ 65 KB/帧 ⇒ 3.9 MB/s），或只发 `artifacts/position_unbounded_ids.bin` 这类 `Position` 洪水 ⇒ 客户端 `entities`/`remote_players` 单调增长 ⇒ 内存上涨 + 每帧构造几十万 `NpcVisual` ⇒ 帧率崩塌/被 OOM 杀掉。

**修复**：
1. `Client` 加 `MAX_ENTITIES`（例如 4096）与 `MAX_REMOTE_PLAYERS`（例如 64）：满了**拒绝新 id**并计数告警（不要静默丢）。
2. `Position` 分支只接受已知 id，或直接**删掉**（`server|client` 模式下服务端从不发 `Position`，见 §8；它只服务于 `RV3D_NET=1` 的旧环回演示）。
3. `prune_stale_entities` 无条件清理 `remote_players`（现在被 `if removed > 0` 挡住）。
4. 判据：`entity_table_is_bounded`、`position_for_unknown_player_is_ignored`。

---

### S5 —【High】服务端单帧收包无上限 + 投射物堆积：单客户端打满服务端帧时间

**证据**（`derived` + 实测乘数）：
- `game/net.rs:135`：`while let Ok(Some((msg, from))) = server.recv()` —— 一帧内把 socket 里**所有**合法包处理完，没有 `MAX_PACKETS_PER_FRAME`（对比 `net.rs:370` 只给"解不开的杂包"设了 32 条上限）。
- `game/net.rs:385-401` + `weapons.rs:205-222`：每 0.12 s-当量就 `projectiles.push`，弹丸寿命 2 s ⇒ `存活 ≈ 发速 × 2`。
- `weapons.rs:552-610`（`nearest_segment_hit`）：**每个弹丸每帧**对 `world.bodies` 全量做线段-AABB 测试，加 `spheres`、`net_players`、`npcs`。

**实测**：10 包/帧 ⇒ 83 发/s ⇒ 约 167 个并发弹丸；100 包/帧 ⇒ 833 发/s ⇒ 约 1,667 个并发弹丸。以一次 `nearest_segment_hit` 扫数百个 AABB 计，1,667 弹丸/帧 × 数百 = **每帧百万级相交测试**，而这只需要 **100 个小包/帧（2.6 KB/帧）**。

**影响**：单客户端让房主帧率归零（连带 `RV3D_AI_DECIMATE` 也救不了，因为成本在弹道不在 AI）；与 S1 叠加时代价更低。

**修复**：
1. `step_net_server` 加 `const MAX_PACKETS_PER_FRAME: usize = 256;`（超出的留到下一帧/丢弃并计数）。
2. `MAX_PROJECTILES`（建议 256）+ 到上限时丢最旧（`Projectile` 已有 `alive` 位，`retire_projectile` 已有回收路径）。
3. 判据：`input_flood_cannot_exceed_frame_budget`、`projectile_count_is_capped`。

---

### S6 —【High】`Refuse` 一码两用：把"服务器已满"（瞬态）记成永久拒绝，客户端永不重连

**证据**（`derived`）：
- `net.rs:1049-1062`：**同一个** `Refuse` 同时用于"版本不匹配"（永久）与"服务器已满"（瞬态）。
- `net.rs:1391-1409`：收到 `Refuse` 且 `player_id.is_none()` ⇒ `refused_reason = Some(reason)`。
- `net.rs:1538-1549`：`retry_join` 见到 `refused_reason.is_some()` 直接 `return false`（粘性，注释写明是刻意的）。
- `net.rs:1518-1536`：`reset_connection` 清 10 个字段，**不清 `refused_reason`**；而 `game/net.rs:330-336` 的自动重连只在 `player_id.is_some()` 时才调它 ⇒ **一旦被拒，连重连入口都进不去**。
- 既有判据 `refuse_stops_retry_join`（`net.rs:1837-1876`）只覆盖"版本不匹配"这一语义，**没有**覆盖"满"。

**复现思路**：先按 S3(b) 用 64 个 socket 占满名额；让受害者此刻启动客户端 ⇒ 它收到 `Refuse{"服务器已满…"}` ⇒ **即使攻击者立刻释放全部名额，受害者也永远不会再 Join**（日志只有一行 `握手被服务器拒绝（版本不匹配？）`，具误导性）。不需要任何伪造、不需要重启客户端以外的手段。

**修复**：
1. `Refuse` 增加 `retryable: u8`（或拆 `Busy` 消息类型）：`服务器已满` ⇒ retryable=1。
2. `retry_join`：`refused_reason` 只在 `retryable == 0` 时才阻断；`reset_connection` 必须清 `refused_reason`（它是**会话**级状态）。
3. 日志文案不要预设"版本不匹配"。
4. 判据：`transient_refusal_keeps_retrying`、`reset_connection_clears_refusal`。

---

### S7 —【Medium】日志注入：`rule_kind` 与 `Refuse.reason` 原样进日志

**证据**（`derived`）：
- `game/net.rs:110-121`：`log::info!("net: client id={:?} … rule={}", … client.objective_rule())` —— `rule_kind` 是**网络来的 `String`**（`net.rs:688`，≤255 B，任意 UTF-8）。
- `net.rs:1402-1406`：`log::warn!("握手被服务器拒绝… 原因：{}", reason)` 同理。
- 受害者侧 `ObjectiveState` 的据点 id 还会进 HUD（`view.rs:59-70` → `hud.capture_points`，最多 64 条 × 255 B 的可控文本）。
- 影响面特别大：本仓多数验收判据**读日志**（`AGENTS.md` 的 `panics==0`、`VUID=0`、`net: server clients=`、`survival` 系列），伪造一行 `net: server clients=0 seq=0 npcs=0` 就能污染取证。

**复现思路**：以 `H:P` 为源发 `artifacts/objective_log_inject.bin`（75 B，`rule_kind` 里带 `\n`）或 `refuse_log_inject.bin`（44 B）⇒ 受害者日志多出一行完全可控的内容。

**修复**：加 `fn sanitize_log_field(s: &str) -> String`（拒绝 `\n\r\t` 与控制字符、截断 64 B、非 ASCII 用 `{:?}`），在**所有**网络字符串进日志/HUD 的地方调用；或统一改成 `{:?}`（会转义换行）。

---

### S8 —【Medium】远端弹的归属语义错了：打不到玩家、能杀己方蓝 NPC、击杀全记房主

**证据**（`derived`）：
- `weapons.rs:205-222`：`fire_remote` 用 `Projectile::new(...)`，而 `Projectile::new` 的 `from_player` 默认 **false**（`weapons.rs:305`），`with_player()` 只有 `fire_shot` 在 `from_player=true` 时调用（`weapons.rs:55-57`）。
- `weapons.rs:583-587`：`SegmentHit::NetPlayer` 只在 `p.from_player()` 时参与 ⇒ **远端玩家的子弹永远打不到任何玩家**（房主与其他客户端都打不到）。
- `weapons.rs:601`：跳过蓝营 NPC 也只在 `p.from_player()` 时生效 ⇒ **远端玩家的子弹会打自己阵营的蓝 NPC**（`team` 无关）。
- `projectiles.rs:360`：NPC 命中一律 `damage_npc(idx, dmg, DamageSource::Player)` ⇒ 远端玩家打死敌人时，`score += KILL_SCORE`（`weapons.rs:839`）、`objective.progress(1)`、击杀提示是 `KILLED … You`。

**影响**：PvP 事实上不存在（客户端无敌）；一个恶意客户端可以**清空自己一方的蓝营 NPC**（`tally_round_deaths(Blue, ..)` 会污染重组判据），同时把房主的分数/胜利进度刷上去（S1 的放大器）。

**修复**：`fire_remote(origin, dir, owner: NetPlayerId)`；`Projectile` 增加 `source: ShotSource { LocalPlayer, RemotePlayer(u32) }`；`nearest_segment_hit` 按 `source` 决定"能不能打玩家/能不能打友军"；`damage_npc` 的 `DamageSource` 增 `RemotePlayer(id)`，计分只给击杀者的阵营。

---

### S9 —【Low】`net_look` 没有写入点：`lib.rs` 那段"客户端输入驱动房主相机"是死分支

**证据**（`derived`，全仓核实）：`net_look` 只有 5 处出现 —— 字段声明（`game.rs:797`）、初始化 `None`（`session.rs:239`）、getter（`game/net.rs:90-91`）、读者（`lib.rs:1731`）。**没有任何赋值**（`grep -rn "net_look\s*="` 为空）。

**影响**：注释宣称"服务器模式：客户端输入视角驱动本机相机（快照权威视角）"，实际永不生效（房主视角始终本地）。这是一条**声明与实现不符**（`AGENTS.md` 教训 34 那类），而不是漏洞 —— 但要注意：**若按注释把它接上（`net_look = Some((input.yaw, input.pitch))`），立刻变成 Critical**（同网段任意客户端可强行扭动房主视角 = 控制房主瞄准方向）。

**修复**：二选一 —— 要么删掉该死分支与注释，要么接线时**加"仅当该客户端是认证过的观察者"并夹紧**（且不要在多人自由对战里用）。

---

### S10 —【Low】整数/强转的"还没炸"点

**证据**（`derived`）：
- `net.rs:577`：`(payload.len() as u16)` 无断言。当前最大 26,654 B（`layout` 实测），余量 38,881 B；**一旦 `MAX_SNAPSHOT_NPCS` 提到 ~2,519 或加字段就会静默截断**（截断后对端 `decode` 直接 `Truncated`，表现是"偶发丢包"，很难查）。
- `game/net.rs:239`、`319`：`NET_PLAYER_BASE + p.id` 用普通 `+`，而 `net.rs:1302`、`1364` 用 `wrapping_add` —— 两种写法混用。当前 `register` 已把 id 限在 `NET_PLAYER_BASE` 以下（`net.rs:939-945`）⇒ 不可达；但 debug 构建里溢出会 panic。
- `Reader::bytes`（`net.rs:446-453`）的 `self.off + n`：`off ≤ 65,507`、`n ≤ 255` ⇒ **不可能溢出**（这一条是"已确认安全"，不是缺陷）。

**修复**：`encode` 里加 `debug_assert!(payload.len() <= u16::MAX as usize)` + 运行期 `min` 截断并发一次告警；统一用 `wrapping_add` 或一处 `player_entity_id(id)` 辅助函数。

---

### S11 —【Low】客户端没 `connect()` 套接字；只比地址带来的两个副作用

**证据**：`net.rs:1130-1172` 只 `bind`（注释也写明"没有 `connect(server)`"）；`net.rs:1206-1223` 用户态丢弃非 `self.server` 来源。

**影响**：① 内核仍然把任意来源的包投递到本进程（每包一次 `decode` + 一次比较；洪泛时纯浪费）；② 房主换端口/中继重绑定后，旧 `self.server` 永不匹配 ⇒ 客户端表现为"永久断线"（`snapshot_timeout` 触发重连，但仍发往旧地址）。`UdpSocket::connect()` 能同时解决两端（内核过滤 + 收 ICMP 端口不可达）。

**修复**：`Client::connect` 末尾 `socket.connect(server)?`，`recv` 改用 `recv()`；重连时重建 socket（`reset_connection` 里清空并重建，注意 `player_id` 语义）。

---

### S12 —【Low】`MAX_SNAPSHOT_NPCS` 的两处语义不一致

- `net.rs:79-81` 只封顶**容量提示**（好，防了 1.6 MB 的放大，注释也写清了），但 `net.rs:650-661` 的 `for _ in 0..n` 仍按**报文里的 count** 逐条读 —— 实际可接受上限由载荷长度决定（≈2,500 条），与常量宣称的 1024 不符。
- `net.rs:513` 的容量算式按 **24 B/条**，而实际每条 **26 B**（`layout` 实测 3358 = 30 + 128×26）—— 只是容量估计偏小（`Vec` 会增长），无安全影响，但两个数字**将来一定有人照着注释去算**。
- `net.rs:523` 编码侧 `take(MAX_SNAPSHOT_NPCS)` 是**静默截断**，而远端玩家恰好排在 `npcs` **末尾**（`game/net.rs:237-246`）⇒ NPC 数超过 1024 时，**玩家实体会被整体截掉**（不是先截 NPC）。

**修复**：解码时 `if n > MAX_SNAPSHOT_NPCS { return Err(NetError::Truncated) }`（或叫 `OversizedCount`）；容量注释改 26；编码侧把玩家放前面或先按类型分配配额。

---

### S13 —【Info】出生点硬编码、无占位校验

`game/net.rs:404-406`：`fn net_spawn_point(&self) -> [f32; 2] { [-110.0, 60.0] }` —— 所有远端玩家同点出生（`game/net.rs:179-195`），不看队伍、不看地形/障碍；`y` 由每 tick 的 `terrain_height_at` 重算（`game/net.rs:212-215`），所以不会掉出地面。

**影响**（低）：换地图后该坐标可能落在建筑/掩体内 ⇒ 新玩家出生即被 `resolve_circle_static` 推出或卡在几何里；多人同点 ⇒ 实体完全重叠（互相挡枪线、观感像"一个人"）。**复现**：换 `RV3D_MAP=assets/maps/defense_line.toml` 起房主，两个客户端同时 Join，看两人出生坐标是否都是 `(-110, 60)`。

**修复**：按 `id` 取环形偏移（`angle = id * golden_angle`、半径 2~6 m），并复用 `Game::nearest_in_component`（`AGENTS.md` 铁律 H 里出生点吸附的现成判据）。

---

### S14 —【Medium】中继注册仍 fire-and-forget：三种拒绝收不到，日志仍说"已注册"

**证据**（`derived`）：
- `net.rs:915-921`（**唯一生产调用方**，`lib.rs:4980`）：`Server::rdv_register` = `send_to("REG …")` + `Ok(())`，**不读回复**。
- `net.rs:287-319` 的 `parse_rdv_reply` + 自由函数 `rdv_register`（带 `read_timeout`、带 `from == raddr` 过滤）**没有生产调用方** —— 只有 `net.rs:3033-3056` 的测试在调它。
- `lib.rs:5048-5070`：`rdv_register_report` 的 `Ok(())` 分支直接打 `已向中继 {rdv} 注册房间 {name}` ⇒ 中继回 `BUSY name` / `FULL` / `BAD port` 时，**日志与退出码都说成功**。
- 测试 `reg_reply_is_parsed_and_errors_are_surfaced`（`net.rs:3044`）**绿着**，而它测的是没人用的那份实现 —— 典型"判据绿、生产没接"（`AGENTS.md` 教训 46/14）。测试自己的注释甚至写着"`rdv_register_report` 只能报'发送失败'"，说明这是**已知但未接线**。

**影响**：NAT 打洞链路第一环（注册）失败时现场没有证据：中继侧无 `REG` 记录，客户端 `rdv_resolve` 只会等到 5 s 超时，排查会跑到打洞/防火墙上去（`lib.rs:5043-5046` 的注释正是为此写的，但只解决了一半）。日志"说成功"在本仓属于一等缺陷（多条验收判据读日志）。

**修复**：把 `parse_rdv_reply` 接进 `Server::rdv_register`：注册阶段（启动期，不在帧循环里）临时 `set_read_timeout(2s)`，读一条 `from == raddr` 的回复并解析；`BUSY`/`FULL` 要作为**可诊断错误**返回给 `rdv_register_report`。判据：`relay_registration_reports_busy_as_failure`（用回环假中继回 `BUSY name`，断言 `ok == false`）。

**顺带纠正第一轮的同类结论**：第一轮 #3 写"中继回复 `OK <addr>` 直接信任为服务器地址 / 任何能往本机端口发 UDP 的进程都能伪造"—— 这一点**已在 2026-10-19 修掉**（`net.rs:334-338` 有 `from == raddr` 过滤）。残余风险只剩"中继**本身**（或能伪造中继地址并猜对房间名的人）可把客户端指向任意服务器"，因为没有指纹/签名，属**设计性单点信任**（Low）。

---

## 6. 明确"未发现"的类别（附依据）

| 类别 | 结论 | 依据 |
|---|---|---|
| 报文长度/偏移/越界 | **未发现**（不可达越界） | `Reader` 全部走 `get()` 与显式边界（`net.rs:418-453`）；`decode` 三重校验（`583-601`）；生产代码 **0 处 `unwrap()`/`expect()`**（本会话 `awk` 全量扫描）；`decode_never_panics_on_mutated_bytes_and_reencodes`（`net.rs:2091`）是既有模糊式判据；本工具 `layout` 的 5 条拒绝路径实测通过 |
| panic/整数溢出 | **未发现可达 panic** | `off + n ≤ 65,762` 不溢出；`payload_len as usize` 与 `HEADER_LEN + payload_len` 均有界；`f32` 读入拒 NaN/inf（`net.rs:438-444`，判据 `non_finite_floats_are_rejected_at_the_wire`）；仅 S10 的"强转无断言"属**潜在**风险 |
| 魔数/版本校验 | **未发现缺陷** | `net.rs:587-592` 先 magic 后 version 再 type；`handle_join` 再按 `SESSION_VERSION` 二次闸（`net.rs:1049-1055`）；客户端对 `Refuse` 停止重试（S6 的问题是**语义**，不是校验缺失） |
| 序列号/窗口 | **未发现缺陷**（`seq_is_newer` 正确），**但 Input 根本不用序号** | `net.rs:870-880` 半程 `2^31` 正确（判据 `sequence_staleness_uses_the_documented_half_range`）；`Snapshot`（`1264`）、`ObjectiveState`（`1373`）、`Position`（`1249`）都接了；`Input` 在 `game/net.rs:143` 被丢弃 ⇒ 见 S1 |
| 快照容量放大（2 字节 count → 1.6 MB） | **已修，未发现残留** | `snapshot_capacity_hint`（`net.rs:79-81`）+ `net.rs:652`；`layout` 实测 1024 条解码不 panic |
| 分片/重组类漏洞 | **不存在该类攻击面**（不做重组） | 模块头 `net.rs:64-67` 明确"本轮不做分片/重组"⇒ 不存在重叠分片/teardrop 的用户态实现；代价是 MTU 不自适应（3.3 KB 快照在 1500 MTU 上靠 IP 分片，丢一片即整帧丢失），属可用性而非安全 |
| 反射放大 | **发现** | S3 |
| 速率限制 | **完全没有**（4 条独立路径） | `game/net.rs:135`（收包）、`143/197`（Input）、`net.rs:1048`（Join）、`weapons.rs:205`（弹丸） |
| 日志注入 | **发现** | S7 |
| 客户端位置伪造 | **不存在**（设计正确） | `NetInput`（`net.rs:178-193`）只有 5 个 bool + yaw/pitch，**没有坐标**；服务端收到 `Position`/`Snapshot` 一律 `_ => {}` 丢弃（`game/net.rs:151`） |
| 客户端输入夹紧 | **部分缺失** | yaw/pitch 不做 `wrap_angle`/夹紧（无安全后果：方向向量恒为单位长度，`weapons.rs:283-291` 还有零长保护）；缺的是**包速率**与**每帧位移**的夹紧（S1） |
| 作弊面（瞬移/无限弹药/刷击杀） | **发现** | 瞬移 = S1（速度乘倍）+ S2（伪造快照硬对齐）；无限弹药 = S1（`fire_remote` 无弹药）；刷击杀 = S1 + S8（`damage_npc` 无条件 `DamageSource::Player`） |
| 会话恢复/重放 | **无重放保护**（设计如此，无 token/nonce） | 归入 S2；`Input` 重放还能直接乘倍（S1） |
| TLS/加密 | **无**（明文协议） | 全局；不是缺陷而是前提，已写入威胁模型 |

---

## 7. 加固清单（按性价比排序）

> 排序依据 = 消除的风险 ÷ 改动量。前三条做完，Critical 全部清零。

| 优先级 | 改动 | 消除 | 预估改动量 | 落地判据 |
|---|---|---|---|---|
| **1** | **`Input` 每客户端每帧只应用一条 + `seq_is_newer` 去重 + `dt` 改用真实间隔**（`step_net_server` 聚合 `HashMap<SocketAddr, (seq, NetInput)>`，`apply_net_player_input` 读 `p.last_rx` 差值并 `min(MAX_DT)`） | S1 全部（速度×N/穿墙/超射速/无限弹药/刷分） | ~40 行，纯逻辑，无协议变更 | `one_input_packet_per_frame_is_the_speed_baseline`、`duplicate_input_seq_does_not_move_the_player` |
| **2** | **Refuse 拆分 retryable + `reset_connection` 清 `refused_reason`** | S6（永久锁死），并让 S3(b) 从"永久"降级为"5 s 抖动" | ~15 行（`net.rs:1407/1518/1547` 三处） | `transient_refusal_keeps_retrying`、`reset_connection_clears_refusal` |
| **3** | **三道上限**：`MAX_PACKETS_PER_FRAME`(256)、`MAX_PROJECTILES`(256)、`MAX_ENTITIES`(4096)/`MAX_REMOTE_PLAYERS`(64)；`Position` 只认已知 id | S4 + S5（两种"打满"） | ~50 行 | `input_flood_cannot_exceed_frame_budget`、`entity_table_is_bounded` |
| **4** | **Join 挑战应答 + 32 B session token**（新消息 `Challenge`/`Welcome`，`Input`/`Leave` 带 token） | S2 全部 + S3(a) 反射 + S7 的一部分（伪造 `Refuse` 需要过 token） | ~120 行，**协议版本 +1**（`SESSION_VERSION=3`）；客户端/服务端必须同版本 | `join_requires_nonce_echo`、`snapshot_from_unauthenticated_source_is_rejected` |
| **5** | **`UdpSocket::connect()` + 重连时重建 socket** | S11 两条副作用，顺带减少无效 `decode` | ~10 行 | 手工：房主换端口后客户端能重连 |
| **6** | **`sanitize_log_field()` 统一用于网络字符串**（`rule_kind`、`Refuse.reason`、未来的玩家名） | S7 | ~20 行 | `network_strings_never_inject_log_lines`（喂 `\n` 断言日志行数不变） |
| **7** | **`ShotSource` 重构**（`fire_remote` 带 owner；`DamageSource::RemotePlayer`） | S8（PvP 语义 + 友军伤害 + 计分归属） | ~60 行，涉及 `nearest_segment_hit`/`damage_npc` | `remote_shot_cannot_damage_own_team`、`remote_kill_is_not_credited_to_the_host` |
| **8** | **`MAX_SNAPSHOT_NPCS` 语义收敛 + 编码断言 + 玩家优先** | S12、S10 | ~15 行 | `snapshot_count_above_cap_is_rejected` |

**不建议现在做**：给整条协议加密（局域网对战收益低，且会挡掉 S2 一半价值——token 已足够挡伪造；加密留给公网/中继阶段）。

---

## 8. 附录

### A. 工具与复现命令

```bash
# 布局自检（22/22 通过才算布局没漂）
python3 tools/security/netcode_probe.py layout

# 三笔账：放大比 / 每包乘数 / 弹丸堆积
python3 tools/security/netcode_probe.py budget --npcs 128 --tick-hz 60

# 伪造报文样本（写到 docs/security/artifacts/）
python3 tools/security/netcode_probe.py artifacts

# 回环实测（只允许 127.0.0.1；非回环直接 exit 3）
python3 tools/security/netcode_probe.py mock reflect --seconds 2 --npcs 128
python3 tools/security/netcode_probe.py mock move    --seconds 1 --packets-per-frame 10
```

`artifacts/` 内容（7 个样本，均可在隔离实验网重放）：`join_amplifier.bin`(17B)、`snapshot_teleport.bin`(61B)、`leave_kick.bin`(10B)、`refuse_log_inject.bin`(44B)、`objective_log_inject.bin`(75B)、`input_flood_frame.bin`(260B)、`position_unbounded_ids.bin`(232B)。

**本轮未做的验证（诚实标注）**：没有启动游戏进程（禁止 `cargo build/test`），因此**没有真实双进程抓包**；S1/S2/S4/S5/S6 的"影响"是按行号推导 + 协议级实测，属 `derived`。要升级为 `observed`，最小代价是：`RV3D_NET=server` 起一个房主 + 用本工具的 `mock move` 思路发包（同一台机、回环），观察 `net: server clients=` 与房主 HUD 的击杀数。

### B. 与既有判据的关系（哪些已覆盖、哪些是空白）

**已覆盖（结论不必重复怀疑）**：`client_ignores_datagrams_from_a_foreign_source`、`junk_datagram_does_not_block_the_valid_one_behind_it`、`register_refuses_beyond_limits`、`out_of_order_position_is_dropped_not_rolled_back`、`decode_never_panics_on_mutated_bytes_and_reencodes`、`non_finite_floats_are_rejected_at_the_wire`、`snapshot_decode_rejects_oversized_npc_count`、`reset_connection_clears_session_scoped_state`、`refuse_stops_retry_join`、`snapshot_host_and_npc_zero_land_in_distinct_slots`、`sequence_staleness_uses_the_documented_half_range`。

**空白（本报告新增的判据都落在这里）**：

| 空白 | 对应发现 |
|---|---|
| Input 的包速率/去重/dt 归一化 | S1 |
| 认证/挑战/token（一条都没有） | S2、S3 |
| 广播前的订阅校验 | S3 |
| 实体表/插值表上限 | S4 |
| 每帧收包预算、弹丸上限 | S5 |
| Refuse 的可重试语义 | S6 |
| 网络字符串进日志的净化 | S7 |
| 远端弹的归属与友军/阵营语义 | S8 |
| 中继注册回复的解析（判据测的是死代码） | S14 |

> 注意 `refuse_stops_retry_join` 这条判据**把 S6 的错误语义固化成了"正确"**（它只测"版本不匹配"）。修 S6 时它必须**改**（不是删）：加一个 `retryable` 分支后，"不可重试"仍然要断言，另加"可重试"那条。（这正是 `AGENTS.md` 教训 14 "回归测试把错误结论固化下来"的形态。）

### C. 与第一轮审阅（`docs/NET-SECURITY-REVIEW.md`，2026-10-24 11:03 提交）的交叉核对

第一轮是**广度优先的黑盒+读码**（37 行、6 条），本轮是**深度优先的信任边界审计**（14 条 + 工具）。逐条核对（红队口径：`claimed` 必须自己核实过才能算 `observed`）：

| 第一轮条目 | 本轮核对结果 |
|---|---|
| #1 无会话令牌/握手认证（`net.rs:1048`、`game/net.rs:198`） | **确认**（→ S2）。本轮补上三条具体利用链：伪造快照**瞬移本机玩家**（`game/net.rs:318-327`）、**1 包踢人**（`game/net.rs:146-150`）、**序号超前压制**合法快照（`net.rs:1264`） |
| #2 无每源限速（`game/net.rs:135`） | **确认并升级**（→ S1+S5）。`game/net.rs:135` 只是"帧时间被吃满"；真正的乘数在 `143`（丢 `seq`）、`197-201`（逐包应用）、`350`（每包一个整帧 `dt`）、`385`（每包加 `fire_accum`）⇒ 10 包/帧实测 **4.6 → 46 m/s**。**只加收包上限不解决问题**（包还是都进了 `inputs`） |
| #3 中继 `OK <addr>` 直接信任（`net.rs:287-305`） | **部分过期**。`net.rs:334-338` 已有 `from == raddr` 过滤（2026-10-19 修）⇒ "本机任意进程伪造 `ADDR`"已不可行。残余：中继**本身**无指纹/签名 = 设计性单点信任（Low）。**反倒是注册方向仍有问题**（见 S14） |
| #4 名字 255 B 不过滤、会回显（`net.rs:604-608`） | **确认，但当前未触发**：名字既不上 HUD 也不进日志（`handle_join` 只回显给本人；客户端不存 `name`；`join_name_truncated_to_255_bytes` 钉住截断）⇒ 属**潜在**风险（无净化函数）。**第一轮漏了真正在进日志的两个字段**：`rule_kind`（`game/net.rs:112`）与 `Refuse.reason`（`net.rs:1402`）→ S7 |
| #5 远端步长用 `last_dt`，建议 `clamp(0, 0.05)`（`game/net.rs:350`） | **升级为 S1**：不是"卡顿时变大"，而是**每包乘倍**。`clamp` **不足以修**（10 包/帧 × 0.05 s = 2.3 m/帧，照样穿墙）——必须"每客户端每帧只应用一条 + 序号去重" |
| #6 明文 UDP（全局） | **确认**，按前提写入 §3 威胁模型（不是缺陷） |

**第一轮没有覆盖的**（本轮新增）：S3（反射放大 + 名额锁死）、S4（实体表/插值表无上限）、S5（弹丸堆积）、S6（`Refuse` 语义把瞬态写成永久）、S7（日志注入）、S8（远端弹归属/友军）、S9（`net_look` 死分支）、S10·S12（强转与常量语义）、S11（未 `connect()`）、S13（出生点）、S14（中继注册死路径）。

**第一轮建议的验证方式，本轮落地了两条**：它的"对抗性单测"与"黑盒压测"现在有可复算的原型 —— `netcode_probe.py mock move`（等价于"灌 N 包/秒，量位移与发数"）与 `mock reflect`（量放大比）。**没落地的那条**是"源地址伪造 + 中继 MITM"：两者都需要 raw socket 或第二台机器，超出"只做回环"的约束，留给隔离实验网。
