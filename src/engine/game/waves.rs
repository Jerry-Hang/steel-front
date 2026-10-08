// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Game {
    /// 诊断用：把 npc[0] 放到指定位置并固定（弹道隔离实验，RV3D_DIAG_NPC_FRONT）
    pub(crate) fn diag_place_npc(&mut self, pos: [f32; 3]) {
        if !self.npcs.is_empty() {
            self.npcs[0].position = pos;
            self.npcs[0].speed = 0.0;
            self.npcs[0].state_machine = NpcStateMachine::new();
        }
    }
    /// 波次推进：全部 NPC hp≤0 移除（`npcs` 为空）才算清空；清空后 3 秒倒计时刷下一波。
    ///
    /// 跑远的存活 NPC 仍留在列表里，不算清空（必须击杀全部）。
    pub(crate) fn update_waves(&mut self, dt: f32, player: &glam::Vec3) {
        // 援军波：波开始后 reinforcement_at 秒补怪 1..=2 只；波已清空则不再补（清波条件不变）
        let profile = wave_profile(self.effective_wave(self.wave));
        if !self.reinforcement_done
            && profile.kind == WaveKind::Reinforced
            && !self.npcs.is_empty()
            && self.time - self.wave_started_at >= profile.reinforcement_at.unwrap_or(f32::MAX)
        {
            self.reinforcement_done = true;
            let slot_base = (profile.count as f32 * self.npc_scale).round().max(1.0) as u32;
            let divisor = slot_base.max(1);
            let effective = self.effective_wave(self.wave);
            // 援军同样收口到玩家可达域（与主波次同一条规则）
            let reach =
                crate::engine::ai::reachable_mask(&self.grid, world_to_grid(player.x, player.z));
            for k in 0..profile.reinforcement_count {
                self.spawn_npc_ring(
                    player,
                    slot_base + k,
                    divisor,
                    self.wave,
                    profile.speed,
                    profile.hp,
                    profile.attack_range,
                    role_for(slot_base + k, effective, profile.flank_chance),
                    &reach,
                );
            }
            log::info!(
                "wave: reinforcement +{} on wave {}",
                profile.reinforcement_count,
                self.wave
            );
        }
        if self.npcs.is_empty() {
            if self.wave_timer <= 0.0 {
                self.wave_timer = WAVE_INTERMISSION;
                self.score += WAVE_CLEAR_BONUS;
                log::info!(
                    "wave: wave {} cleared (+{}), next in {:.0}s",
                    self.wave,
                    WAVE_CLEAR_BONUS,
                    WAVE_INTERMISSION
                );
            } else {
                self.wave_timer -= dt;
                if self.wave_timer <= 0.0 {
                    // 每关 WAVES_PER_LEVEL 波清完 → 升关：重新生成地图并回到本关第 1 波；
                    // 难度按累计有效波次递进（effective_wave），跨关不回落
                    // survive 规则：总波数 = rule.waves，守住全部波 → 胜利（补给窗口后进入胜利态）
                    //
                    // 🔴 `rule.waves` **可以长于** `WAVES_PER_LEVEL`（`defense_line.toml` 是 5）：
                    // survive 下必须**整条走 rule.waves**，不许掉进「清满 3 波就升关」那条分支 ——
                    // 旧写法在 waves=5 的图上第 3 波清完就升关并把 wave 归 1，于是第 4/5 波永远
                    // 到不了、胜利条件永远不成立（2026-09-25 真机：`wave 3 cleared` 之后紧跟
                    // `wave 1 spawned … effective=4`；红测
                    // `survive_wave_count_above_waves_per_level_still_reaches_victory`）。
                    // 胜利那一拍**不许再生成下一波**：旧代码把 `spawn_wave` 放在整个 if/else
                    // 之后，于是「守住全部波次 → 胜利」的同一帧又把最后一波重新刷了出来
                    // （2026-09-25 真机 5 波通关抓到：`survive: 全部 5 波守住 → 胜利` 之后
                    // 紧跟 `wave: wave 5 spawned 14 enemies (kind=Boss …)`，NPC #60–#73 又刷一批）。
                    let mut spawn_next = true;
                    if self.is_survive_rule() {
                        if self.wave >= self.survive_total_waves() {
                            self.hud.victory_banner = Some("防区固守！全部波次守住".to_string());
                            self.game_state = GameState::Victory(crate::engine::ai::Team::Blue);
                            self.set_won_team(crate::engine::ai::Team::Blue);
                            log::info!(
                                "survive: 全部 {} 波守住 → 胜利",
                                self.survive_total_waves()
                            );
                            spawn_next = false;
                        } else {
                            self.wave += 1;
                            // survive：波间补给窗口（血量回复 + 弹药补满）
                            self.supply_survive_break();
                        }
                    } else if self.wave >= WAVES_PER_LEVEL {
                        let next_level = self.level + 1;
                        self.apply_level(next_level);
                        self.wave = 1;
                        log::info!(
                            "level: advanced to level {} (map regenerated)",
                            next_level
                        );
                    } else {
                        self.wave += 1;
                    }
                    if spawn_next {
                        self.spawn_wave(self.wave, player);
                    }
                }
            }
        }
    }
    /// 生成第 n 波敌人：数量/速度/血量随波次递进，环形出生在玩家周围。
    ///
    /// 出生前清掉残留存活 NPC，保证新旧波不共存；
    /// Boss 波最后一只为主怪（高血量/慢速/攻击距离略长，见 wave_profile）；援军波重置补怪计时。
    pub(crate) fn spawn_wave(&mut self, n: u32, player: &glam::Vec3) {
        // 同步波次号：update_waves/update_ai 都按 self.wave 取 profile，直接调用时也必须一致
        self.wave = n;
        if !self.npcs.is_empty() {
            log::info!(
                "wave: purged {} leftover npcs before wave {}",
                self.npcs.len(),
                n
            );
            self.npcs.clear();
        }
        // 难度按累计有效波次：跨关不回落（level 2 第 1 波 ≈ 原第 4 波强度）
        let effective = self.effective_wave(n);
        let profile = wave_profile(effective);
        // NPC 数量按 RV3D_NPC_SCALE 缩放（默认 1.0；测试/冒烟不设变量行为不变）
        let count = (profile.count as f32 * self.npc_scale).round().max(1.0) as usize;
        let speed = profile.speed;
        let hp = profile.hp;
        let attack_range = profile.attack_range;
        // 出生点收口用的**玩家可达域**：整个波次算一次（O(格数)），每只出生只做一次查表。
        let reach = crate::engine::ai::reachable_mask(&self.grid, world_to_grid(player.x, player.z));
        for i in 0..count {
            // Boss 波最后一只为主怪：替换常规小怪，max_hp 大 → 渲染侧体型/外观体现
            let (spd, hpx, rng) = match profile.boss {
                Some(b) if i + 1 == count => (b.speed, b.hp, b.attack_range),
                _ => (speed, hp, attack_range),
            };
            // Boss 主怪固定突击角色：高血量压阵直线推进（保证参团冲锋，不绕侧/站桩）
            let role = if profile.boss.is_some() && i + 1 == count {
                TacticalRole::Rusher
            } else {
                role_for(i as u32, effective, profile.flank_chance)
            };
            let id = self.spawn_npc_ring(player, i as u32, count as u32, n, spd, hpx, rng, role, &reach);
            if profile.boss.is_some() && i + 1 == count {
                log::info!(
                    "wave: boss #{} spawn (hp={:.0} speed={:.1} attack={:.0})",
                    id,
                    hpx,
                    spd,
                    rng
                );
            }
        }
        // 援军波计时基准：波开始时间 + 补怪标志重置
        self.wave_started_at = self.time;
        self.reinforcement_done = false;
        log::info!(
            "wave: wave {} spawned {} enemies (kind={:?} total={} speed={:.1} hp={:.0} effective={})",
            n,
            count,
            profile.kind,
            profile.total_count,
            speed,
            hp,
            effective
        );
    }
    /// 环形出生一只 NPC：按 `slot/divisor` 均分角度 + 波次相位，半径 40..80m 确定性抖动，
    /// 出生点避开障碍格（沿径向外推最多 8 步，每步 4m）。返回新 NPC id。
    pub(crate) fn spawn_npc_ring(
        &mut self,
        player: &glam::Vec3,
        slot: u32,
        divisor: u32,
        wave_n: u32,
        speed: f32,
        hp: f32,
        attack_range: f32,
        role: TacticalRole,
        reach: &[bool],
    ) -> usize {
        let tau = std::f32::consts::TAU;
        let angle = slot as f32 * (tau / divisor.max(1) as f32) + wave_n as f32 * 0.37;
        let radius = 40.0 + 40.0 * ((slot * 7 + wave_n * 3) % 5) as f32 / 4.0;
        let (x, z) = self.push_out_of_obstacle(
            (player.x + angle.cos() * radius).clamp(-250.0, 250.0),
            (player.z + angle.sin() * radius).clamp(-250.0, 250.0),
        );
        // 🔴 出生点收口到**玩家可达域**（`reach`）：只保证"可站立"是不够的 ——
        // 可站立的小口袋会让这只永远走不到玩家（#17 根因链第 3 条的出生侧），
        // 且它每 1/3 秒重规划一次、每秒几百次 A* 全部 `连通域穷尽`。
        let (x, z) = match self.nearest_in_component(reach, world_to_grid(x, z)) {
            Some(g) => grid_to_world(g),
            None => (x, z),
        };
        let id = self.next_npc_id as usize;
        self.next_npc_id += 1;
        let y = terrain_height_at(x, z);
        log::info!("wave: npc #{} spawn ({:.1}, {:.1}, {:.1})", id, x, y, z);
        self.npcs.push(Npc {
            id,
            position: [x, y, z],
            speed,
            attack_range,
            home: [x, z],
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
                reposition: None,            hp,
            max_hp: hp,
            role,
            tactic: Tactic::Advance,
            dodge_timer: 0.0,
            hit_cooldown: 0.0,
            last_hp: hp,
            team: Team::Red,
            facing: (player.z - z).atan2(player.x - x),
            fire_accum: 0.0,
            knockback: [0.0, 0.0],
            grenade_timer: 0.0,
        });
        id
    }
    /// 出生点避开障碍：从给定点出发做**确定性扩环搜索**，返回最近的可站立点。
    ///
    /// 🔴 2026-09-12 重写。原实现有三个缺陷，实测 **6/255（红5 蓝1）** 个单位出生后仍卡在
    /// 不可通行格上：
    /// ① 外推方向是**离世界原点**（`sx += sx/d*4`），与障碍无关 —— 位于原点西北/东南侧的
    ///    单位会被推得更远，完全可能推进另一栋楼；
    /// ② 只走 8 步，走完仍不可通行就**静默返回坏点**，既无兜底也无计数；
    /// ③ 因此这个故障从上线起就没有任何人看见过。
    ///
    /// 现在改为「环 r 上均匀取 8r 个采样、由近及远、方向顺序固定」⇒ 结果**确定**
    /// （同一输入必得同一输出，冒烟与截图对比才不会失效）；扫完仍失败则打 `warn` 并原样返回。
    /// 判据仍是导航网格 `is_passable`（它与单位能否移动是同一条口径）；
    /// 「建筑视觉体大于碰撞盒」是另一件事，见 AGENTS.md 未结案 4。
    pub(crate) fn push_out_of_obstacle(&self, x: f32, z: f32) -> (f32, f32) {
        const CELL: f32 = 4.0;
        const MAX_RING: i32 = 8;
        if !self.blocked_at(x, z) {
            return (x.clamp(-250.0, 250.0), z.clamp(-250.0, 250.0));
        }
        for r in 1..=MAX_RING {
            let rf = r as f32;
            let samples = 8 * r;
            for k in 0..samples {
                let a = std::f32::consts::TAU * (k as f32) / (samples as f32);
                let px = x + a.cos() * rf * CELL;
                let pz = z + a.sin() * rf * CELL;
                if !self.blocked_at(px, pz) {
                    return (px.clamp(-250.0, 250.0), pz.clamp(-250.0, 250.0));
                }
            }
        }
        log::warn!("spawn: ({x:.0}, {z:.0}) 扩环 {MAX_RING} 层仍找不到可站立点，原样返回");
        (x.clamp(-250.0, 250.0), z.clamp(-250.0, 250.0))
    }
    /// 该点是否不可站立（导航网格判据，与单位移动同一条口径）
    pub(crate) fn blocked_at(&self, x: f32, z: f32) -> bool {
        !self.grid.is_passable(world_to_grid(x, z))
    }
    /// 在给定连通域（`mask`，由 `ai::reachable_mask` / `ai::largest_component_mask` 给出）里
    /// 取离 `want` 最近的格；`want` 本身在域内就原样返回。
    ///
    /// 确定性：先比直线距离²，再比行主序序号（同一输入必得同一格）。
    /// 用途 = **出生点收口**：`push_out_of_obstacle` 只保证"可站立"，而可站立的**小口袋**
    /// （院子里 9 格、墙缝 2 格）会让单位永远走不到任何人（实测每秒几百次 A* 全部
    /// `连通域穷尽`）。域内没有格时返回 `None`（调用方保持原点位，不 panic）。
    pub(crate) fn nearest_in_component(&self, mask: &[bool], want: GridPos) -> Option<GridPos> {
        let w = self.grid.width();
        let h = self.grid.height();
        let inside = |x: i32, y: i32| {
            x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && mask[y as usize * w + x as usize]
        };
        if inside(want.x, want.y) {
            return Some(want);
        }
        let mut best: Option<(i64, usize, GridPos)> = None;
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                if !inside(x, y) {
                    continue;
                }
                let d = ((x - want.x) as i64).pow(2) + ((y - want.y) as i64).pow(2);
                let idx = y as usize * w + x as usize;
                if best.as_ref().map_or(true, |(bd, bi, _)| (d, idx) < (*bd, *bi)) {
                    best = Some((d, idx, GridPos::new(x, y)));
                }
            }
        }
        best.map(|(_, _, g)| g)
    }
    /// 该点是否可站立。调试机位用它挑一个**不被墙挡**的观察方向 ——
    /// 手算方向失败过 13 次，判据本来就该由程序用（见 docs/PROGRESS.md）。
    pub(crate) fn standable(&self, x: f32, z: f32) -> bool {
        !self.blocked_at(x, z)
    }
    /// 调试用：该点是否落在**静态刚体（建筑/障碍）**的 AABB 内。
    ///
    /// 用的是**与 `npc_occluded` 完全同一套** `segment_hits_aabb`（退化成一个 ±0.01m 的
    /// 极短线段），不另写一套包含判定 —— 本仓最贵的 bug 就是"同一个量两套来源"。
    ///
    /// 存在的理由：`standable` 走导航网格，建筑走刚体 AABB，**两者不是同一套几何**。
    /// 这个查询用来量化两者的差异（未结案 4 的根因），也是第 24 轮取景问题的判据。
    pub(crate) fn point_in_body(&self, x: f32, y: f32, z: f32) -> bool {
        self.world
            .bodies
            .iter()
            .any(|body| Self::segment_hits_aabb(x - 0.01, y, z, x + 0.01, y, z, &body.aabb()))
    }
    /// 压力模式开战：红蓝各 `stress_sides` 名 NPC 分两半场环形出生（半径 150m+，避障外推），
    /// 角色/速度/血量/攻击距离按第 1 波 profile 确定性分配。清掉旧 NPC（全量重开一轮）。
    pub(crate) fn spawn_stress_battle(&mut self, player: &glam::Vec3) {
        self.npcs.clear();
        // 新一轮：任务目标重置为本轮歼灭一队（补员/波次逻辑不变）；
        // 上一轮胜利横幅保留到下一轮（start_run/apply_level 时才清空）
        self.objective = MissionObjective::new(self.stress_sides as u32);
        let sides = self.stress_sides as u32;
        let profile = wave_profile(self.effective_wave(1));
        // 🔬 **互换对照开关**（2026-09-13 加，用于未结案第一条"红蓝阵营不对称"）
        //
        // 压力模式下**红方恒胜**（四次 190s 对撞，蓝方净耗 101/102/102/103，红方 34/52/56/76），
        // 而出生几何按构造是对称的。判据写在未结案清单里：
        //   **把两侧半场对调 —— 赢家跟着半场走 = 地图几何；跟着队伍走 = 单位/AI 行为。**
        // `RV3D_SWAP_SIDES=1` 只交换两个半场，不动队伍/人数/角色/任何其它参数，
        // 所以它是干净的**单变量**对照。默认关闭 ⇒ 生产行为逐字节不变。
        let swap_sides = std::env::var("RV3D_SWAP_SIDES").as_deref() == Ok("1");
        // 见下方 `facing` 的注释：出生时朝向"对面半场"而不是朝向玩家
        let face_enemy = std::env::var("RV3D_FACE_ENEMY").as_deref() == Ok("1");
        // 🔴 压力模式的出生点必须收口到**主连通域**（2026-09-25 真机实测）：
        // 出生环 150–198m 正好穿过城市街区，`push_out_of_obstacle` 只保证"可通行"，
        // 实测 4 只红方里 2 只落在 **9 格 / 2 格**的小口袋（探针
        // `tmp_stress_spawn_reachability`），于是每秒几百次 A* 全部 `连通域穷尽` ——
        // 红蓝两军各在自己的院子里隔空对射，那套"20 轮红蓝对撞"的 A/B 也受此影响。
        // 参考域取**全图最大连通域**（不能取玩家所在的中央安全区：那是另一个小域）。
        let reach = crate::engine::ai::largest_component_mask(&self.grid);
        for side in 0..2u32 {
            let team = if side == 0 { Team::Red } else { Team::Blue };
            let base_angle = if (side == 0) != swap_sides { 0.0 } else { std::f32::consts::PI };
            // 蓝方少生 1 名：玩家本身就是蓝方士兵（红 64 vs 蓝 63+玩家 = 64v64）
            let per_side = if side == 0 { sides } else { sides.saturating_sub(1) };
            for i in 0..per_side {
                // 半场 ±~63° 扇形铺开，半径 150m + 确定性抖动（超出障碍环带 58-130m）
                let spread = -1.1 + (i as f32 / sides.max(1) as f32) * 2.2;
                let angle = base_angle + spread;
                let radius = STRESS_SPAWN_RADIUS + 12.0 * ((i * 7 + side) % 5) as f32;
                let (x, z) = self.push_out_of_obstacle(
                    (player.x + angle.cos() * radius).clamp(-250.0, 250.0),
                    (player.z + angle.sin() * radius).clamp(-250.0, 250.0),
                );
                let (x, z) = match self.nearest_in_component(&reach, world_to_grid(x, z)) {
                    Some(g) => grid_to_world(g),
                    None => (x, z),
                };
                let id = self.next_npc_id as usize;
                self.next_npc_id += 1;
                let y = terrain_height_at(x, z);
                self.npcs.push(Npc {
                    id,
                    position: [x, y, z],
                    speed: profile.speed,
                    attack_range: profile.attack_range,
                    home: [x, z],
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
                reposition: None,                    hp: profile.hp,
                    max_hp: profile.hp,
                    role: role_for(i, 1, profile.flank_chance),
                    tactic: Tactic::Advance,
                    dodge_timer: 0.0,
                    hit_cooldown: 0.0,
                    last_hp: profile.hp,
                    team,
                    // 🔬 2026-09-13：**出生朝向**。
                    //
                    // 原写法是 `(player.z - z).atan2(player.x - x)` —— **所有 NPC 都朝向玩家**。
                    // 而玩家恒为 `Team::Blue`、站在原点，于是：
                    //   * 红方 NPC 朝向的是一个**敌人** ⇒ 方向正确；
                    //   * 蓝方 NPC 朝向的是一个**友军** ⇒ **开局就在看错方向**。
                    // 这正是"赢家跟着队伍走、不跟着半场走"（`RV3D_SWAP_SIDES` 对照实测）的
                    // 一个候选解释：若 `facing` 参与感知/推进，蓝方等于让了先手。
                    //
                    // `RV3D_FACE_ENEMY=1` 把朝向改成**指向对面半场**（真正的敌人方向），
                    // 作为单变量对照。默认关 ⇒ 生产行为不变。
                    facing: if face_enemy {
                        if base_angle == 0.0 { std::f32::consts::PI } else { 0.0 }
                    } else {
                        (player.z - z).atan2(player.x - x)
                    },
                    fire_accum: 0.0,
                    knockback: [0.0, 0.0],
                grenade_timer: 0.0,
                });
            }
        }
        // 临时埋点（RV3D_NPC_POS=1）：前 3 个 NPC 的实际坐标（摆调试机位用）+
        // **出生后仍卡在不可通行格上的红/蓝计数**。
        // 后者是 `push_out_of_obstacle` 的失败计数：它 8 步推不出去时会**静默返回原样的坏点**，
        // 而它的判据是导航网格 `is_passable` —— 与建筑视觉体/碰撞盒不是同一套几何。
        // 红方 base_angle=0(+X)、蓝方 π(−X)，城市非镜像对称 ⇒ 两侧落点可能系统性不同。
        // 取景验完即删（教训 20）。
        if std::env::var("RV3D_NPC_POS").as_deref() == Ok("1") {
            for n in self.npcs.iter().take(3) {
                log::info!(
                    "npcpos: #{} team={:?} ({:.1}, {:.1}, {:.1})",
                    n.id,
                    n.team,
                    n.position[0],
                    n.position[1],
                    n.position[2]
                );
            }
            let (mut bad_r, mut bad_b) = (0u32, 0u32);
            for n in &self.npcs {
                if !self
                    .grid
                    .is_passable(world_to_grid(n.position[0], n.position[2]))
                {
                    match n.team {
                        Team::Red => bad_r += 1,
                        Team::Blue => bad_b += 1,
                    }
                }
            }
            log::info!(
                "npcpos: 出生仍卡在不可通行格 红={} 蓝={} / 共 {}",
                bad_r,
                bad_b,
                self.npcs.len()
            );
        }
        let red_ids: Vec<usize> = self.npcs.iter().filter(|n| n.team == Team::Red).map(|n| n.id).collect();
        let blue_ids: Vec<usize> = self.npcs.iter().filter(|n| n.team == Team::Blue).map(|n| n.id).collect();
        self.command = Some((
            crate::engine::ai_command::Army::build(Team::Red, &red_ids),
            crate::engine::ai_command::Army::build(Team::Blue, &blue_ids),
        ));
        log::info!("command: 三三制编成 红营 {} 人 / 蓝营 {} 人", red_ids.len(), blue_ids.len());
        self.round_started_at = self.time;
        log::info!(
            "battle: 压力模式第 {} 轮开战（红 {} vs 蓝 {}+玩家，共 {} 名 NPC，并行 AI={}）",
            self.stress_round,
            sides,
            sides.saturating_sub(1),
            self.npcs.len(),
            self.ai_parallel
        );
    }
    /// 压力模式减员与补员：移除阵亡 NPC；任一阵营团灭 → 全量补员开新一轮。
    pub(crate) fn update_stress_respawns(&mut self, player: &glam::Vec3) {
        // 菜单态不结算（初始 NPC 全为红方，会误判蓝方团灭提前开战）
        if self.game_state == GameState::StartMenu {
            return;
        }
        let before = self.npcs.len();
        // 本轮自损累计（阵亡者按阵营计数；供指挥军情的 kills 字段 = **该营自身阵亡数**）
        // enemy_dead = 敌军（Red）本帧阵亡数，单独累计供任务目标推进
        // ⚠️ 这里只扫「还留在数组里的 hp<=0」（兜底路径）；`damage_npc` 打死的人当场就被移出数组，
        // 所以那边必须自己计数 —— 见 `tally_round_deaths` 的注释与判据。
        let mut red_dead = 0u32;
        let mut blue_dead = 0u32;
        for n in &self.npcs {
            if n.hp <= 0.0 {
                match n.team {
                    Team::Red => red_dead += 1,
                    Team::Blue => blue_dead += 1,
                }
            }
        }
        let enemy_dead = red_dead;
        self.tally_round_deaths(Team::Red, red_dead);
        self.tally_round_deaths(Team::Blue, blue_dead);
        self.npcs.retain(|n| n.hp > 0.0);
        let red = self.npcs.iter().filter(|n| n.team == Team::Red).count();
        let blue = self.npcs.len() - red;
        if self.npcs.len() != before {
            // 任务目标：本轮歼灭数推进（达成 → 胜利横幅/日志；补员逻辑不受影响）
            // 🔴 只计敌军阵亡。旧写法用 (before - self.npcs.len()) = **双方合计**阵亡，
            // 压力模式下 target=128 是**单方**兵力、全场 255 人，于是双方合计死到 128
            // （全场才死一半）就刷"本轮敌军全灭"横幅 —— 横幅是假的。普通模式全部 NPC
            // 都属 Red，两种口径等价，所以这个改动只修正压力模式。
            if self.objective.progress(enemy_dead) {
                self.on_objective_complete();
            }
            log::info!(
                "battle: 阵亡 {}（红={} 蓝={} 存活）",
                before - self.npcs.len(),
                red,
                blue
            );
        }
        // 回合超时上限（2026-08-23：城市地图下残局易僵持刷步，300s 按存活多者判胜）
        if red > 0 && blue > 0 && self.round_started_at >= 0.0 && self.time - self.round_started_at > 300.0 {
            let winner = if red >= blue { Some(Team::Red) } else { Some(Team::Blue) };
            let who = if winner == Some(Team::Red) { "红方" } else { "蓝方" };
            self.round_reset_at = self.time + 10.0;
            self.round_winner = winner;
            self.hud.victory_banner = Some(format!("{} 获胜（回合超时 存活 {}:{}）— 10 秒后下一轮", who, red, blue));
            log::info!(
                "battle: 第 {} 轮超时（红={} 蓝={}）→ {} 获胜，10 秒后重置",
                self.stress_round, red, blue, who
            );
        }
        // 兵力先耗尽方判负（2026-08-23：玩家无敌观战，纯 AI-vs-AI 博弈）
        if (red == 0 || blue == 0) && self.round_reset_at < 0.0 {
            let winner = if red == 0 && blue == 0 {
                None
            } else if red == 0 {
                Some(Team::Blue)
            } else {
                Some(Team::Red)
            };
            self.round_winner = winner;
            self.round_reset_at = self.time + 10.0;
            let who = match winner {
                Some(Team::Red) => "红方",
                Some(Team::Blue) => "蓝方",
                None => "平局",
            };
            self.hud.victory_banner = Some(format!("{} 获胜（兵力耗尽）— 10 秒后下一轮", who));
            log::info!(
                "battle: 第 {} 轮结束（红={} 蓝={}）→ {} 获胜，10 秒后重置",
                self.stress_round,
                red,
                blue,
                who
            );
        }
        // 10 秒后重置：清场重开本轮
        if self.round_reset_at >= 0.0 && self.time >= self.round_reset_at {
            self.round_reset_at = -1.0;
            self.round_winner = None;
            self.stress_round += 1;
            log::info!("battle: 第 {} 轮重开（阵亡清场）", self.stress_round);
            self.spawn_stress_battle(player);
        }
    }
}
