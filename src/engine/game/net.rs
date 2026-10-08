// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Game {
    /// 初始化网络环回演示：绑定环回 Server + 连入 Client，发起 Join
    pub(crate) fn init_network_demo() -> Result<NetworkDemo, String> {
        let server = Server::bind("127.0.0.1:0").map_err(|e| format!("bind 失败: {}", e))?;
        let addr = server.local_addr().map_err(|e| format!("local_addr 失败: {}", e))?;
        let client = Client::connect(addr).map_err(|e| format!("connect 失败: {}", e))?;
        client
            .send(&NetworkMessage::Join {
                player_id: 0,
                name: "local".into(),
                version: crate::net::SESSION_VERSION,
            })
            .map_err(|e| format!("发送 Join 失败: {}", e))?;
        Ok(NetworkDemo {
            server,
            client,
            seq: 0,
            last_log: 0.0,
        })
    }
    /// 服务器模式（RV3D_NET=server）：绑定好的 Server 交给 Game 托管，开始权威模拟 + 快照广播
    pub(crate) fn set_net_server(&mut self, server: Server) {
        log::info!("net: server mode active");
        self.net_server = Some(server);
    }
    /// 客户端模式（RV3D_NET=client）：连上的 Client 交给 Game 托管，开始输入上报 + 快照缓冲
    pub(crate) fn set_net_client(&mut self, client: Client) {
        log::info!("net: client mode active");
        self.net_client = Some(client);
    }
    /// main.rs 转发本帧开火意图（客户端模式随 Input 上报服务端）
    pub(crate) fn set_net_fire(&mut self, fire: bool) {
        self.net_fire_pending = fire;
    }
    /// 服务器模式最近一次客户端输入视角（main.rs 应用到相机，快照权威视角）
    pub(crate) fn net_look(&self) -> Option<(f32, f32)> {
        self.net_look
    }
    /// 网络对战模式步进（RV3D_NET=server|client；默认关闭，不破坏单机模式）。
    /// 与旧环回演示（update_net）互斥：演示用 RV3D_NET=1|demo，对战用 server|client。
    pub(crate) fn step_net(&mut self, camera: &Camera) {
        self.step_net_server(camera);
        self.step_net_client(camera);
        // 1 秒一条状态日志（联调/冒烟断言依据）
        if self.time - self.last_net_log >= 1.0 {
            self.last_net_log = self.time;
            match (&self.net_server, &self.net_client) {
                (Some(server), _) => {
                    log::info!(
                        "net: server clients={} seq={} npcs={}",
                        server.client_count(),
                        self.net_snap_seq,
                        self.npcs.len()
                    );
                }
                (None, Some(client)) => {
                    log::info!(
                        "net: client id={:?} seq={} entities={} own={:?} connected={} obj={} rule={}",
                        client.player_id(),
                        client.snapshot_seq(),
                        client.entities().len(),
                        client.own_state(),
                        client.is_connected(),
                        client.objective_state().len(),
                        client.objective_rule()
                    );
                }
                _ => {}
            }
        }
    }
    /// 服务器模式：收客户端输入应用（远端玩家）+ 超时清理 + 每 tick 广播快照
    pub(crate) fn step_net_server(&mut self, camera: &Camera) {
        let mut inputs: Vec<(std::net::SocketAddr, NetInput)> = Vec::new();
        let mut joined: Vec<u32> = Vec::new();
        let mut left: Vec<u32> = Vec::new();
        {
            let Some(server) = self.net_server.as_mut() else {
                return;
            };
            while let Ok(Some((msg, from))) = server.recv() {
                match msg {
                    NetworkMessage::Join { name, version, .. } => {
                        let _ = server.handle_join(from, name, version);
                        if let Some(id) = server.player_id_of(from) {
                            joined.push(id);
                        }
                    }
                    NetworkMessage::Input { input, .. } => inputs.push((from, input)),
                    // 🔴 2026-09-26：显式离场通知（客户端正常退出会发 Leave）。以前这条落在
                    // `_ => {}`：既不注销注册表、也不清 `net_players` ⇒ 快照里继续带着他。
                    NetworkMessage::Leave { .. } => {
                        if let Some(id) = server.unregister(from) {
                            left.push(id);
                        }
                    }
                    _ => {}
                }
            }
            // 断线基本处理：超过 SERVER_TIMEOUT 无包的客户端移除注册（重连为后续 TODO）
            let removed = server.timeout_clients(SERVER_TIMEOUT);
            if !removed.is_empty() {
                log::warn!("net: server 移除超时客户端: {:?}", removed);
                left.extend(removed);
            }
        }
        // 🔴 停止广播"注册表里已经没有的人"：以前 `timeout_clients` 的返回值**只用来打一行
        // 日志**，`self.net_players` 一条都不删 ⇒ 离场者永远以最后一帧的姿态留在每帧广播的
        // 快照里，而客户端对远端玩家实体是**无条件进画面**的 ⇒ 所有人看到一个站着不动的幽灵。
        // 判据取**服务器注册表**（唯一真源）：Leave / 超时 / 将来任何移除路径都自动覆盖。
        if let Some(server) = self.net_server.as_ref() {
            let alive = server.player_ids();
            if self.net_players.iter().any(|p| !alive.contains(&p.id)) {
                let before = self.net_players.len();
                self.net_players.retain(|p| alive.contains(&p.id));
                log::info!(
                    "net: 停止广播 {} 个已离场远端玩家（注册表剩 {} 个，离场 {:?}）",
                    before - self.net_players.len(),
                    alive.len(),
                    left
                );
            }
        }
        // Join 注册（借用解耦后执行）：服务器权威远端玩家实体（主机=红营视角，远端=蓝营）
        let spawn = self.net_spawn_point();
        for id in joined {
            if !self.net_players.iter().any(|p| p.id == id) {
                self.net_players.push(NetPlayer {
                    id,
                    pos: [spawn[0], 0.0, spawn[1]],
                    yaw: 0.0,
                    pitch: 0.0,
                    hp: 100.0,
                    alive: true,
                    last_rx: self.time,
                    fire_accum: 0.0,
                    last_fire: 0.0,
                });
                log::info!("net: server 远端玩家 #{id} 加入 @({:.0},{:.0})", spawn[0], spawn[1]);
            }
        }
        // 各远端玩家应用最新输入（移动 + 开火，服务器权威）
        for (from, input) in inputs {
            if let Some(id) = self.net_server.as_ref().and_then(|s| s.player_id_of(from)) {
                self.apply_net_player_input(id, input);
            }
        }
        // 远端玩家：死亡复活 + 位置贴地
        let spawn = self.net_spawn_point();
        for p in &mut self.net_players {
            if !p.alive && self.time - p.last_rx >= 4.0 {
                p.pos = [spawn[0], 0.0, spawn[1]];
                p.hp = 100.0;
                p.alive = true;
                p.last_rx = self.time;
                log::info!("net: server 远端玩家 #{} 复活 @({:.0},{:.0})", p.id, spawn[0], spawn[1]);
            }
            if p.alive {
                p.pos[1] = terrain_height_at(p.pos[0], p.pos[2]) + 1.6;
            }
        }
        // 每 tick 广播快照：本机玩家 + 全部 NPC + 远端玩家（同一 id 空间，远端用保留基址）
        self.net_snap_seq = self.net_snap_seq.wrapping_add(1);
        let cam_pos = camera.position();
        let player = PlayerState::new([cam_pos.x, cam_pos.y, cam_pos.z], camera.yaw);
        let mut npcs: Vec<NpcSnapshot> = self
            .npcs
            .iter()
            .map(|n| NpcSnapshot {
                id: n.id as u32,
                pos: n.position,
                facing: n.facing,
                hp: n.hp,
                team: match n.team {
                    Team::Red => 0,
                    Team::Blue => 1,
                },
                firing: if n.state_machine.state() == NpcState::Attack { 1 } else { 0 },
            })
            .collect();
        // 远端玩家 → 保留 id 区（NET_PLAYER_BASE + player_id），客户端据此渲染
        for p in &self.net_players {
            npcs.push(NpcSnapshot {
                id: NET_PLAYER_BASE + p.id,
                pos: p.pos,
                facing: p.yaw,
                hp: p.hp,
                team: 1, // 远端玩家统一蓝营
                firing: if self.time - p.last_fire < 0.2 { 1 } else { 0 },
            });
        }
        let snapshot = NetworkMessage::Snapshot {
            seq: self.net_snap_seq,
            time: self.time,
            player_id: 0,
            player,
            npcs,
        };
        if let Some(server) = self.net_server.as_ref() {
            let _ = server.broadcast(&snapshot, None);
            // 目标状态（据点归属/进度）广播：关卡系统启用时组包（归属码 0=中立/1=Red/2=Blue）。
            // 未启用关卡系统（obj_state=None）→ 空据点列表广播（客户端据此可知无目标）。
            if let Some(obj) = self.obj_state.as_ref() {
                let points = obj
                    .points
                    .iter()
                    .map(|p| {
                        let owner = match p.owner {
                            None => 0u8,
                            Some(crate::engine::ai::Team::Red) => 1,
                            Some(crate::engine::ai::Team::Blue) => 2,
                        };
                        (p.id.clone(), owner, p.progress)
                    })
                    .collect();
                let obj_msg = NetworkMessage::ObjectiveState {
                    seq: self.net_snap_seq,
                    time: self.time,
                    rule_kind: obj.rule.rule_kind().to_string(),
                    points,
                };
                let _ = server.broadcast(&obj_msg, None);
            }
        }
    }
    /// 客户端模式：握手重试 + 每 tick 上报输入/姿态 + 收快照进插值缓冲
    pub(crate) fn step_net_client(&mut self, camera: &Camera) {
        let Some(client) = self.net_client.as_mut() else {
            return;
        };
        // 握手：未确认时按 0.5s 重发 Join（UDP 尽力而为下唯一带重试的报文）
        client.retry_join("steel", std::time::Duration::from_millis(500));
        // 每 tick 上报本地输入/姿态
        self.net_input_seq = self.net_input_seq.wrapping_add(1);
        let input = NetInput {
            forward: self.move_forward,
            backward: self.move_backward,
            left: self.move_left,
            right: self.move_right,
            fire: self.net_fire_pending,
            yaw: camera.yaw,
            pitch: camera.pitch,
        };
        let _ = client.send(&NetworkMessage::Input {
            seq: self.net_input_seq,
            time: client.now() as f32,
            input,
        });
        // 收快照：进入实体插值表（位置平滑）
        while let Ok(Some((msg, _))) = client.recv() {
            if let Some(left_id) = client.handle_message(msg) {
                log::info!("net: 远端玩家 #{left_id} 离场（Leave），已从实体表移除");
            }
        }
        // 🔴 兜底清理：快照里连续 ENTITY_STALE_AFTER 秒没出现的实体一律删掉 —— `Leave` 可能
        // 丢包、服务端超时路径也可能不通知，而客户端对远端玩家实体是**无条件进画面**的
        // ⇒ 不清理就会留下"永久站在场上"的幽灵（2026-09-26 修）。
        let stale = client.prune_stale_entities(client.now(), crate::net::ENTITY_STALE_AFTER);
        if stale > 0 {
            log::info!("net: 清理 {} 个已离场远端实体（{}s 未出现在快照里）", stale, crate::net::ENTITY_STALE_AFTER);
        }
        // 自身上行权威校正（2026-08-25）：服务器端本远端实体位置与本地超 3m → 硬对齐（防漂移）
        if let Some(own) = client.player_id() {
            if let Some(st) = client.entity_state_at(NET_PLAYER_BASE + own, client.now()) {
                let dx = st.pos[0] - self.player_body.pos.x;
                let dz = st.pos[2] - self.player_body.pos.z;
                if dx * dx + dz * dz > 9.0 {
                    self.player_body.pos.x = st.pos[0];
                    self.player_body.pos.z = st.pos[2];
                    log::debug!("net: client 行正（位置校正 {:.1},{:.1}）", st.pos[0], st.pos[2]);
                }
            }
        }
        // 断线自动重连（2026-08-25）：超过 CLIENT_TIMEOUT 无数据 → 重置握手态，retry_join 续发
        if client.player_id().is_some() && client.snapshot_timeout() {
            log::warn!(
                "net: client 断线（{}s 无数据），自动重连中...",
                CLIENT_TIMEOUT.as_secs()
            );
            client.reset_connection();
        }
    }
    /// 服务端应用客户端输入：移动标志 + 开火（方向 = 输入视角）+ 记录视角供 main.rs 应用
    /// 远端玩家输入应用（服务器权威：移动该玩家 + 从该玩家眼睛开火）
    pub(crate) fn apply_net_player_input(&mut self, id: u32, input: NetInput) {
        let Some(idx) = self.net_players.iter().position(|p| p.id == id) else {
            return;
        };
        let (pi, dt, alive) = (idx, self.last_dt, self.net_players[idx].alive);
        if !alive {
            return;
        }
        // 移动：以 yaw 为前向（服务器统一步长，与本地玩家手感一致）
        let p = &mut self.net_players[pi];
        let speed = 4.6 * dt;
        let mut dx = 0.0f32;
        let mut dz = 0.0f32;
        let (sy, cy) = (input.yaw.sin(), input.yaw.cos());
        if input.forward {
            dx += sy * speed;
            dz += cy * speed;
        }
        if input.backward {
            dx -= sy * speed;
            dz -= cy * speed;
        }
        if input.left {
            dx += cy * speed;
            dz -= sy * speed;
        }
        if input.right {
            dx -= cy * speed;
            dz += sy * speed;
        }
        let nx = p.pos[0] + dx;
        let nz = p.pos[2] + dz;
        // 静态障碍 AABB 水平推开（盒体数少，线性扫描可接受）
        let (rx, rz) = resolve_circle_static(&self.world.bodies, nx, nz, 0.45);
        p.pos = [rx, 0.0, rz];
        p.yaw = input.yaw;
        p.pitch = input.pitch;
        p.last_rx = self.time;
        // 开火（服务器权威弹道：命中 NPC/友方实体走 self.fire 链路）
        p.fire_accum += dt;
        if input.fire && p.fire_accum >= 0.12 {
            p.fire_accum = 0.0;
            let eye = [p.pos[0], p.pos[1] + 1.2, p.pos[2]];
            let dir = glam::Vec3::new(
                input.pitch.cos() * input.yaw.sin(),
                input.pitch.sin(),
                input.pitch.cos() * input.yaw.cos(),
            );
            self.fire([eye[0], eye[1], eye[2]], [dir.x, dir.y, dir.z]);
            self.net_players[pi].last_fire = self.time;
        }
    }
    /// 远端玩家出生点（蓝营侧固定点：主机=红营出生区对面）
    pub(crate) fn net_spawn_point(&self) -> [f32; 2] {
        [-110.0, 60.0]
    }
    /// 网络环回演示：server 收包回环广播，client 发包/收包做远端插值；不参与帧率逻辑
    pub(crate) fn update_net(&mut self, camera: &Camera) {
        let Some(demo) = &mut self.net_demo else {
            return;
        };
        // 服务器收包：Join 分配 id 回 ack；其余消息回环广播给所有客户端（含发送者）
        while let Ok(Some((msg, from))) = demo.server.recv() {
            match &msg {
                NetworkMessage::Join { name, .. } => {
                    let _ = demo.server.handle_join(from, name.clone(), crate::net::SESSION_VERSION);
                }
                _ => {
                    let _ = demo.server.broadcast(&msg, None);
                }
            }
        }
        // 客户端：每帧发送自身位置
        demo.seq = demo.seq.wrapping_add(1);
        let pos = camera.position();
        let player_id = demo.client.player_id().unwrap_or(0);
        let _ = demo.client.send(&NetworkMessage::Position {
            player_id,
            seq: demo.seq,
            state: PlayerState::new([pos.x, pos.y, pos.z], 0.0),
        });
        // 客户端收包：Join 确认 + 回环 Position（进入远端插值缓冲）
        while let Ok(Some((msg, _))) = demo.client.recv() {
            demo.client.handle_message(msg);
        }
        // 每秒一条日志：远端玩家数与插值采样
        if self.time - demo.last_log >= 1.0 {
            demo.last_log = self.time;
            let t = demo.client.now();
            let n = demo.client.remote_players().len();
            let sample = demo
                .client
                .remote_players()
                .values()
                .next()
                .map(|r| r.state_at(t));
            log::info!("net: remote_players={} sample={:?}", n, sample);
        }
    }
    /// 正常退出时通知服务端"我走了"（best-effort，UDP 同步发送，退出前能发出去）。
    ///
    /// 🔴 2026-09-26 加：不发的话服务端要等 `SERVER_TIMEOUT`(5s) 才摘掉我们，而这 5 秒里
    /// **每个客户端都还看得见我们站在原地**（远端玩家实体是**无条件进画面**的）。
    /// 非客户端模式（单机/服务端）下这是个空操作。
    pub(crate) fn send_leave(&mut self) {
        let Some(client) = self.net_client.as_ref() else {
            return;
        };
        let Some(id) = client.player_id() else {
            return;
        };
        if let Err(e) = client.send(&NetworkMessage::Leave { player_id: id, reason: 0 }) {
            log::warn!("net: 离场通知发送失败（服务端会在 {SERVER_TIMEOUT:?} 后按超时清理）：{e}");
        }
    }
    /// 玩家弹 vs 网络远端玩家命中（胶囊近似：水平 0.5m + 高度 1.8m）
    pub(crate) fn hit_net_player(&mut self, p: &Projectile) -> bool {
        let (px, pz, ph) = (p.position[0], p.position[2], p.position[1]);
        for q in self.net_players.iter_mut() {
            if !q.alive {
                continue;
            }
            let dx = px - q.pos[0];
            let dz = pz - q.pos[2];
            let dy = ph - (q.pos[1] - 1.6); // 脚底高
            if dx * dx + dz * dz <= 0.25 && dy >= 0.0 && dy <= 1.8 {
                let dmg = p.damage_at_distance();
                q.hp -= dmg;
                if q.hp <= 0.0 {
                    q.alive = false;
                    q.last_rx = self.time;
                    self.hud.push_kill(format!("击杀联机玩家 #{}", q.id));
                    log::info!("net: server 远端玩家 #{} 被击杀\n", q.id);
                }
                return true;
            }
        }
        false
    }
}
