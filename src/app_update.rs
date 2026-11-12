// 由 src/lib.rs 拆出（2026-11-12，纯移动 + 仅加 pub(crate)）：每帧的**逻辑更新**。
//
// 这是游戏规则真正跑起来的地方：输入 → 玩家移动/射击 → 波次与 AI → 目标点 → HUD 状态。
// 先整体搬出（`lib.rs` 曾 6320 行），后续可按"玩家/武器/波次/HUD"再切内部几段。
use std::time::Instant;

use crate::engine::camera::CameraMode;
use crate::{env_f32, env_truthy, last_fg_pid, my_pid, GameApp, GameState, Particle};

impl GameApp {

    /// 更新逻辑（每帧调用）
    pub(crate) fn update(&mut self) {
        // RV3D_NPC_CAM=<i>：把调试机位吸附到第 i 个 NPC 的斜前方（坐标由程序算）。
        let npc_cam = crate::syscfg::cfg("RV3D_NPC_CAM")
            .and_then(|s| s.parse::<usize>().ok());
        // RV3D_CAM=fly:x,y,z:yaw_deg,pitch_deg：调试固定机位（地图/场景检查用）
        // 两个变量都给时**必须一起进这个分支**（否则 cam_override 为 None 时分支根本不跑），
        // 且 NPC 机位要在 cam_override **之后**应用才不会被覆盖。
        if self.cam_override.is_some() || npc_cam.is_some() {
            // 🔴 固定机位分支 early-return 早于 update() 的 autostart 段
            // （2026-10-21 画质取景时发现：只给 RV3D_CAM 时永远停在菜单，截到的全是
            // "PRESS ANY KEY" 覆盖层）。调试机位需要真实游戏态才有 NPC/波次/枪模。
            if self.game.state() == GameState::StartMenu && env_truthy("RV3D_AUTOSTART") {
                self.game.on_any_key();
            }
            // 仍推进帧时间/HUD FPS（避免调试机位下 HUD 恒 0 显像为“卡死”）
            let now = Instant::now();
            let dt = now.duration_since(self.last_frame).as_secs_f32();
            self.last_frame = now;
            if dt > 1e-6 {
                self.last_fps = 1.0 / dt.min(0.1) as f64;
            }
            self.anim_clock += dt.min(0.1);
            self.camera.mode = CameraMode::Flight;
            // RV3D_NPC_CAM=<i>：把调试机位**吸附到第 i 个 NPC 的斜前方**，坐标由程序算。
            // 存在的理由：我按 `RV3D_NPC_POS` 打出的坐标手算过 13 次机位，全部失败
            // （见 docs/PROGRESS.md 第 8–16 轮）。这些坐标程序本来就有 —— 该由程序算，
            // 不该由我猜。`pitch` 取正 = 低头（与鼠标 dy>0 同号，见铁律 C）。
            if let Some((p, yaw, pitch, yaw_rate)) = self.cam_override {
                self.camera.set_flight_pos(p);
                if yaw_rate != 0.0 {
                    // 受控匀速旋转（运动向量解析验证专用）。用**帧内真实 dt** 累加，
                    // 与着色器里用的 `view` 矩阵同源 ⇒ 期望值可以精确算出来。
                    self.cam_spin += yaw_rate * dt;
                    self.camera.yaw = yaw + self.cam_spin;

                } else {
                    self.camera.yaw = yaw;
                }
                self.camera.pitch = pitch;
            }
            // 🔴 NPC 机位**必须放在 cam_override 之后**：否则同时给 `RV3D_CAM` 时，
            // 上面那段 fly: 会把它整个覆盖掉 —— 第 16 轮就是这么白跑一次的
            // （无告警、没报错，画面一直是 fly: 那个远景）。
            if let Some(i) = npc_cam {
                // 🔴 本分支的 early-return **早于** `update()` 里调用 `on_any_key()` 的
                // autostart 段 ⇒ 不补这一步，游戏会停在菜单态、**NPC 永远不会生成**。
                // 13 次取景失败里有相当一部分就是这个（我一直在对着没有士兵的世界摆机位）。
                if self.game.state() == GameState::StartMenu {
                    self.game.on_any_key();
                }
                match self.game.npcs.get(i) {
                    Some(n) => {
                        // **由程序挑一个不被墙挡的方向**：四个正交方向各 4m 做可站立判定，
                        // 取第一个通过的。手算方向失败过 13 次，而判据本来就在引擎里
                        // （`GameState::standable`，与单位能否移动同一条口径）。
                        // 偏移量与 yaw 的对应：forward = (-sin, 0, -cos) ⇒ 相机在 +Z 看 -Z 时 yaw=0。
                        const DIRS: [(f32, f32, f32); 4] = [
                            (0.0, 4.0, 0.0),
                            (0.0, -4.0, 180.0),
                            (4.0, 0.0, 90.0),
                            (-4.0, 0.0, -90.0),
                        ];
                        let (bx, bz) = (n.position[0], n.position[2]);
                        // 判据从"一个点"升级为"**一条线**"：沿 相机→NPC 线段均匀取 6 个采样点，
                        // 全部可站立才接受该方向。只测相机那一格是不够的 —— 墙可以横在相机与
                        // NPC 之间，相机站得再合法也看不见人（第 17 轮定位到的缺口）。
                        let los_clear = |ox: f32, oz: f32| -> bool {
                            (1..=6).all(|k| {
                                let f = k as f32 / 7.0;
                                self.game.standable(bx + ox * f, bz + oz * f)
                            })
                        };
                        let (dx, dz, yaw) = DIRS
                            .iter()
                            .copied()
                            .find(|(dx, dz, _)| los_clear(*dx, *dz))
                            .unwrap_or((0.0, 4.0, 0.0));
                        self.camera
                            .set_flight_pos(glam::Vec3::new(bx + dx, n.position[1] + 1.6, bz + dz));
                        self.camera.yaw = yaw.to_radians();
                        self.camera.pitch = 10.0_f32.to_radians();
                        if crate::syscfg::is_set("RV3D_NPC_POS") {
                            // 假设验证（第 21 轮）：NPC 的 y 恒为 0，而该处地形可能高于 0
                            // ⇒ 士兵被地形埋掉。用**与地形渲染同一个** `terrain_height`，
                            // 不另写一套判据。差 > 1m 即假设成立。
                            let th = crate::engine::renderer::terrain_height(bx, bz);
                            // 一致性对照（第 24 轮）：在**同一组采样点**上各跑一次
                            // `standable`（导航网格）与 `point_in_body`（建筑刚体 AABB）。
                            // 两者不一致 ⇒ 导航网格缺建筑，也就是未结案 4 的根因，
                            // 同时解释了"六个采样点全 standable 却仍看不到人"。
                            let (mut n_nav, mut n_body, mut n_mismatch) = (0u32, 0u32, 0u32);
                            for k in 1..=6 {
                                let f = k as f32 / 7.0;
                                let (px, pz) = (bx + dx * f, bz + dz * f);
                                let nav = self.game.standable(px, pz);
                                let body = self.game.point_in_body(px, 1.0, pz);
                                if nav {
                                    n_nav += 1;
                                }
                                if body {
                                    n_body += 1;
                                }
                                if nav && body {
                                    n_mismatch += 1;
                                }
                            }
                            // 最后一道未验证环节：把**相机自己的读数**打出来。
                            // 前面所有推理都建立在"机位就是 (bx+dx, +1.6, bz+dz)"这个假设上，
                            // 而它是本轮唯一还没被验证过的东西（pitch 符号、NPC 是否生成、
                            // 是否被埋、是否尸体、是否有墙 —— 都已逐个否掉）。
                            let cp = self.camera.position();
                            let cf = self.camera.forward();
                            log::info!(
                                "npc_cam: 目标 #{} npc=({:.1},{:.1},{:.1}) hp={:.0} state={:?} 地形高={th:.1} 机位=({:.1},{:.1},{:.1}) offset=({dx:.0},{dz:.0}) | 采样6点: 导航可走={n_nav} 建筑体内={n_body} 两者矛盾={n_mismatch} | 相机读数=({:.1},{:.1},{:.1}) 朝向=({:.2},{:.2},{:.2})",
                                n.id,
                                n.position[0], n.position[1], n.position[2],
                                n.hp,
                                n.state_machine.state(),
                                bx + dx, n.position[1] + 1.6, bz + dz,
                                cp.x, cp.y, cp.z,
                                cf.x, cf.y, cf.z
                            );
                            // 🔴 2026-09-12 第④条补：**离相机最近的 3 个 NPC**。
                            // 理由：`npc=` 与 `机位=` 两条读数各自都对（#8 在 x=65.0，
                            // 机位在 x=61.0，朝向 +X），但实机截图里 4m 正前方**没有士兵**，
                            // 画面里那个人在 ~19m 外。两者不能同时成立 ⇒
                            // 直接问"相机附近到底有没有人"，而不是继续推坐标。
                            let mut near: Vec<(f32, usize)> = self
                                .game
                                .npcs
                                .iter()
                                .enumerate()
                                .map(|(k, q)| {
                                    let d = glam::Vec3::new(
                                        q.position[0] - cp.x,
                                        q.position[1] - cp.y,
                                        q.position[2] - cp.z,
                                    )
                                    .length();
                                    (d, k)
                                })
                                .collect();
                            near.sort_by(|a, b| {
                                a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                            });
                            let s: Vec<String> = near
                                .iter()
                                .take(3)
                                .map(|(d, k)| format!("#{k} d={d:.1}m"))
                                .collect();
                            log::info!("npc_cam: 距相机最近 3 人 = {}", s.join(" / "));
                        }
                    }
                    // 找不到**必须**打日志。本轮的核心教训就是"静默失败"：相机没生效、
                    // NPC 不存在、pitch 反了，三者都只有画面能看出来，而画面又要靠它们
                    // 才能取到 —— 于是必须让失败自己浮出来。
                    None => log::warn!(
                        "npc_cam: 第 {i} 个 NPC 不存在（当前 {} 个），机位未覆盖",
                        self.game.npcs.len()
                    ),
                }
            }
            // HUD 那行大号青色 FPS 由 `game.update()` 里的滑动窗口算出（game.rs
            // `self.hud.fps = frames / window_secs`），而本分支直接 return、不跑玩法帧，
            // 于是 hud.fps 永远停在初值 0 —— 表现成"游戏 0 帧"，而同一帧 HUD 上
            // `VULKAN: 150 FPS`（渲染器自己的计数）和 logs 里的 fps=146~152 都是正常的。
            // 调试机位下用刚量到的 last_fps 直接补上，别让取证截图显示一个假 0。
            self.game.hud.fps = self.last_fps as f32;
            // 🔴 剔除眼位必须在这里设（2026-09-12 第④条修）：**本分支下面立刻 return**，
            // 放在常规更新路径里的赋值根本跑不到（第一次就栽在这，实测 npc 仍是 288）。
            // `npc_occluded` 原本硬取 `player_eye()`；调试相机移离玩家后，
            // "相机眼前的人"会被按"从玩家位置看不到"整片剔掉。
            self.game.cull_eye_override = Some(self.camera.position());
            return;
        }
        // 枪械检视模式：不跑游戏逻辑，仅 Orbit 相机绕枪模（鼠标拖拽旋转/滚轮缩放，
        // 事件处理已有 orbit 控制）；首次进入设置相机朝向。
        // 用 `if let` 绑住槽位号，替掉原来"外层 is_some() + 日志里 unwrap()"的写法：
        // 那对组合靠两处代码之间的距离保证不 panic，改一处就可能漏另一处。
        if let Some(inspect_w) = self.inspect_weapon {
            self.camera.mode = CameraMode::Orbit;
            if !self.inspect_armed {
                self.inspect_armed = true;
                self.camera.target = glam::Vec3::new(0.0, 1.0, 0.0);
                self.camera.yaw = std::f32::consts::FRAC_PI_2; // 正侧视：枪口朝左
                self.camera.pitch = 0.08;
                self.camera.fov = 45.0_f32.to_radians();
                // 产品照式取景：远距离 + 长焦（弱透视，近远端大小接近，同真枪照片）
                self.camera.distance = 2.0;
                if let Some(n) = self.inspect_weapon {
                    if let Some(spec) = crate::engine::weapon_data::spec_by_number(n) {
                        if let Some(gm) = crate::engine::guns::gun_mesh_by_key(spec.key) {
                            let mut mn = [f32::MAX; 3];
                            let mut mx = [f32::MIN; 3];
                            for v in &gm.verts {
                                for i in 0..3 {
                                    mn[i] = mn[i].min(v.pos[i]);
                                    mx[i] = mx[i].max(v.pos[i]);
                                }
                            }
                            let e = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
                            let diag = (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt();
                            // 距离 = 4.5× 对角线：近端/远端大小差 <25%（之前 1.36m 时差达 2 倍，
                            // 广角微距式变形就是 1-4/1-5 里“这不像枪”的根源）
                            let dist = (diag * 4.5).max(2.0);
                            self.camera.distance = dist;
                            // 长焦 fov：按距离反推，保证整枪入画（1.15 余量）
                            self.camera.fov = (2.0 * ((diag * 0.5 * 1.15) / dist).atan())
                                .to_degrees()
                                .to_radians();
                            log::info!(
                                "inspect: bbox=[{:.3},{:.3},{:.3}]..[{:.3},{:.3},{:.3}] ext=[{:.3},{:.3},{:.3}] diag={:.3} dist={:.3}",
                                mn[0], mn[1], mn[2], mx[0], mx[1], mx[2],
                                e[0], e[1], e[2], diag, self.camera.distance
                            );
                        }
                    }
                }
                log::info!(
                    "inspect: 枪械检视模式（武器 #{}）——拖拽旋转 / 滚轮缩放",
                    inspect_w
                );
            }
            // 🔬 取证用：检视模式**自动旋转**（弧度/秒），便于无人值守拍多角度。
            // 缺省不转（`RV3D_INSPECT_SPIN` 未设 ⇒ 行为与之前逐字节相同）。
            // 用帧内真实 dt 累加，与着色器用的 view 矩阵同源 ⇒ 角度可精确复算。
            if let Some(spin) = crate::syscfg::parse::<f32>("RV3D_INSPECT_SPIN") {
                if spin.is_finite() && spin.abs() > 1e-6 {
                    // 这里是取证开关，**不需要精确时间基**：用固定 1/60 s 步长即可
                    // （`dt` 不在该作用域内，而为此把时间基一路传进来不值得）。
                    // 目的只是"拍几张不同角度的图"，角度偏差几度无影响。
                    self.camera.yaw += spin * (1.0 / 60.0);
                }
            }
            return;
        }
        // RV3D_AUTOSTART=1：测试用自动开始（绕过键盘，进 Playing 复现/冒烟）
        use std::sync::atomic::{AtomicBool, Ordering};
        static AUTO_STARTED: AtomicBool = AtomicBool::new(false);
        if !AUTO_STARTED.swap(true, Ordering::SeqCst)
            && env_truthy("RV3D_AUTOSTART")
        {
            let st = self.game.state();
            if st == GameState::StartMenu || st == GameState::LoadingMap {
                log::info!("autostart: RV3D_AUTOSTART=1 自动开始");
                self.game.on_any_key();
            }
            // RV3D_SWITCH_WEAPON=n：进入后自动切到 n 号武器（复现切枪崩溃用）；
            // RV3D_SWITCH_WEAPON_AFTER=秒：延迟切枪（模拟玩一会儿再切）
            let after = env_f32("RV3D_SWITCH_WEAPON_AFTER");
            let (target, switch_at) = match crate::syscfg::cfg("RV3D_SWITCH_WEAPON") {
                Some(n) => (n.parse::<usize>().ok(), after.unwrap_or(0.0)),
                None => (None, 0.0),
            };
            if let Some(n) = target {
                if switch_at <= 0.0 {
                    log::info!("autostart: 自动切枪 #{}", n);
                    self.game.switch_weapon(n.saturating_sub(1));
                } else {
                    self.switch_weapon_at = Some((n, self.anim_clock + switch_at));
                }
            }
        }
        // 延迟自动切枪（测试用）
        if let Some((n, at)) = self.switch_weapon_at {
            if self.anim_clock >= at && self.game.state() == GameState::Playing {
                log::info!("autostart: 延迟切枪 #{}", n);
                self.game.switch_weapon(n.saturating_sub(1));
                self.switch_weapon_at = None;
            }
        }
        // RV3D_DIAG_NPC_FRONT=1：把 npc[0] 放到玩家正前方 20m 固定（弹道诊断隔离实验）
        static DIAG_NPC_FRONT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *DIAG_NPC_FRONT.get_or_init(|| {
            env_truthy("RV3D_DIAG_NPC_FRONT")
        }) && self.game.state() == GameState::Playing
        {
            // 相机 yaw=0 时 forward 方向（与 fire 弹道同源），NPC 放前方 20m
            let fwd = self.camera.forward();
            let pos = self.camera.position();
            let nx = pos.x + fwd.x * 20.0;
            let nz = pos.z + fwd.z * 20.0;
            let ny = crate::engine::renderer::terrain_height_at(nx, nz);
            self.game.diag_place_npc([nx, ny, nz]);
        }
        // RV3D_AUTOFIRE=1：自动开火（诊断射击链路：fire 是否发射、弹道是否命中）
        static AUTO_FIRE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *AUTO_FIRE.get_or_init(|| env_truthy("RV3D_AUTOFIRE"))
            && self.game.state() == GameState::Playing
        {
            self.fire_requested = true;
        }
        // 同步光标捕获状态（Playing + 聚焦 = 捕获；菜单/结算/失焦 = 释放）
        self.sync_cursor();

        // 计算帧时间差
        let now = Instant::now();
        let delta_time = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        // 未夹取的**真实帧间隔**（微秒）：只喂给性能日志。上面那句 `min(0.1)` 是给玩法用的
        // （防卡顿大跳），拿它去量帧率会把 1 秒的长卡记成 100ms；而窗口帧率本身由
        // `perf_log` 按"窗口内帧数 / 窗口时长"算，不受这里影响。
        self.frame_dt_us = (delta_time * 1e6) as u64;

        // 确保 delta_time 不会太大（防止卡顿时大跳）
        let delta_time = delta_time.min(0.1);
        if delta_time > 1e-6 {
            self.last_fps = 1.0 / delta_time as f64;
        }
        // NPC 动画时钟（步态/后坐相位）与尸体老化
        self.anim_clock += delta_time;
        for c in self.corpses.iter_mut() {
            c.3 += delta_time;
        }
        self.corpses.retain(|c| c.3 < 10.0); // 尸体 10 秒后消退
        while self.corpses.len() > 20 {
            self.corpses.remove(0); // 上限 20 具（NPC 槽位容量见 renderer.rs::MAX_NPC_INSTANCES = 3072）
        }
        // 粒子推进：弹壳重力下落 + 落地停止；超龄移除
        for p in self.particles.iter_mut() {
            p.age += delta_time;
            if p.kind == 1 {
                p.vel[1] -= 18.0 * delta_time; // 弹壳重力
                p.pos[0] += p.vel[0] * delta_time;
                p.pos[1] += p.vel[1] * delta_time;
                p.pos[2] += p.vel[2] * delta_time;
                if p.pos[1] <= 0.05 {
                    p.pos[1] = 0.05;
                    p.vel = [0.0, 0.0, 0.0];
                    p.life = p.life.min(0.4); // 落地后最多停留 0.4s
                }
            }
        }
        self.particles.retain(|p| p.age < p.life);
        while self.particles.len() > 48 {
            self.particles.remove(0); // 上限 48 颗粒子
        }

        // 更新相机（双模式：轨道/飞行，含惯性速度与边界 clamp）
        self.camera.update(&self.key_state, delta_time);
        // 开镜瞄准：FOV 平滑过渡（70° 腰射 → 55° 开镜，步枪 ADS 轻微收窄而非狙击 zoom）
        // + 锚点混合度（枪模腰射右下 → 开镜居中，0.2s 指数平滑）
        let ads_target = if self.ads_active {
            55.0_f32.to_radians()
        } else {
            70.0_f32.to_radians()
        };
        let fov_delta = ads_target - self.camera.fov;
        if fov_delta.abs() > 1e-4 {
            self.camera.fov += fov_delta * (1.0 - (-10.0 * delta_time).exp());
        }
        let ads_blend_target = if self.ads_active { 1.0 } else { 0.0 };
        self.ads_blend +=
            (ads_blend_target - self.ads_blend) * (1.0 - (-10.0 * delta_time).exp());
        // 开镜状态硬化：非 Playing/菜单/设置打开时强制复位（防右键状态卡死 → 准星变小/消失）
        let ads_valid = self.ads_active
            && self.camera.mode == CameraMode::FirstPerson
            && self.game.state() == GameState::Playing
            && !self.game.settings_open()
            && !self.game.hud.esc_menu_open;
        self.game.hud.ads = ads_valid;
        // 🔴 剔除眼位（2026-09-12 第④条修）：`npc_occluded` 原本硬取 `player_eye()`。
        // 正常玩法相机就在玩家眼位 ⇒ 下面的距离判定为 0 ⇒ **恒为 None，行为完全不变**。
        // 只有 `RV3D_CAM` / `RV3D_NPC_CAM` 把相机移离玩家时（> 1m）才改用相机 ——
        // 否则"相机眼前的人"会被按"从玩家位置看不到"整片剔掉
        // （实测关剔除前 npc=288 ≈16 人，开后 4590 = 255 人 × 18 段）。
        let cam_pos = self.camera.position();
        let eye_pos = self.game.player_eye();
        self.game.cull_eye_override = if cam_pos.distance(eye_pos) > 1.0 {
            Some(cam_pos)
        } else {
            None
        };
        // 小地图朝向（旋转地图使玩家前方朝上）
        self.game.hud.mm_yaw = self.camera.yaw;
        if !ads_valid {
            self.ads_active = false;
        }

        // 更新游戏逻辑（物理、武器、AI 等）
        // 先把本帧开火意图转发给网络层（客户端模式随 Input 上报服务端）
        self.game.set_net_fire(self.fire_requested);
        // V3.0 散射：开镜时散布缩小到 30%（腰射 100%）
        self.game.set_spread_scale(1.0 - self.ads_blend * 0.7);
        self.game.update(delta_time, &self.camera);

        // 基准挂钩：RV3D_BENCH_YAW / RV3D_BENCH_PITCH（度）每帧强制相机朝向，
        // 供性能基准固定视角用（与 RV3D_NPC_SCALE / RV3D_STRESS_AI 同类的测试环境变量，
        // 不设置则完全不影响正常游玩）。鼠标/后坐力每帧会被覆盖，基准时无需 bot 拖视角。
        if let Some(yaw) = crate::syscfg::cfg("RV3D_BENCH_YAW") {
            if let Ok(y) = yaw.parse::<f32>() {
                self.camera.yaw = y.to_radians();
            }
        }
        if let Some(pitch) = crate::syscfg::cfg("RV3D_BENCH_PITCH") {
            if let Ok(p) = pitch.parse::<f32>() {
                self.camera.pitch = p.to_radians().clamp(
                    -crate::engine::camera::PITCH_LIMIT,
                    crate::engine::camera::PITCH_LIMIT,
                );
            }
        }

        // 第一人称：玩家身体位置 → 相机眼睛（FP 相机不自己移动），并同步灵敏度
        if self.camera.mode == CameraMode::FirstPerson {
            // 爆炸震屏：本帧抖动偏移叠加到眼睛位置（无震屏时偏移为 0）
            let mut eye = self.game.player_eye();
            let (sx, sz) = self.game.camera_shake_offset();
            eye.x += sx;
            eye.z += sz;
            self.camera.set_first_person_eye(eye);
            self.camera.set_mouse_sens(self.game.sensitivity_rads());
        }

        // 开火：按开火模式分发（Semi=edge 单发 / Burst3=edge 三连发 / Auto=按住连发）。
        // 按住状态 fire_requested 保持 true，由武器 fire_cooldown 控制射速。
        let pos = self.camera.position();
        let dir = self.camera.forward();
        let mut fired = 0u32;
        match self.game.fire_mode() {
            crate::engine::game::FireMode::Semi => {
                if self.fire_edge {
                    let ok = self
                        .game
                        .fire_player([pos.x, pos.y, pos.z], [dir.x, dir.y, dir.z]);
                    if ok {
                        fired = 1;
                        self.last_shot_at = self.anim_clock;
                    }
                }
            }
            crate::engine::game::FireMode::Burst2 | crate::engine::game::FireMode::Burst3 => {
                if self.fire_edge {
                    // 发数由档位决定（双发=2 / 三连发=3），共用同一条连打路径
                    fired = self.game.fire_burst_player(
                        [pos.x, pos.y, pos.z],
                        [dir.x, dir.y, dir.z],
                        self.game.fire_mode().burst_rounds(),
                    );
                    if fired > 0 {
                        self.last_shot_at = self.anim_clock;
                    }
                }
            }
            crate::engine::game::FireMode::Auto => {
                if self.fire_requested {
                    let ok = self
                        .game
                        .fire_player([pos.x, pos.y, pos.z], [dir.x, dir.y, dir.z]);
                    if ok {
                        fired = 1;
                        self.last_shot_at = self.anim_clock;
                    }
                }
            }
        }
        self.fire_edge = false;
        // 枪口焰 + 弹壳粒子（每实际发射一发生成一组）
        for _ in 0..fired {
            let muzzle = [
                pos.x + dir.x * 0.5,
                pos.y - 0.25,
                pos.z + dir.z * 0.5,
            ];
            self.particles.push(Particle {
                pos: muzzle,
                vel: [0.0, 0.0, 0.0],
                age: 0.0,
                life: 0.09,
                size: 0.18,
                tint: [1.0, 0.75, 0.25, 1.0], // 橙黄枪口焰
                kind: 0,
            });
            self.particles.push(Particle {
                pos: muzzle,
                vel: [dir.z * 1.5 + 0.4, 2.2, -dir.x * 1.5], // 侧向抛出
                age: 0.0,
                life: 1.4,
                size: 0.06,
                tint: [0.72, 0.55, 0.18, 1.0], // 黄铜弹壳
                kind: 1,
            });
        }

        // 伤害飘字：本帧命中伤害入列（0.6s 衰减淡出）。
        // 同帧同值合并（霰弹一次开火 8 弹丸命中只显示一条伤害），
        // 上限 3 条滚动——超出丢最旧，新伤害补进来（不出现"一次命中刷屏"）。
        {
            let mut seen = std::collections::HashSet::new();
            for dmg in self.game.take_hit_damages() {
                if seen.insert(dmg.to_bits()) {
                    self.hit_damage_popups.push((dmg, 0.6));
                }
            }
            if self.hit_damage_popups.len() > 3 {
                let overflow = self.hit_damage_popups.len() - 3;
                self.hit_damage_popups.drain(0..overflow);
            }
        }
        // 衰减（iter_mut 可修改）→ 过滤（retain 只读判断，闭包参数为 &T）
        for (_, t) in self.hit_damage_popups.iter_mut() {
            *t -= delta_time;
        }
        self.hit_damage_popups.retain(|item| item.1 > 0.0);
        // 命中火花：本帧命中点在目标处生成小火花粒子（受击反馈增强）
        for hp in self.game.take_hit_points() {
            for _ in 0..5 {
                self.particles.push(Particle {
                    pos: hp,
                    vel: [
                        (hp[0] * 13.7).fract() * 2.0 - 1.0,
                        ((hp[0] + hp[2]) * 7.3).fract() * 1.4,
                        (hp[2] * 11.3).fract() * 2.0 - 1.0,
                    ],
                    age: 0.0,
                    life: 0.18,
                    size: 0.03,
                    tint: [1.0, 0.85, 0.3, 1.0], // 橙黄火花
                    kind: 0,
                });
            }
        }
        // 武器后坐力：取走本帧开火累计的 kick 施加到相机（指数衰减由 camera.update 处理）
        let (kick_pitch, kick_yaw) = self.game.drain_kick();
        if kick_pitch != 0.0 || kick_yaw != 0.0 {
            self.camera.add_recoil(kick_pitch, kick_yaw);
        }

        // S9：已删掉 net_look 死分支。原注释写明"客户端输入视角驱动本机相机"，但全仓没有
        // 任何写入点（声明与实现不符）；若按注释接线，同网段客户端就能强行转动房主视角。

        // 第一人称枪摆动状态积分（2026-09-01）。
        // 速度取自**玩家脚底的实际位移 / dt**，不用 `game.player_speed()`：后者读
        // PlayerBody.vel，而玩家移动走的是 `PlayerBody::try_move()`（直接改 pos，
        // 从不写 vel）→ vel 恒为 0 → 旧摆动分支实际上一次都没执行过。这里改成
        // 自带估计后，摆动不再依赖那个通道（game.rs/physics.rs 的 vel 修复另行提出）。
        {
            let p = self.game.player_pos();
            // dt 下限 1e-4 s：卡帧后 delta_time 被 clamp 到 0.1，正常帧约 6 ms；
            // 只有 0（同一时刻重复调用）会越过硬下限，此时按静止处理
            let dt = delta_time.max(1e-4);
            // 前向取**水平投影后归一化**：与 game.rs move_first_person 计算位移用的
            // 同一个基向量一致（俯仰时 forward 含 y 分量，直接点乘水平位移会低估前向
            // 速度，抬头/低头时前向摆动会莫名变小）。pitch 被 clamp 在 ±89°，
            // 水平分量最小 cos(89°)=0.0175，归一化不会退化。
            let f = self.camera.forward();
            let fwd = glam::Vec3::new(f.x, 0.0, f.z).normalize_or_zero();
            let right = self.camera.right();
            self.gun_sway.tick(dt, p, right, fwd, fired > 0, self.game.sprinting());
        }

        // 相机参数日志（默认 1 秒一条；`RV3D_NPC_POS=1` 时跟随 `RV3D_NPC_POS_HZ`）。
        // 🔴 2026-09-25：注入 harness 的瞄准环**拿这一行做回读**（注入像素 → 读回 yaw/pitch →
        // 再算误差），1 Hz 的回报让每轮 0.5s 的闭环经常读到同一行 ⇒ 重复注入 ⇒ 过冲/假收敛。
        // 同频到 10 Hz 之后，闭环才真的闭合。判据见 `game::diagnostic_period_secs` 的文档。
        if self.last_cam_log.elapsed().as_secs_f32()
            >= crate::engine::game::diagnostic_period_secs()
        {
            let (yaw, pitch, dist) = self.camera.orbit_params();
            // `spread` = 腰射准星扩散（第⑤条）。打在这里是为了**能脱离截图做验收** ——
            // 像素测量会被"两次运行场景不同 / 蹲下相机高度不同"混杂（第 65、76 轮实测），
            // 而这个值是确定的：站立 0.30 / 蹲 0.18 / 趴 0.10（+冲刺 +开火）。
            // 🔴 2026-09-13：加上**输入状态**（用户实测"鼠标抓不住、只有左右键能用"）。
            // 原先这行没有这几项，导致我只能靠推理猜是哪一环挂了 —— 按教训 20，
            // 卡住就去加埋点：这一行现在能直接区分
            //   ① focused=false ⇒ 窗口没拿到焦点 ⇒ 根本不抓（want 的第一个条件）
            //   ② captured=false ⇒ 抓取调用失败
            //   ③ locked=true  ⇒ 走了 Locked（Windows 上等于视角失效）
            //   ④ dragging     ⇒ 未捕获时的拖拽转视角路径有没有被置位
            log::info!(
                "cam: yaw={:.1} pitch={:.1} dist={:.1} mode={:?} spread={:.2} \
                 focus={} cap={} lock={} drag={} rdrag={} absbase={} \
                 fgpid={} mypid={} \
                 mouse={} eaten={} tp={} at=({:.0},{:.0}) cyc={} upd={} ren={}",
                yaw.to_degrees(),
                pitch.to_degrees(),
                dist,
                self.camera.mode,
                self.game.crosshair_spread(),
                self.focused,
                self.cursor_captured,
                self.cursor_locked,
                self.dragging,
                self.right_dragging,
                self.abs_baseline_valid,
                last_fg_pid().load(std::sync::atomic::Ordering::Relaxed),
                my_pid(),
                self.cursor_evt_count,
                self.cursor_evt_eaten,
                self.cursor_evt_teleport,
                self.cursor_evt_last.0,
                self.cursor_evt_last.1,
                self.last_cycle_us,
                self.last_update_us,
                self.last_render_us
            );
            self.last_cam_log = Instant::now();
        }
    }
}
