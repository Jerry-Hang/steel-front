// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Game {
    /// 创建游戏中枢：初始化物理演示场景
    pub(crate) fn new() -> Self {
        let mut world = physics::World::new();
        world.gravity = 9.8;
        // 地形中央 60×60 区域已压平到 y=0（renderer.rs flatten_mask）；障碍刚体由关卡布局生成
        world.ground_y = 0.0;
        let event_buf = Arc::new(Mutex::new(Vec::new()));
        world.add_listener(Box::new(EventBuffer(event_buf.clone())));
        // NPC 出生点：集中在中心 ±40m（相机默认在原点附近，可触发 Chase）
        let spawns = [
            (-30.0, -20.0),
            (-20.0, 15.0),
            (-10.0, -35.0),
            (0.0, 25.0),
            (15.0, -15.0),
            (25.0, 20.0),
            (35.0, -30.0),
            (40.0, 10.0),
        ];
        let mut npcs = Vec::with_capacity(NPC_COUNT);
        for (id, (x, z)) in spawns.iter().enumerate().take(NPC_COUNT) {
            npcs.push(Npc {
                id,
                position: [*x, terrain_height_at(*x, *z), *z],
                speed: 4.0,
                attack_range: 12.0,
                home: [*x, *z],
                state_machine: NpcStateMachine::new(),
                perception: NpcPerception::default(),
                path: Vec::new(),
                path_index: 0,
                direct_goal: false,
                direct_x: 0.0,
                direct_z: 0.0,
                last_goal: [0.0, 0.0],
                aidiag_bucket: u64::MAX,
                attack_timer: 0.0,
                reposition: None,                hp: 100.0,
                max_hp: 100.0,
                role: TacticalRole::Rusher,
                tactic: Tactic::Advance,
                dodge_timer: 0.0,
                hit_cooldown: 0.0,
                last_hp: 100.0,
                team: Team::Red,
                facing: 0.0,
                fire_accum: 0.0,
                knockback: [0.0, 0.0],
                grenade_timer: 0.0,
            });
        }
        let mut game = Self {
            world,
            collisions: Vec::new(),
            total_collisions: 0,
            time: 0.0,
            last_dt: 0.0,
            event_buf,
            last_event_log_time: 0.0,
            wave_started_at: 0.0,
            reinforcement_done: false,
            weapons: WeaponRack::new(
                ALL_WEAPONS
                    .iter()
                    .map(|spec| (spec.name_zh.to_string(), build_firearm(spec)))
                    .collect(),
                0.6,
            ),
            grenades: 2,
            grenades_max: 2,
            medkits: 2,
            medkits_max: 2,
            heal_timer: 0.0,
            grenades_vec: Vec::new(),
            pending_kick: (0.0, 0.0),
            // 碰撞半径 0.35（真人肩宽 ~0.7m）：0.5 太胖，视觉"离障碍还有距离就卡住"
        player_body: PlayerBody::new(Pv::new(0.0, 0.0, 0.0), 0.35, 1.6),
            move_forward: false,
            jump_pressed: false,
            stance: Stance::Standing,
            sprint_held: false,
            jump_vel: 0.0,
            jump_hvel: glam::Vec3::ZERO,
            move_backward: false,
            move_left: false,
            move_right: false,
            footstep_timer: 0.0,
            sfx: SfxBank::new(48_000),
            projectiles: Vec::new(),
            explosions: Vec::new(),
            last_blast_center: [0.0; 3],
            shake_timer: 0.0,
            shake_strength: 0.0,
            fire_cooldown: 0.0,
            spread_scale: 1.0,
            cull_eye_override: None,
            occl_cache: Vec::new(),
            occl_cache_age: 0,
            occl_us: std::cell::Cell::new(0),
            occl_calls: std::cell::Cell::new(0),
            npc_vis: Vec::new(),
            npc_vis_frame: 0,
            npc_vis_scans: std::cell::Cell::new(0),
            fire_mode: FireMode::Auto,
            auto_heat: 0.0,
            npc_hit_flash: std::collections::HashMap::new(),
            hit_points: Vec::new(),
            impact_marks: Vec::new(),
            hit_damages: Vec::new(),
            shots: 0,
            hits: 0,
            grid: GridMap::new(GRID_SIZE, GRID_SIZE),
            npcs,
            ai_log_time: 0.0,
            ai_diag_prev: Vec::new(),
            ai_diag_prev_t: 0.0,
            charge_active: false,
            npc_scale: {
                let v = std::env::var("RV3D_NPC_SCALE")
                    .ok()
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                v.max(0.5)
            },
            // 大战场判定：变量存在且值非 "0"/"off" → 大战场；无变量/0/off → 波次模式
            // （main.rs 未设置时默认注入 "64"；测试环境无变量 → wave 模式，勿反转）
            stress: !matches!(
                std::env::var("RV3D_STRESS_AI").as_deref(),
                Err(_) | Ok("0") | Ok("off")
            ),
            // 2026-08-22：默认 128v128（用户要求：以海量 NPC 逼近真人联机压力）
            stress_sides: std::env::var("RV3D_STRESS_AI")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|&n| n >= 4)
                .unwrap_or(128),
            stress_round: 0,
            command: None,
            llm: crate::llm_cmd::LlmCommander::from_env(),
            player_invincible: {
                let llm_on = std::env::var("RV3D_LLM")
                    .map(|v| !(v.is_empty() || v == "0" || v == "off"))
                    .unwrap_or(false);
                let inv = std::env::var("RV3D_INVINCIBLE")
                    .map(|v| v == "1" || v == "on" || v == "true")
                    .unwrap_or(false);
                inv || llm_on
            },
            round_reset_at: -1.0,
            round_winner: None,
            round_started_at: -1.0,
            round_kills_red: 0,
            round_kills_blue: 0,
            command_log_at: 0.0,
            ai_parallel: std::env::var("RV3D_AI_PARALLEL")
                .map(|v| v != "off")
                .unwrap_or(true),
            stage_physics_us: 0,
            stage_ai_us: 0,
            stage_audio_us: 0,
            stage_net_us: 0,
            stage_proj_us: 0,
            stage_ai_only_us: 0,
            stage_wave_us: 0,
            stage_obj_us: 0,
            acc_proj_us: 0,
            acc_ai_only_us: 0,
            acc_wave_us: 0,
            acc_obj_us: 0,
            explosion_sim: std::env::var("RV3D_EXPLOSION_SIM")
                .is_ok_and(|v| v == "1" || v == "true"),
            shock_points: Vec::new(),
            bench_points: Vec::new(),
            shock_out: Vec::new(),
            stage_explosion_us: 0,
            last_explosion_log: 0.0,
            explosion_path: "scalar",
            last_damage_time: 0.0,
            hud: HudState::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
            frames: 0,
            frame_no: 0,
            fps_window_start: Instant::now(),
            audio_sample_rate: 48_000,
            level: 1,
            map_generation: 0,
            map: LevelMap::default(),
            audio: {
                let mut player =
                    AudioPlayer::new(crate::audio_out::open_default_sink(48_000, 2));
                player.mixer_mut().set_master(0.9);
                player.mixer_mut().set_channel_volume(Channel::Sfx, 1.0);
                // 环境风：DSP 慢速调制噪声（真实声卡输出）
                player
                    .synth_mut()
                    .set_ambient(glam::Vec3::new(0.0, 2.0, 0.0), 0.35);
                player
            },
            net_demo: {
                let enabled = std::env::var("RV3D_NET")
                    .map(|v| v == "1" || v == "demo")
                    .unwrap_or(false);
                if !enabled {
                    None
                } else {
                    match Self::init_network_demo() {
                        Ok(demo) => {
                            log::info!("net: loopback demo enabled (in-process server+client)");
                            Some(demo)
                        }
                        Err(e) => {
                            log::warn!("net: 环回演示初始化失败，已禁用: {}", e);
                            None
                        }
                    }
                }
            },
            net_server: None,
            net_client: None,
            net_players: Vec::new(),
            net_input_seq: 0,
            net_snap_seq: 0,
            net_fire_pending: false,
            net_look: None,
            last_net_log: 0.0,
            game_state: GameState::StartMenu,
            wave: 1,
            wave_timer: 0.0,
            score: 0,
            next_npc_id: NPC_COUNT as u32,
            last_status_log: 0.0,
            last_npcpos_log: 0.0,
            objective: MissionObjective::new(0),
            map_mgr: None,
            map_path: None,
            level_list: Vec::new(),
            level_idx: 0,
            obj_state: None,
        };
        // 关卡系统初始化：RV3D_MAP=<单关 toml> 或 RV3D_MAPS=<index.toml 关卡列表>
        // 启用时替换程序化地图；未设置 → None（程序化地图，默认行为零回归）
        game.init_map_system();
        // 初始关卡布局（level 1，种子 = 1）：物理刚体 + AI 网格 + 玩家安全区复位
        game.apply_level(1);
        game
    }
    /// 当前游戏状态（供 main.rs 控制光标捕获等输入行为）
    pub(crate) fn state(&self) -> GameState {
        self.game_state
    }
    /// 初始化关卡系统（Game::new 调用一次）：
    /// - `RV3D_MAP=<assets/maps/xxx.toml>`：加载单张地图（关卡列表为空）
    /// - `RV3D_MAPS=<assets/maps/index.toml>`：加载关卡列表，进入第一关
    /// 两者皆未设置 → 保持 None（程序化地图 + StartMenu，默认行为与测试基线零回归）。
    /// 加载失败仅告警并回退程序化地图（不 panic、不中断启动）。
    pub(crate) fn init_map_system(&mut self) {
        let single = std::env::var("RV3D_MAP").ok().filter(|p| !p.is_empty());
        let index = std::env::var("RV3D_MAPS").ok().filter(|p| !p.is_empty());
        let (path, list) = if let Some(p) = single {
            (p, Vec::new())
        } else if let Some(idx) = index {
            match crate::engine::map::load_map_list(&idx) {
                Ok(list) if !list.is_empty() => (list[0].clone(), list),
                Ok(_) => {
                    log::warn!("map: 关卡列表 {} 为空，回退程序化地图", idx);
                    return;
                }
                Err(e) => {
                    log::warn!("map: 关卡列表加载失败 {}: {}，回退程序化地图", idx, e);
                    return;
                }
            }
        } else {
            return;
        };
        match crate::engine::map::MapManager::load(&path) {
            Ok(mgr) => {
                use crate::engine::objective::GameRule;
                let rule = match mgr.data().rule.kind.as_str() {
                    "kill" => GameRule::KillCount {
                        target: mgr.data().rule.target,
                    },
                    "time" => GameRule::TimeLimit {
                        seconds: mgr.data().rule.seconds,
                    },
                    "survive" => GameRule::Survive {
                        waves: mgr.data().rule.waves.max(1),
                    },
                    _ => GameRule::CapturePoints {
                        required: mgr.data().rule.required.max(1),
                    },
                };
                let mut pts = Vec::new();
                for o in mgr
                    .data()
                    .objectives
                    .iter()
                    .filter(|o| o.kind.eq_ignore_ascii_case("capture"))
                {
                    pts.push(crate::engine::objective::CapturePoint::new(
                        o.id.clone(),
                        o.x,
                        o.z,
                        o.radius,
                        o.capture_time,
                    ));
                }
                let mut obj_state = crate::engine::objective::ObjectiveState::new(rule);
                obj_state.points = pts;
                log::info!(
                    "map: 关卡系统启用 {}（{} 出生点 / {} 障碍 / {} 目标，规则 {}）",
                    mgr.data().name,
                    mgr.data().spawn_points.len(),
                    mgr.data().obstacles.len(),
                    mgr.data().objectives.len(),
                    obj_state.rule.rule_kind()
                );
                self.map_mgr = Some(mgr);
                self.map_path = Some(path);
                self.level_list = list;
                self.level_idx = 0;
                // 据点立柱碰撞体（旗杆 0.4m 宽 × 4m 高）：玩家不可穿过据点标记
                // （修复"穿越障碍物"——据点此前只渲染 marker、无任何碰撞体）
                if let Some(obj) = self.obj_state.as_ref() {
                    for pt in &obj.points {
                        self.world.bodies.push(Body::new_static(
                            Pv::new(pt.x, 2.0, pt.z),
                            Pv::new(0.2, 2.0, 0.2),
                        ));
                    }
                }
                self.obj_state = Some(obj_state);
                // 加载完成 → LoadingMap 态（按任意键开玩）
                self.game_state = GameState::LoadingMap;
            }
            Err(e) => {
                log::warn!("map: 关卡加载失败 {}: {}，回退程序化地图", path, e);
            }
        }
    }
    /// 开始菜单任意键：进入游戏（开始/重开一局）
    pub(crate) fn on_any_key(&mut self, player: &glam::Vec3) {
        if self.game_state == GameState::StartMenu || self.game_state == GameState::LoadingMap {
            self.start_run(player);
        }
    }
    /// 死亡/失败结算界面 R：重开一局（重开本关）
    pub(crate) fn request_restart(&mut self, player: &glam::Vec3) {
        match self.game_state {
            GameState::GameOver | GameState::Victory(_) | GameState::Defeat => {
                self.start_run(player);
            }
            _ => {}
        }
    }
    /// 开始一局：复位血量/弹药/分数/波次/关卡，重建第 1 关地图，清掉残留 NPC 后生成第 1 波
    pub(crate) fn start_run(&mut self, player: &glam::Vec3) {
        self.hud.health = self.hud.max_health;
        self.hud.ammo = self.hud.max_ammo;
        self.hud.reserve = self.weapons.active_firearm_ref().reserve();
        self.hud.settings_open = false;
        self.hud.confirm_quit = false;
        self.hud.victory_banner = None;
        // 🔴 2026-09-22 复查补：击杀提示是**每局**的事件流。不清它，上一局的
        // "你被击杀了" / 击杀行会跟到新一局（最多 6s，`KILL_FEED_DURATION`），
        // 而重开时 score 已归零 —— 读数自相矛盾。`victory_banner` 同理（上一行）。
        self.hud.kill_feed.clear();
        self.hud.cancel_rebind();
        self.score = 0;
        self.wave = 1;
        self.wave_timer = 0.0;
        self.fire_cooldown = 0.0;
        self.weapons.reset_all_ammo();
        self.fire_mode = FireMode::Auto;
        self.auto_heat = 0.0;
        self.npc_hit_flash.clear();
        self.hit_points.clear();
        self.hit_damages.clear();
        self.impact_marks.clear();
        self.pending_kick = (0.0, 0.0);
        // 重开一局 = 从第 1 关全新地图开始（同时把玩家拉回原点安全区）
        self.apply_level(1);
        // 关卡系统：玩家出生点用地图 Blue 出生点（未启用时保持原点）
        if let Some(mgr) = self.map_mgr.as_ref() {
            if let Some((sx, sy, sz)) = mgr.spawn_point("blue") {
                self.player_body.pos = Pv::new(sx, sy, sz);
            } else {
                self.player_body.pos = Pv::new(0.0, 0.0, 0.0);
            }
        } else {
            self.player_body.pos = Pv::new(0.0, 0.0, 0.0);
        }
        self.player_body.vel = Pv::ZERO;
        // 🔴 2026-09-22 复查补：跳跃状态必须一起复位。玩家可以在**空中**被打死
        // （NPC 伤害每秒结算一次，不看你在不在空中），此时 `jump_vel`（上升速度）与
        // `jump_hvel`（冲刺跳的水平惯性）会带进新一局 —— 重开瞬间凭空弹起/继续滑行；
        // `jump_pressed`（一直按着空格不松）则会"落地即起跳"。
        // ⚠️ 必须在这里清：`move_first_person` 里只要 `jump_vel != 0` 就跳过落地分支，
        // 残留的上升速度不会自己消失（`jump_hvel` 才会）。
        self.jump_vel = 0.0;
        self.jump_hvel = glam::Vec3::ZERO;
        self.jump_pressed = false;
        self.move_forward = false;
        self.move_backward = false;
        self.move_left = false;
        self.move_right = false;
        self.footstep_timer = 0.0;
        self.projectiles.clear();
        // 🔴 2026-09-22 复查补：重开/换图必须连**在飞的手榴弹与爆炸**一起清。
        // 否则"投掷后死亡（或通关），再按 R 重开"时，上一局的手榴弹会留在场上并进入新一局 ——
        // 它在新的第 1 波里爆炸，会多算击杀得分，还可能伤到玩家。
        // （`grenades_vec` 只在爆炸后由 `retain` 清，不会自己过期。）
        self.grenades_vec.clear();
        self.explosions.clear();
        self.shake_timer = 0.0;
        self.shots = 0;
        self.hits = 0;
        self.total_collisions = 0;
        self.last_damage_time = 0.0;
        self.stress_round = 0;
        if !self.npcs.is_empty() {
            log::info!("game: purged {} leftover npcs on run start", self.npcs.len());
            self.npcs.clear();
        }
        if self.stress {
            self.spawn_stress_battle(player);
        } else {
            self.spawn_wave(1, player);
        }
        // 关卡系统：重开本关 → 据点进度/归属/推进方归零（rule 保持当前地图的规则）
        if let Some(obj) = self.obj_state.as_mut() {
            for pt in obj.points.iter_mut() {
                pt.reset(); // 归属 + 进度 + 推进方一起清（单一定义在 objective.rs）
            }
            obj.kills = 0;
            obj.elapsed = 0.0;
            obj.won_team = None;
        }
        self.game_state = GameState::Playing;
        log::info!("game: run started (wave 1)");
    }
    /// 累计有效波次：跨关不回落（level 2 第 1 波 ≈ 原第 4 波强度）
    pub(crate) fn effective_wave(&self, wave: u32) -> u32 {
        wave + (self.level.saturating_sub(1)) * WAVES_PER_LEVEL
    }
    /// 应用关卡布局：重建物理障碍刚体 + AI 导航网格（确定性，种子 = 关卡号）。
    ///
    /// 由 new() / start_run() / 升关时调用；同时把玩家拉回原点安全区，防止卡进障碍。
    pub(crate) fn apply_level(&mut self, level: u32) {
        self.level = level;
        self.map_generation += 1;
        // 地图来源：关卡系统启用（RV3D_MAP/RV3D_MAPS）→ TOML 障碍；否则程序化生成（默认行为）。
        // 关卡系统地图已在 init_map_system / advance_level / reload_current_map 加载到 map_mgr。
        if let Some(mgr) = self.map_mgr.as_ref() {
            let mut obstacles = Vec::new();
            for def in mgr.obstacles() {
                let (kind, x, z, half_w, half_d) =
                    crate::engine::map::obstacle_to_map_obstacle(def);
                let max_hp = obstacle_max_hp(kind);
                // TOML 障碍贴地：y = half_h = 1.2（def.y 仅作提示位，实际贴地渲染）
                obstacles.push(MapObstacle {
                    x,
                    z,
                    half_w,
                    half_d,
                    y: 1.2,
                    half_h: 1.2,
                    kind,
                    tint: None,
                    max_hp,
                    hp: max_hp,
                    shape: Shape::Legacy,
                });
            }
            self.map = LevelMap { obstacles, decor: Vec::new(), props: Vec::new() };
        } else {
            // 地图：默认手绘现代城市（不再随机种子生成；RV3D_PROC_MAP=1 回退程序化生成用于 A/B）
            // 生成换核执行（线程优化第 3 步）：走 ai_pool（AMD CCD1 / Intel E-core / 小核），
            // 生成计算不吃主线程所在簇；join 语义保证返回时地图已就绪。
            if std::env::var("RV3D_PROC_MAP").as_deref() == Ok("1") {
                self.map =
                    crate::engine::cpu::ai_pool().run_sync(move || generate_level_map(level));
            } else {
                self.map = crate::engine::city::generate_city();
            }
        }
        // 任务目标：普通波次 = 本关 WAVES_PER_LEVEL 波出场总数（含援军）；
        // 压力模式 = 歼灭一队即本轮胜利（spawn_stress_battle 每轮重置）
        self.objective = MissionObjective::new(if self.stress {
            self.stress_sides as u32
        } else {
            self.level_objective_target(level)
        });
        self.hud.victory_banner = None;
        // 物理世界：清掉上一关障碍，重建为当前关卡障碍盒（贴地 AABB，供玩家碰撞/投射物拦截）。
        // 刚体 AABB = (x, 1.2, z) ± (half_w, 1.2, half_d) 与渲染 marker 严格同尺寸
        // （renderer.rs WorldMarker::for_obstacle：平移 (x,1.2,z) × 缩放 2·half_w/2.4/2·half_d），
        // 水平足迹逐米一致 —— 玩家被挡距离只由玩家胶囊半径（0.5m）决定（见 physics.rs）。
        self.world.bodies.clear();
        self.world.spheres.clear();
        for ob in &self.map.obstacles {
            // 2026-08-23：地图障碍 = 静态刚体（不沉地/不被撞动）
            // 注意：这里**不能**按条件跳过元素。`hit_obstacle_index` 返回的是
            // world.bodies 下标，而 `damage_obstacle` 拿它当 map.obstacles 下标用
            // （两表按下标严格一一对应并在摧毁时同步 remove）。装饰件走
            // LevelMap::decor 独立表，不要在这里加 filter —— 那会让伤害结算打到
            // 错误的障碍上。
            self.world.bodies.push(Body::new_static(
                Pv::new(ob.x, ob.y, ob.z),
                Pv::new(ob.half_w, ob.half_h, ob.half_d),
            ));
        }
        // AI 网格：**唯一**建网规则 = `block_obstacle_cells`（NPC 寻路绕行 / 掩体点判定共用）
        let mut grid = GridMap::new(GRID_SIZE, GRID_SIZE);
        let mut blocked_cells = 0usize;
        for ob in &self.map.obstacles {
            blocked_cells += block_obstacle_cells(&mut grid, ob);
        }
        self.grid = grid;
        // 升关/重开时把玩家拉回原点安全区（中央环带无阻碍，见 MAP_RING_INNER 注释）
        self.player_body.pos = Pv::new(0.0, 0.0, 0.0);
        self.player_body.vel = Pv::ZERO;
        // 障碍种类分布统计（供日志/调试；渲染 marker 颜色由 main.rs 按 kind 映射）
        let mut kind_counts = [0u32; 6];
        for ob in &self.map.obstacles {
            kind_counts[match ob.kind {
                ObstacleKind::Wall => 0,
                ObstacleKind::Block => 1,
                ObstacleKind::Barrier => 2,
                ObstacleKind::Tree => 3,
                ObstacleKind::Building => 4,
                ObstacleKind::Ruin => 5,
            }] += 1;
        }
        log::info!(
            "map: level {} generated: {} obstacle bodies (wall={} block={} barrier={} tree={} building={} ruin={}), {} grid cells blocked",
            level,
            self.map.obstacles.len(),
            kind_counts[0],
            kind_counts[1],
            kind_counts[2],
            kind_counts[3],
            kind_counts[4],
            kind_counts[5],
            blocked_cells
        );
    }
    /// 本关任务目标：本关出场敌人总数（含援军；count 按
    /// RV3D_NPC_SCALE 缩放后四舍五入，与 spawn_wave 的出生数量逐波一致）。
    ///
    /// 🔴 **survive 规则的波数 = `rule.waves`，可以长于 `WAVES_PER_LEVEL`**（`defense_line`
    /// 是 5）⇒ 目标数必须同源，否则第 3 波刚打完就报「本关敌军全灭」。2026-09-25 真机抓到：
    /// wave 3 清完那一刻 `objective: 普通模式本关 歼灭全部敌人达成（26 击杀）→ victory`，
    /// 而后面还有第 4、5 波（实际通关是 52 杀）。
    pub(crate) fn level_objective_target(&self, level: u32) -> u32 {
        let mut total = 0u32;
        let waves = if self.is_survive_rule() {
            self.survive_total_waves()
        } else {
            WAVES_PER_LEVEL
        };
        for w in 1..=waves {
            let effective = w + (level.saturating_sub(1)) * WAVES_PER_LEVEL;
            let profile = wave_profile(effective);
            let count = (profile.count as f32 * self.npc_scale).round().max(1.0) as u32;
            total += count + profile.reinforcement_count;
        }
        total.max(1)
    }
    /// 任务目标达成：一次性胜利日志 + HUD 横幅（游戏继续，不阻断波次生成/补员逻辑）
    pub(crate) fn on_objective_complete(&mut self) {
        self.hud.victory_banner = Some(if self.stress {
            "VICTORY — 本轮敌军全灭".to_string()
        } else {
            "VICTORY — 本关敌军全灭".to_string()
        });
        log::info!(
            "objective: {} 歼灭全部敌人达成（{} 击杀）→ victory",
            if self.stress {
                "压力模式本轮"
            } else {
                "普通模式本关"
            },
            self.objective.eliminated
        );
    }
    /// 地图代号。渲染层据此判断道具几何是否需要重传（见 `Renderer::set_props`）。
    pub(crate) fn map_generation(&self) -> u64 {
        self.map_generation
    }
    /// 本关的 GLB 道具摆放列表（见 [`LevelMap::props`]）。
    pub(crate) fn prop_placements(&self) -> &[crate::engine::props::PropPlacement] {
        &self.map.props
    }
    /// 渲染与路径追踪应绘制的全部几何 = 障碍 + 装饰件（见 [`LevelMap::decor`]）。
    /// 顺序恒为 obstacles 在前、decor 在后，所以 marker 下标与刚体下标在前 N 个一致。
    pub(crate) fn render_geometry(&self) -> impl Iterator<Item = &MapObstacle> {
        self.map.render_geometry()
    }
    /// 每帧推进所有已接入系统
    pub(crate) fn update(&mut self, dt: f32, camera: &Camera) {
        self.frame_no = self.frame_no.wrapping_add(1);
        self.last_dt = dt;
        self.time += dt;
        self.fire_cooldown = (self.fire_cooldown - dt).max(0.0);
        // 武器架切换计时/换弹计时 + HUD 武器/弹药/换弹状态同步
        self.weapons.update(dt);
        // 连发热量衰减（停火后枪口上扬恢复）
        self.auto_heat = (self.auto_heat - dt * 0.9).max(0.0);
        // NPC 受击闪白衰减
        self.npc_hit_flash.retain(|_, t| {
            *t -= dt;
            *t > 0.0
        });
        self.hud.ammo = self.weapons.active_firearm_ref().magazine();
        self.hud.max_ammo = self.weapons.active_firearm_ref().max_magazine();
        self.hud.reserve = self.weapons.active_firearm_ref().reserve();
        self.hud.reloading = self.weapons.active_firearm_ref().is_reloading();
        self.hud.reload_progress = self.weapons.active_firearm_ref().reload_progress();
        self.hud.weapon_name = self.weapons.active_name().to_string();
        self.hud.switching = self.weapons.is_switching();
        self.hud.grenades = self.grenades;
        self.hud.crosshair_spread = self.crosshair_spread();
        self.hud.medkits = self.medkits;
        self.hud.heal_progress = self.heal_progress();
        // 打药推进（计时归零那一帧一次性回血）
        self.update_heal(dt);
        // 手榴弹推进（抛物线 + 引信）
        self.update_grenades(dt);
        // 关卡号同步（由关卡推进 / 重开写入，供 HUD 显示）
        self.hud.level = self.level;
        // 命中标记衰减 + 音量同步
        self.hud.tick(dt);
        self.audio.mixer_mut().set_master(self.hud.volume);
        // 音乐通道独立音量（设置面板 MUSIC 项，0..=1）
        self.audio
            .mixer_mut()
            .set_channel_volume(Channel::Music, self.hud.music_volume);
        // 程序化环境音乐：战斗状态开大、菜单/结算调小（1.5s 淡入淡出由 audio 内部插值）
        let music_target = match self.game_state {
            GameState::Playing => 1.0,
            _ => 0.3,
        };
        self.audio.set_music_target(music_target);
        // 第一人称玩家移动（WASD + 碰撞）
        if self.game_state == GameState::Playing && camera.mode == CameraMode::FirstPerson {
            self.move_first_person(camera, dt);
        }
        // fps 统计（1 秒窗口）
        self.frames += 1;
        let window_secs = self.fps_window_start.elapsed().as_secs_f32();
        if window_secs >= 1.0 {
            self.hud.fps = self.frames as f32 / window_secs;
            self.frames = 0;
            self.fps_window_start = Instant::now();
        }
        let t0 = std::time::Instant::now();
        self.world.step(dt);
        self.drain_collisions();
        self.stage_physics_us = t0.elapsed().as_micros() as u64;
        let t0 = std::time::Instant::now();
        match self.game_state {
            GameState::StartMenu => {
                // 菜单吸引模式：世界照常运行（NPC 游走/追击），不结算伤害与波次
                let t = std::time::Instant::now();
                self.update_projectiles(dt, true);
                self.stage_proj_us = t.elapsed().as_micros() as u64;
                let t = std::time::Instant::now();
                self.update_ai(dt, camera);
                self.stage_ai_only_us = t.elapsed().as_micros() as u64;
                self.stage_wave_us = 0;
                self.stage_obj_us = 0;
            }
            GameState::LoadingMap => {
                // 关卡加载为同步操作（init_map_system 已载入），此态仅作状态机过渡
            }
            GameState::Playing => {
                let t = std::time::Instant::now();
                self.update_projectiles(dt, true);
                self.stage_proj_us = t.elapsed().as_micros() as u64;
                let t = std::time::Instant::now();
                self.update_ai(dt, camera);
                self.stage_ai_only_us = t.elapsed().as_micros() as u64;
                let t = std::time::Instant::now();
                self.update_waves(dt, &camera.position());
                self.stage_wave_us = t.elapsed().as_micros() as u64;
                // 关卡系统：每帧推进据点占领 + 胜负判定（未启用时无操作）
                let t = std::time::Instant::now();
                self.update_objectives(dt, &camera.position());
                self.stage_obj_us = t.elapsed().as_micros() as u64;
            }
            GameState::GameOver
            | GameState::Victory(_)
            | GameState::Defeat => {
                // 冻结玩法：AI/伤害/波次停止；投射物继续飞行但不再判定命中/击杀
                self.update_projectiles(dt, false);
            }
        }
        self.stage_ai_us = t0.elapsed().as_micros() as u64;
        // 爆炸实体生命周期 + 震屏衰减（生成在 update_projectiles 内，AoE 已即时结算）
        self.step_explosions(dt);
        // 冲击波/爆炸 SIMD 实测（默认关；RV3D_EXPLOSION_SIM=1 时每帧推进压力场并输出加速比）
        if self.explosion_sim {
            self.step_explosion_sim();
        }
        // 状态日志（1 秒一条，冒烟断言 game: wave= 序列用）
        if self.time - self.last_status_log >= 1.0 {
            self.last_status_log = self.time;
            // 玩法分项计时累计（见 `stage_proj_us` 段注释）：`ai_us` 那一格是整段，不是 AI。
            self.acc_proj_us += self.stage_proj_us as u64;
            self.acc_ai_only_us += self.stage_ai_only_us as u64;
            self.acc_wave_us += self.stage_wave_us as u64;
            self.acc_obj_us += self.stage_obj_us as u64;
            // 寻路诊断（**独立一行**，只在 RV3D_AI_DIAG=1 时打）：
            // 未结案 #25 的验收要"数 find_path 返回 None 的比例"，而上面那行状态日志的字段顺序
            // 是被冒烟/survive harness 解析的，**不能往里塞字段** ⇒ 另起一行。
            if ai_diag() {
                let calls = crate::engine::ai::astar_calls_take();
                let fails = crate::engine::ai::astar_fails_take();
                let (f_start, f_goal, f_ex) = crate::engine::ai::astar_fail_reasons_take();
                let partial = crate::engine::ai::astar_partial_take();
                let unstuck = NOTE_NPC_UNSTUCK.swap(0, std::sync::atomic::Ordering::Relaxed);
                let (expanded, expanded_max, pushed) = crate::engine::ai::astar_work_take();
                let ord = std::sync::atomic::Ordering::Relaxed;
                let mv_step = NOTE_MOVE_STEP.swap(0, ord);
                let mv_undone = NOTE_MOVE_UNDONE.swap(0, ord);
                let sep_pushed = NOTE_SEP_PUSHED.swap(0, ord);
                let sep_big = NOTE_SEP_BIG.swap(0, ord);
                // 移动归因（#17 定位工具）：Chase 群上一秒的**真实速度** = Δ位移/Δt。
                // 判据：`state` 与路点进度都正常、速度却 ≈0 ⇒ 移动被抵消，再看上面四个计数
                // 是"障碍推回"还是"邻居分离力"。
                let dt_win = (self.time - self.ai_diag_prev_t).max(1e-3);
                let pp = self.player_pos();
                let mut chase = 0u32;
                let mut measured = 0u32;
                let mut stalled = 0u32;
                let mut sum_v = 0.0f32;
                // (离玩家距离, id, 速度, 目标, 路点, 路点总数, 速度设定, 战术)
                let mut far: Vec<(f32, u32, f32, [f32; 2], usize, usize, f32, Tactic)> = Vec::new();
                for npc in &self.npcs {
                    if npc.state_machine.state() != NpcState::Chase {
                        continue;
                    }
                    chase += 1;
                    let dx = npc.position[0] - pp.x;
                    let dz = npc.position[2] - pp.z;
                    let d_player = (dx * dx + dz * dz).sqrt();
                    let Some((_, px, pz)) = self
                        .ai_diag_prev
                        .iter()
                        .find(|(id, _, _)| *id == npc.id as u32)
                    else {
                        far.push((
                            d_player,
                            npc.id as u32,
                            -1.0,
                            npc.last_goal,
                            npc.path_index,
                            npc.path.len(),
                            npc.speed,
                            npc.tactic,
                        ));
                        continue;
                    };
                    let v = ((npc.position[0] - px).powi(2) + (npc.position[2] - pz).powi(2)).sqrt()
                        / dt_win;
                    measured += 1;
                    sum_v += v;
                    if v < 0.5 {
                        stalled += 1;
                    }
                    far.push((
                        d_player,
                        npc.id as u32,
                        v,
                        npc.last_goal,
                        npc.path_index,
                        npc.path.len(),
                        npc.speed,
                        npc.tactic,
                    ));
                }
                // 列**离玩家最远的 3 只**：波次清不掉就是它们（有路径、有速度、却永远不靠近）。
                far.sort_by(|a, b| b.0.total_cmp(&a.0));
                let mut farthest = String::new();
                for (d, id, v, g, idx, len, sp, tac) in far.iter().take(3) {
                    farthest.push_str(&format!(
                        " #{} d={:.1} v={:.2}/{:.1} tac={:?} goal=({:.1},{:.1}) wp={}/{}",
                        id, d, v, sp, tac, g[0], g[1], idx, len
                    ));
                }
                let avg = if measured > 0 {
                    format!("{:.2}", sum_v / measured as f32)
                } else {
                    "--".to_string()
                };
                // 🔴 2026-09-26 加：**战术分布**按秒打一行。
                // 为什么：AGENTS 未结案 #18 挂着"CoverSeek 占比偏低（压力模式 4%，另一次 0）"，
                // 但那个数字**没有一把留在仓库里的尺子** —— `aidiag: move` 只打"最远 3 只"的战术。
                // 想判断"是掩体不够，还是触发条件太窄"，先把整场的分布量出来（先量再改）。
                // `Tactic` 是 8 个取值，直接数一遍（NPC 数百，1 Hz，可忽略）。
                {
                    use crate::engine::ai::Tactic;
                    let mut t = [0u32; 8];
                    for n in self.npcs.iter() {
                        let i = match n.tactic {
                            Tactic::Advance => 0,
                            Tactic::Flank => 1,
                            Tactic::Ambush => 2,
                            Tactic::Suppress => 3,
                            Tactic::CoverAdvance => 4,
                            Tactic::Retreat => 5,
                            Tactic::Hold => 6,
                            Tactic::CoverSeek => 7,
                        };
                        t[i] += 1;
                    }
                    let total: u32 = t.iter().sum();
                    let pct = |v: u32| {
                        if total == 0 {
                            0.0
                        } else {
                            v as f32 * 100.0 / total as f32
                        }
                    };
                    log::info!(
                        "aidiag: tactic 1s Advance={}({:.0}%) Flank={}({:.0}%) Ambush={}({:.0}%) Suppress={}({:.0}%) CoverAdvance={}({:.0}%) Retreat={}({:.0}%) Hold={}({:.0}%) CoverSeek={}({:.0}%) 共{}",
                        t[0], pct(t[0]), t[1], pct(t[1]), t[2], pct(t[2]), t[3], pct(t[3]),
                        t[4], pct(t[4]), t[5], pct(t[5]), t[6], pct(t[6]), t[7], pct(t[7]), total
                    );
                }
                log::info!(
                    "aidiag: move 1s Chase={} 实测均速={} m/s 停滞(<0.5m/s)={}；想走={} 被障碍抵消={}；分离推={} 推>半步={}；最远{}",
                    chase,
                    avg,
                    stalled,
                    mv_step,
                    mv_undone,
                    sep_pushed,
                    sep_big,
                    farthest
                );
                self.ai_diag_prev.clear();
                self.ai_diag_prev.extend(
                    self.npcs
                        .iter()
                        .map(|n| (n.id as u32, n.position[0], n.position[2])),
                );
                self.ai_diag_prev_t = self.time;
                log::info!(
                    "aidiag: astar 1s 内 calls={} fails={} partial={}（起点阻挡={} 目标阻挡={} 连通域穷尽={}）；NPC 站在阻挡格里被挪回={}；展开={}（单次最大={}）入队={}",
                    calls,
                    fails,
                    partial,
                    f_start,
                    f_goal,
                    f_ex,
                    unstuck,
                    expanded,
                    expanded_max,
                    pushed
                );
                // 玩法分项（未结案 #25 的"AI 花在哪"）：`game:` 行的 `ai_us` 是**整段**
                // （投掷物 + AI + 波次 + 据点）⇒ 这里给每秒累计值，四段互不相交。
                log::info!(
                    "aidiag: stage 1s proj={}us ai={}us wave={}us obj={}us（合计={}us，占 ai_us 的一格）",
                    self.acc_proj_us,
                    self.acc_ai_only_us,
                    self.acc_wave_us,
                    self.acc_obj_us,
                    self.acc_proj_us + self.acc_ai_only_us + self.acc_wave_us + self.acc_obj_us
                );
            }
            // 累计值**无条件**清零（不依赖诊断开关，否则关掉 diag 时会一直累加）
            self.acc_proj_us = 0;
            self.acc_ai_only_us = 0;
            self.acc_wave_us = 0;
            self.acc_obj_us = 0;
            let enemy_hp = self.npcs.first().map(|n| n.max_hp).unwrap_or(0.0);
            // 玩家位置入状态行：survive harness 走位支持需要它算相对方位角
            let pp = self.player_pos();
            // `hits=` = 玩家弹丸**命中 NPC** 的累计次数（打墙/打友军不计，见结算循环）。
            // 🔴 2026-09-25 加：harness 想知道"打中率"此前只能拿 `kills/shots` 反推，
            // 而那里面混着"打了掩体"和"残局空点" ⇒ 无法判"改瞄法到底有没有用"。
            log::info!(
                "game: wave={} enemies={} enemy_hp={:.0} hp={:.0}/{:.0} score={} pos=({:.1},{:.1}) phys_us={} ai_us={} audio_us={} net_us={} hits={}",
                self.wave,
                self.npcs.len(),
                enemy_hp,
                self.hud.health,
                self.hud.max_health,
                self.score,
                pp.x,
                pp.z,
                self.stage_physics_us,
                self.stage_ai_us,
                self.stage_audio_us,
                self.stage_net_us,
                self.hits()
            );
        }
        // 机器可读的**活靶**位置（`RV3D_NPC_POS=1`）：默认每秒每只 NPC 一行，
        // `RV3D_NPC_POS_HZ` 可提到最高 30 Hz（harness 打移动靶的命中率取决于**样本年龄**：
        // 1 Hz 意味着它可能瞄一个 1 秒前的位置，而 NPC 是 4–5 m/s，见 `npc_pos_period`）。
        //
        // 🔴 2026-09-25 加：注入 harness 此前只能从 `npc: #N stand (x,y,z)` 取目标位置，
        // 而那行是**进入 Attack 那一刻**的快照 —— 移动靶/反复进出 Attack 的目标全程打空
        // （实测 12 发/杀、残局 8 分钟零命中）。这一行让 harness 打"当前位置"。
        // 与 `RV3D_AI_DIAG` 分开：harness 要的是位置，不需要 AI 归因那一堆统计。
        if npc_pos_log() && self.time - self.last_npcpos_log >= npc_pos_period(npc_pos_hz()) {
            self.last_npcpos_log = self.time;
            // 玩家位置同频发一行：harness 的角度是**以玩家位置为基准**算的，而它走路 6 m/s
            // ⇒ 1 Hz 的 `game:` 行会让"收敛好的准星"指向错的目标点
            // （弹道埋点：过期弹 117/118 差最近 NPC 有 2m 以上，见 PROGRESS §21.21）。
            let pp = self.player_pos();
            log::info!("playerpos: {:.2} {:.2}", pp.x, pp.z);
            for (i, n) in self.npcs.iter().enumerate() {
                // `vis=0/1`：玩家眼位能不能看到它（= 这一枪的射线有没有被障碍挡）。
                // 🔴 2026-09-25 加：埋点量出**33% 的子弹打在掩体上**（§21.18），
                // 而 harness 拿不到遮挡信息、只能乱选目标 ⇒ 把判据直接发给它
                // （`npc_occluded` 是既有真源，别再写第二套）。
                log::info!(
                    "npcpos: #{} {:.2} {:.2} {:.2} {:?} vis={}",
                    n.id,
                    n.position[0],
                    n.position[1],
                    n.position[2],
                    n.state_machine.state(),
                    if self.npc_occluded(i) { 0 } else { 1 }
                );
            }
        }
        // 音频：每帧按 dt 渲染样本（SilentSink 丢弃输出，混音/衰减链路真实运行）
        let t0 = std::time::Instant::now();
        let frames = ((self.audio_sample_rate as f32) * dt) as usize;
        self.audio
            .tick(&AudioListener::new(camera.position()), frames.min(8192));
        self.stage_audio_us = t0.elapsed().as_micros() as u64;
        let t0 = std::time::Instant::now();
        self.update_net(camera);
        self.step_net(camera);
        self.stage_net_us = t0.elapsed().as_micros() as u64;
    }
    /// 关卡系统每帧推进（仅 Playing 态调用；未启用时直接返回）：
    /// 1. 据点占领：玩家（Blue 阵营）站在据点内且无 Red NPC 在场 → 进度增长；
    ///    有敌对 NPC 在场 → 压制衰减；玩家撤离 → 缓慢消散。
    /// 2. 胜负判定：规则达成 → 切换 GameState::Victory(team) / Defeat（幂等，只触发一次）。
    /// 3. 限时统计同步给 ObjectiveState。
    pub(crate) fn update_objectives(&mut self, dt: f32, player: &glam::Vec3) {
        let Some(obj) = self.obj_state.as_mut() else { return };
        let player_team = crate::engine::ai::Team::Blue; // 玩家恒为 Blue 阵营
        for pt in obj.points.iter_mut() {
            let inside = pt.is_inside(player.x, player.z);
            let has_enemy = self
                .npcs
                .iter()
                .any(|n| n.team != player_team && pt.is_inside(n.position[0], n.position[2]));
            let players_inside: Vec<crate::engine::ai::Team> =
                if inside { vec![player_team] } else { Vec::new() };
            crate::engine::objective::update_point(pt, dt, &players_inside, has_enemy);
        }
        obj.elapsed += dt as f64;
        match obj.evaluate() {
            crate::engine::objective::WinState::Victory(team) => {
                obj.won_team = Some(team);
                self.game_state = GameState::Victory(team);
                log::info!("objective: 关卡胜利，获胜方 {:?}", team);
            }
            crate::engine::objective::WinState::Defeat => {
                obj.won_team = Some(player_team.opposite());
                self.game_state = GameState::Defeat;
                log::info!("objective: 关卡失败（时间到/据点尽失）");
            }
            crate::engine::objective::WinState::None => {}
        }
    }
    /// 关卡系统击杀计数：普通波次击杀（damage_npc）调用，KillCount 规则用
    pub(crate) fn objective_register_kill(&mut self) {
        if let Some(obj) = self.obj_state.as_mut() {
            obj.kills = obj.kills.saturating_add(1);
        }
    }
    /// 是否启用 survive（防守波次）规则
    pub(crate) fn is_survive_rule(&self) -> bool {
        matches!(
            self.obj_state.as_ref().map(|o| o.rule),
            Some(crate::engine::objective::GameRule::Survive { .. })
        )
    }
    /// survive 总波数（默认 0 = 非 survive）
    pub(crate) fn survive_total_waves(&self) -> u32 {
        match self.obj_state.as_ref().map(|o| o.rule) {
            Some(crate::engine::objective::GameRule::Survive { waves }) => waves,
            _ => 0,
        }
    }
    /// survive 波间补给窗口：血量回复 50% + 当前武器弹匣补满 + 手榴弹补满
    pub(crate) fn supply_survive_break(&mut self) {
        self.hud.health = (self.hud.health + self.hud.max_health * 0.5).min(self.hud.max_health);
        self.weapons.active_firearm().reset();
        self.grenades = self.grenades_max;
        self.medkits = self.medkits_max;
        self.heal_timer = 0.0;
        log::info!(
            "survive: 波间补给（血量 {:.0}% + 弹药补满 + 手榴弹 {}）",
            self.hud.health / self.hud.max_health * 100.0,
            self.grenades
        );
    }
    /// 置位关卡胜负归属（幂等：已判定则不覆盖）
    pub(crate) fn set_won_team(&mut self, team: crate::engine::ai::Team) {
        if let Some(obj) = self.obj_state.as_mut() {
            if obj.won_team.is_none() {
                obj.won_team = Some(team);
            }
        }
    }
    /// 关卡系统据点数据（供 main.rs 渲染世界标记）：(id, x, z, 归属, 进度 0..=1)。
    /// 未启用关卡系统或无据点 → 空列表。
    ///
    /// 🔴 返回值里**必须带 `radius`**：占领底盘是**视觉**，它必须等于**玩法**的占领判定半径。
    /// 早先这里只给 `(id,x,z,owner,progress)`，渲染侧拿不到 radius，于是 `main.rs` 把底盘
    /// 写成了硬编码 `from_scale(10.0, …)` —— 而立方体模板是 ±1、`from_scale` 传的是
    /// **半尺寸**，结果底盘画成半径 10m，对 `street_fight`(5.0) / `bridgehead`(5.0/6.0)
    /// 是**真实占领圈的两倍**，对 `defense_line`(12.0) 又**反而小一圈**。
    /// 玩家据此判断"我进圈了没有"，是玩法级的误导，不只是好看问题。
    pub(crate) fn capture_points(
        &self,
    ) -> Vec<(String, f32, f32, f32, Option<crate::engine::ai::Team>, f32)> {
        self.obj_state
            .as_ref()
            .map(|o| {
                o.points
                    .iter()
                    .map(|p| (p.id.clone(), p.x, p.z, p.radius, p.owner, p.progress))
                    .collect()
            })
            .unwrap_or_default()
    }
    /// 关卡系统热重载（F5）：重新读取当前地图 TOML 并重建物理/AI 网格/据点
    pub(crate) fn reload_current_map(&mut self) -> Result<(), String> {
        let Some(path) = self.map_path.clone() else {
            return Ok(()); // 未启用关卡系统：无事可做
        };
        {
            let Some(mgr) = self.map_mgr.as_mut() else {
                return Ok(());
            };
            mgr.reload(&path)?;
        }
        // 重建物理/AI 网格（复用 apply_level 的障碍→刚体/网格逻辑）
        self.apply_level(self.level);
        // 重建据点（进度归零，规则沿用新地图的 rule）
        let rule = {
            let d = self.map_mgr.as_ref().map(|m| &m.data().rule);
            match d.map(|r| r.kind.as_str()) {
                Some("kill") => crate::engine::objective::GameRule::KillCount {
                    target: d.map(|r| r.target).unwrap_or(0),
                },
                Some("time") => crate::engine::objective::GameRule::TimeLimit {
                    seconds: d.map(|r| r.seconds).unwrap_or(0.0),
                },
                Some("survive") => crate::engine::objective::GameRule::Survive {
                    waves: d.map(|r| r.waves.max(1)).unwrap_or(1),
                },
                _ => crate::engine::objective::GameRule::CapturePoints {
                    required: d.map(|r| r.required.max(1)).unwrap_or(1),
                },
            }
        };
        let mut obj = crate::engine::objective::ObjectiveState::new(rule);
        obj.points = self
            .map_mgr
            .as_ref()
            .map(|m| {
                m.data()
                    .objectives
                    .iter()
                    .filter(|o| o.kind.eq_ignore_ascii_case("capture"))
                    .map(|o| {
                        crate::engine::objective::CapturePoint::new(
                            o.id.clone(),
                            o.x,
                            o.z,
                            o.radius,
                            o.capture_time,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.obj_state = Some(obj);
        log::info!("map: 热重载完成（{}）", path);
        Ok(())
    }
    /// 关卡系统下一关（胜利结算 N 键）：进入列表下一张地图；已到最后一关 → 返回 false（通关）
    ///
    /// 2026-09-08：删掉夹在这里的 `player_speed()`——它是一次误插入（把本注释和函数体
    /// 隔开了），且无人调用：持枪摆动取的是玩家脚底实际位移 / dt，见 `main.rs` 那条说明。
    pub(crate) fn advance_level(&mut self, player: &glam::Vec3) -> bool {
        if self.level_list.is_empty() {
            return false; // 单关模式（RV3D_MAP）：无下一关
        }
        if self.level_idx + 1 >= self.level_list.len() {
            return false; // 最后一关通关
        }
        self.level_idx += 1;
        let path = self.level_list[self.level_idx].clone();
        match crate::engine::map::MapManager::load(&path) {
            Ok(mgr) => {
                self.map_mgr = Some(mgr);
                self.map_path = Some(path);
                self.apply_level(self.level);
                self.start_run(player);
                log::info!(
                    "map: 进入下一关 {}（第 {} 关）",
                    self.map_mgr
                        .as_ref()
                        .map(|m| m.data().name.clone())
                        .unwrap_or_default(),
                    self.level_idx + 1
                );
                true
            }
            Err(e) => {
                log::warn!("map: 下一关加载失败 {}: {}", path, e);
                false
            }
        }
    }
    /// 构建 HUD quad 列表：血条/弹药/FPS + 调试行（LOD、实体数、NPC 状态、碰撞/命中）
    pub(crate) fn hud_quads(&mut self, near: u32, far: u32, lod: &str) -> Vec<crate::ui::Quad> {
        use crate::ui::{render_text, Color, HudScreen};
        // 每帧同步 HUD 显示字段
        self.hud.score = self.score;
        self.hud.wave = self.wave;
        self.hud.countdown = self.wave_timer.max(0.0);
        self.hud.survive_waves = self.survive_total_waves();
        self.hud.objective = (self.objective.eliminated, self.objective.target);
        // 关卡系统：据点状态同步到 HUD（id/归属/进度）。
        // 单机 = 本机 obj_state；联机客户端 = 网络 ObjectiveState 广播（归属码 0=中立/1=Red/2=Blue）。
        // 无联机且无关卡系统 → 空列表（HUD 不显示进度条，行为零回归）。
        // 小地图快照：单位（含阵营）/障碍（含种类与尺寸）/玩家位置（朝向由 main.rs 同步）
        self.hud.mm_units = self
            .npcs
            .iter()
            .map(|n| {
                (
                    n.position[0],
                    n.position[2],
                    if n.team == crate::engine::ai::Team::Red { 0 } else { 1 },
                )
            })
            .collect();
        self.hud.mm_obstacles = self
            .map
            .obstacles
            .iter()
            .filter(|o| o.hp > 0.0)
            .map(|o| {
                let kind = match o.kind {
                    ObstacleKind::Wall => 0u8,
                    ObstacleKind::Block => 1,
                    ObstacleKind::Barrier => 2,
                    ObstacleKind::Tree => 3,
                    ObstacleKind::Building => 4,
                    ObstacleKind::Ruin => 5,
                };
                (o.x, o.z, o.half_w, o.half_d, kind)
            })
            .collect();
        {
            let eye = self.player_eye();
            self.hud.mm_player = [eye.x, eye.z];
        }
        self.hud.capture_points = if let Some(o) = self.obj_state.as_ref() {
            o.points
                .iter()
                .map(|p| (p.id.clone(), p.owner, p.progress))
                .collect()
        } else if let Some(client) = self.net_client.as_ref() {
            client
                .objective_state()
                .iter()
                .map(|(id, code, progress)| {
                    let owner = match code {
                        1 => Some(crate::engine::ai::Team::Red),
                        2 => Some(crate::engine::ai::Team::Blue),
                        _ => None,
                    };
                    (id.clone(), owner, *progress)
                })
                .collect()
        } else {
            Vec::new()
        };
        self.hud.screen = if self.hud.settings_open {
            HudScreen::Settings
        } else {
            match self.game_state {
                GameState::StartMenu => HudScreen::Start,
                GameState::LoadingMap => HudScreen::Start,
                GameState::GameOver | GameState::Defeat => HudScreen::GameOver,
                GameState::Victory(_) => HudScreen::Game,
                GameState::Playing => HudScreen::Game,
            }
        };
        let mut quads = self.hud.layout();
        if self.game_state != GameState::Playing {
            return quads;
        }
        let mut counts = [0u32; 4];
        for npc in &self.npcs {
            counts[npc.state_machine.state() as usize] += 1;
        }
        let line1 = format!(
            "LOD: {}  entities: {}/65536  npc: I{} P{} C{} A{}",
            lod,
            near + far,
            counts[0],
            counts[1],
            counts[2],
            counts[3]
        );
        let hud_s = self.hud.ui_scale();
        render_text(&line1, 10.0 * hud_s, 44.0 * hud_s, Color::YELLOW, 1.3 * hud_s, &mut quads);
        // 当前武器规格：口径 + 阵营配色（联合体=红 / 同盟=蓝）
        let active_spec = ALL_WEAPONS.get(self.weapons.active_index());
        let (wcolor, caliber_txt) = match active_spec {
            Some(s) if s.faction == crate::engine::weapon_data::Faction::Union => (
                Color::new(0.95, 0.45, 0.35, 1.0),
                s.caliber,
            ),
            Some(s) => (Color::new(0.35, 0.65, 0.98, 1.0), s.caliber),
            None => (Color::CYAN, ""),
        };
        let line2 = format!(
            "{} ({}) [{}]  ammo: {:.0}%  hits: {}  col: {}",
            self.weapons.active_name(),
            caliber_txt,
            self.fire_mode.label(),
            self.weapons.active_firearm_ref().ammo_ratio() * 100.0,
            self.hits(),
            self.total_collisions()
        );
        render_text(&line2, 10.0 * hud_s, 62.0 * hud_s, wcolor, 1.3 * hud_s, &mut quads);
        quads
    }
    /// 构建默认光照场景（方向光 + 环境光 + 2 点光；阴影默认开，`RV3D_NO_SHADOW=1` 关）
    ///
    /// 强度配平（2026-09-01 建模重构第 1 批）：旧值 sun=1.5 / ambient=0.5×(0.5,0.55,0.6)
    /// 使任何 NdotL>0.5 的面全部撞上 `min(radiance,1)` 截顶，明暗比只有 4:1 且高光端全糊。
    /// 片元改用不截顶的指数压缩后，批次 1 先降到 sun=1.15 / 环境 0.30，把
    /// [背光 0.20, 迎光 0.87] 整个区间放回曲线未饱和段；同日质量 pass（c5a2a67）
    /// 因阴影面街道读不清再把两者上调到**当前的 sun=1.35 / 环境 0.55**。
    /// 改这两个数必须同时看 build.rs::apply_lighting 的曲线。
    pub(crate) fn light_uniform(&self) -> super::lighting::LightUniform {
        use super::lighting::{DirectionalLight, LightUniform, PointLight, ShadowConfig};
        let sun = DirectionalLight::new(
            glam::Vec3::new(-0.4, 0.9, -0.3).normalize(),
            glam::Vec3::new(1.0, 0.95, 0.85),
            1.35,
        );
        let point_a = PointLight::new(
            glam::Vec3::new(0.0, 6.0, 0.0),
            glam::Vec3::new(0.9, 0.6, 0.4),
            1.5,
        );
        let point_b = PointLight::new(
            glam::Vec3::new(-24.0, 5.0, -16.0),
            glam::Vec3::new(0.4, 0.7, 1.0),
            1.0,
        );
        // 阴影贴图（2026-08-11）：正交光空间以地图中心为 target、半宽 250m，
        // 覆盖障碍环带（58-130m）与两军接火区；相机无现成引用，取原点近似。
        // ShadowConfig.light_dir 语义 = 表面→光源方向，与 sun.direction 一致直接传入
        // （旧实现传 -sun.direction 使光相机在地面下方仰视：阴影图只剩背面剔除后的
        //   竖面，地面/地形整片缺失，阴影完全失效）。
        // RV3D_NO_SHADOW=1 关闭阴影（仅环境光+点光源），用于 A/B 验证与阴影 pass 性能对比。
        // 2026-08-22：城市高楼群下阴影图覆盖异常（全图判黑），默认改用环境光+方向光直照
        // （RV3D_NO_SHADOW=0 强制回阴影贴图路径，用于后续排查）
        // 2026-08-28：用户机器全高实测——默认开阴影（RV3D_NO_SHADOW=1 显式关闭）；
        // 阴影覆盖范围加大以适配城市高楼群（扩展 250→400 米，远平面 500→800）
        let shadow = if std::env::var("RV3D_NO_SHADOW").as_deref() == Ok("1") {
            None
        } else {
            Some(ShadowConfig::new(sun.direction, glam::Vec3::ZERO, 400.0, 1.0, 800.0))
        };
        LightUniform::build(
            Some(&sun),
            &[point_a, point_b],
            glam::Vec3::new(0.5, 0.55, 0.6),
            0.55,
            shadow.as_ref(),
        )
    }
}
