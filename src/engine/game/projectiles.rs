// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Game {
        /// 当前飞行/落地手榴弹位置列表（渲染用：世界内可见手雷实体）
    pub(crate) fn grenade_positions(&self) -> Vec<[f32; 3]> {
        self.grenades_vec.iter().map(|g| g.position()).collect()
    }
    /// 投掷手榴弹（G 键）：库存 >0 且切枪/换弹中时投掷。方向 = 相机方向 + 上仰角
    /// （水平方向 + 0.25rad 上抛，保证抛物线落地）；引信 1.5-2.5s 确定性伪随机。
    pub(crate) fn throw_grenade(&mut self, origin: [f32; 3], direction: [f32; 3]) -> bool {
        if self.grenades == 0 || self.weapons.is_switching() {
            return false;
        }
        self.grenades -= 1;
        // 投掷哨声（高音下滑，与枪声区分）
        self.audio
            .synth_mut()
            .play_grenade_throw(glam::Vec3::new(origin[0], origin[1], origin[2]));
        // 上抛：水平方向（归一化）+ 固定上仰分量；Grenade::new 会归一化 dir 再乘 speed
        let mut dir = glam::Vec3::new(direction[0], direction[1], direction[2]);
        if dir.length_squared() < 1e-6 {
            dir = glam::Vec3::Z;
        }
        dir.y = 0.0;
        if dir.length_squared() < 1e-6 {
            dir = glam::Vec3::Z;
        }
        let dir = dir.normalize();
        let vx = dir.x;
        let vz = dir.z;
        let vy = 0.35; // 上抛分量（竖直向上 ≈ 35% 总初速）
        // 引信：确定性伪随机（随投掷次数变化），落在 1.5-2.5s
        let fuse = GRENADE_FUSE_MIN
            + (self.shots.wrapping_mul(13) % 1000) as f32 / 1000.0
                * (GRENADE_FUSE_MAX - GRENADE_FUSE_MIN);
        self.grenades_vec.push(Grenade::new(
            origin,
            [vx, vy, vz],
            GRENADE_SPEED,
            fuse,
        ));
        log::info!(
            "grenade: thrown #{} fuse={:.2}s remaining={}",
            self.grenades_vec.len(),
            fuse,
            self.grenades
        );
        true
    }
    /// 手榴弹每帧推进：抛物线 + 引信；到期 → 爆炸（复用 spawn_explosion：AoE 伤害 +
    /// 径向击退 + 震屏 + 自发光闪光）。落地（y ≤ 地形高度）也触发爆炸。
    pub(crate) fn update_grenades(&mut self, dt: f32) {
        let mut explosions: Vec<[f32; 3]> = Vec::new();
        for g in &mut self.grenades_vec {
            g.update(dt);
            // 落地或引信到期 → 爆炸
            let ground = terrain_height_at(g.position()[0], g.position()[2]);
            if g.exploded() || g.position()[1] <= ground + 0.05 {
                // 落地滚动音（短促低音 thud；爆炸音由 spawn_explosion 的 SfxKind::Explosion 承担）
                self.audio
                    .synth_mut()
                    .play_grenade_bounce(glam::Vec3::new(g.position()[0], g.position()[1], g.position()[2]));
                log::info!(
                    "grenade: detonate fuse_max={:.2}s",
                    g.fuse_max()
                );
                explosions.push(g.position());
            }
        }
        if !explosions.is_empty() {
            self.grenades_vec.retain(|g| !g.exploded() && g.position()[1] > terrain_height_at(g.position()[0], g.position()[2]) + 0.05);
            for center in explosions {
                // 手榴弹 AoE：半径 8m、伤害 120（近距可秒标准 NPC）、径向击退
                self.spawn_explosion(center, GRENADE_EXPLOSION_RADIUS, GRENADE_EXPLOSION_DAMAGE, true);
            }
        }
    }
    /// NPC 投掷手榴弹（压力模式 / survive 防守波次）：交火中（Attack 态）的 NPC 按
    /// 确定性概率（5-8%，随 id/帧哈希）向敌对目标投掷；冷却 10-18s 确定性伪随机。
    /// - 普通波次（打玩家）不调用 → AI 行为零回归；
    /// - 压力模式目标 = pick_stress_targets 的敌对 NPC（阵营区分，目标与投掷者异阵营）；
    /// - survive 目标 = 玩家（防守方 NPC 进攻）。
    pub(crate) fn npc_throw_grenades(&mut self, dt: f32, targets: &[Option<(usize, [f32; 3], f32)>]) {
        if dt <= 0.0 {
            return;
        }
        // 先递减冷却
        for npc in &mut self.npcs {
            npc.grenade_timer = (npc.grenade_timer - dt).max(0.0);
        }
        let mut to_throw: Vec<(usize, [f32; 3])> = Vec::new(); // (npc_idx, 目标位置)
        for (i, npc) in self.npcs.iter().enumerate() {
            if npc.grenade_timer > 0.0 || npc.state_machine.state() != NpcState::Attack {
                continue;
            }
            // 确定性概率：id/帧哈希 → 5-8%（投掷窗口内约每 12-20 帧判定一次）
            // 班长（指挥体系）投掷倾向更高（15%），其余战士维持 8%
            let is_leader = self.command.as_ref().map(|cmd| match npc.team {
                Team::Red => cmd.0.is_leader(npc.id),
                Team::Blue => cmd.1.is_leader(npc.id),
            }).unwrap_or(false);
            let h = (npc.id as u64 * 31 + self.frame_no as u64 * 7) % 100;
            if h >= if is_leader { 15 } else { 8 } {
                continue;
            }
            // 目标：压力模式 = 敌对 NPC（异阵营）；survive = 玩家
            let target_pos = if self.stress {
                targets
                    .get(i)
                    .and_then(|t| t.as_ref())
                    .filter(|(_, _, _)| {
                        // 只投掷异阵营目标（pick_stress_targets 已保证异阵营，此处冗余防御）
                        true
                    })
                    .map(|(_, tp, _)| *tp)
            } else {
                Some([
                    self.player_body.pos.x,
                    self.player_body.pos.y,
                    self.player_body.pos.z,
                ])
            };
            if let Some(tp) = target_pos {
                to_throw.push((i, tp));
            }
        }
        for (i, tp) in to_throw {
            let npc = &self.npcs[i];
            // 🔴 出手点必须抬到离地高度：脚底出手 = 高帧率下第一帧就被判"落地" → 自爆
            // （机理与现场证据见 `NPC_GRENADE_RELEASE_Y` 定义处）
            let origin = [
                npc.position[0],
                npc.position[1] + NPC_GRENADE_RELEASE_Y,
                npc.position[2],
            ];
            // 方向：本 NPC → 目标（水平）+ 上抛分量（与玩家投掷同链路，参数化）
            let dx = tp[0] - origin[0];
            let dz = tp[2] - origin[2];
            let len = (dx * dx + dz * dz).sqrt().max(1e-4);
            let dir = [dx / len, 0.35, dz / len];
            let fuse = GRENADE_FUSE_MIN
                + (npc.id as u32 * 17 % 1000) as f32 / 1000.0
                    * (GRENADE_FUSE_MAX - GRENADE_FUSE_MIN);
            self.grenades_vec
                .push(Grenade::new(origin, dir, GRENADE_SPEED * 0.9, fuse));
            log::info!(
                "grenade: npc #{} throws at ({:.0}, {:.0}) fuse={:.2}s",
                npc.id,
                tp[0],
                tp[2],
                fuse
            );
            // 冷却 10-18s（确定性伪随机）
            if let Some(npc) = self.npcs.get_mut(i) {
                npc.grenade_timer = 10.0 + (npc.id as f32 * 3.7 % 8.0);
            }
        }
    }
    /// 投射物推进 + 碰撞检测：物理刚体/球体命中即销毁；NPC 命中扣血，hp≤0 移除并计分。
    ///
    /// `allow_kills = false`（GameOver 冻结）：投射物照常飞行/到期，但不判定任何命中。
    pub(crate) fn update_projectiles(&mut self, dt: f32, allow_kills: bool) {
        // 弹道诊断（RV3D_PROJ_DIAG=1 时启用，节流 2s）：默认关闭避免生产日志噪音
        static LAST_PROJ_LOG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // 弹丸**去向**计数（2026-09-25 加）：harness 只有 `hits` 一个数，打了多少发、
        // 有多少打在掩体上、多少飞没了**都看不见** ⇒ "改瞄法到底有没有用"无法判定。
        // 三个计数器在下面三个结算分支里各加一，随 2s 诊断行一起打印并清零。
        static PROJ_OBSTACLE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static PROJ_NPC: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static PROJ_EXPIRED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // 过期弹**差多远**（分桶）：<0.5m = 差一点点（瞄点/抖动），0.5–2m = 接近，
        // >2m = 根本不是瞄着它飞的（瞄错目标/几何）。判"该怎么修"全靠这三格。
        static PROJ_MISS_NEAR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static PROJ_MISS_MID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static PROJ_MISS_FAR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let diag = proj_diag_on();
        if diag {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let last = LAST_PROJ_LOG.load(std::sync::atomic::Ordering::Relaxed);
            if now_ms - last > 2000 {
                LAST_PROJ_LOG.store(now_ms, std::sync::atomic::Ordering::Relaxed);
                use std::sync::atomic::Ordering::Relaxed;
                let obs = PROJ_OBSTACLE.swap(0, Relaxed);
                let npc = PROJ_NPC.swap(0, Relaxed);
                let exp = PROJ_EXPIRED.swap(0, Relaxed);
                let m_near = PROJ_MISS_NEAR.swap(0, Relaxed);
                let m_mid = PROJ_MISS_MID.swap(0, Relaxed);
                let m_far = PROJ_MISS_FAR.swap(0, Relaxed);
                let first = self.projectiles.first().map(|p| {
                    format!(
                        "pos=({:.0},{:.0},{:.0}) dist={:.0}m",
                        p.position[0],
                        p.position[1],
                        p.position[2],
                        p.distance_traveled()
                    )
                });
                let nearest_npc = self
                    .npcs
                    .iter()
                    .map(|n| {
                        let dx = n.position[0] - self.player_body.pos.x;
                        let dz = n.position[2] - self.player_body.pos.z;
                        (dx * dx + dz * dz).sqrt()
                    })
                    .fold(f32::MAX, f32::min);
                log::info!(
                    "proj-diag: alive={} first=[{}] nearest_npc={:.0}m 去向(2s) npc={} obstacle={} expired={} | 过期弹离最近 NPC: <0.5m={} 0.5-2m={} >2m={}",
                    self.projectiles.len(),
                    first.unwrap_or_else(|| "none".to_string()),
                    nearest_npc,
                    npc,
                    obs,
                    exp,
                    m_near,
                    m_mid,
                    m_far
                );
            }
        }
        for p in self.projectiles.iter_mut() {
            if p.is_alive() {
                p.update(dt);
                // 只诊断时算：这发弹离**最近 NPC 胸口**有多近（过期分桶用）。
                // 字段级借用互不相交（projectiles 可变 / npcs 只读），不必克隆。
                if diag {
                    let mut best = f32::MAX;
                    for n in &self.npcs {
                        let dx = n.position[0] - p.position[0];
                        let dy = n.position[1] + 1.2 - p.position[1];
                        let dz = n.position[2] - p.position[2];
                        let d = (dx * dx + dy * dy + dz * dz).sqrt();
                        if d < best {
                            best = d;
                        }
                    }
                    if best < p.min_npc_dist {
                        p.min_npc_dist = best;
                    }
                }
            }
        }
        // 弹着标记老化：先老化再生成（本帧刚打的孔从 age=0 开始）
        for m in self.impact_marks.iter_mut() {
            m.age += dt;
        }
        self.impact_marks.retain(|m| m.age < IMPACT_MARK_LIFE);
        let mut hit_count = 0u32;
        let old = std::mem::take(&mut self.projectiles);
        let mut alive = Vec::with_capacity(old.len());
        for p in old {
            if !p.is_alive() {
                PROJ_EXPIRED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if p.min_npc_dist < 0.5 {
                    PROJ_MISS_NEAR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else if p.min_npc_dist <= 2.0 {
                    PROJ_MISS_MID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else {
                    PROJ_MISS_FAR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                // 弹着点：仅爆炸弹（如未来榴弹武器）过期时引爆 AoE；普通子弹静默消失
                // （修复：V3.0 高速弹射程尽头"神秘连发爆炸"特效——子弹不是炸弹）
                if p.explosive() {
                    self.spawn_explosion(
                        p.position,
                        EXPLOSION_RADIUS,
                        if allow_kills { EXPLOSION_DAMAGE } else { 0.0 },
                        true,
                    );
                    self.audio.synth_mut().play_explosion(
                        glam::Vec3::new(p.position[0], p.position[1], p.position[2]),
                        0.45,
                    );
                }
                continue;
            }
            if !allow_kills {
                alive.push(p);
                continue;
            }
            if self.collide_physics(&p) {
                PROJ_OBSTACLE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // 命中障碍：子弹直接消耗（障碍永久存在，枪械不再打爆障碍；
                // 手榴弹/爆炸物 AoE 仍可摧毁掩体，见爆炸结算）。
                // 不计入 hit_count → 不触发命中提示/音效（打墙没有"命中反馈"）。
                // 爆炸弹命中才有 AoE。
                // 弹着标记（弹孔）：只在**障碍刚体**上留痕 —— 球体（树冠一类）与打空的子弹不留。
                if let Some((_, point, normal)) = self.first_obstacle_hit(&p) {
                    self.push_impact_mark(point, normal);
                }
                if self.hit_obstacle_index(&p).is_some() && p.explosive() {
                    self.spawn_explosion(p.position, EXPLOSION_RADIUS, EXPLOSION_DAMAGE, false);
                }
                continue;
            }
            if let Some((idx, hit_h)) = self.hit_npc_index(&p) {
                // 友军伤害关闭：玩家弹命中同阵营 NPC 穿身而过（不造成伤害/火花/命中计数）
                if p.from_player()
                    && self.npcs[idx].team == crate::engine::ai::Team::Blue
                {
                    alive.push(p);
                    continue;
                }
                hit_count += 1;
                PROJ_NPC.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // V3.0：部位倍率按武器分段表 + 已飞距离查表（投射物携带 part_tiers）
                let mult = p.part_multiplier(hit_h, self.npcs[idx].position[1]);
                let dmg = p.damage_at_distance() * mult;
                // 命中火花点（NPC 身体命中位置）+ 伤害飘字值
                self.hit_points.push([p.position[0], hit_h, p.position[2]]);
                self.hit_damages.push(dmg);
                log::info!(
                    "weapons: 命中 NPC #{} 高度={:.1} 倍率={:.1} 伤害={:.0} 距离={:.0}m",
                    self.npcs[idx].id,
                    hit_h - self.npcs[idx].position[1],
                    mult,
                    dmg,
                    p.distance_traveled()
                );
                self.damage_npc(idx, dmg, DamageSource::Player);
                continue;
            }
            // 玩家弹 vs 网络远端玩家（服务器权威命中：杀远端玩家）
            if p.from_player() && self.net_players.iter().any(|q| q.alive) {
                let hit = self.hit_net_player(&p);
                if hit {
                    hit_count += 1;
                    self.hud.show_hit_marker();
                    continue;
                }
            }
            alive.push(p);
        }
        self.projectiles = alive;
        self.hits += hit_count as u64;
        if hit_count > 0 {
            self.hud.show_hit_marker();
            let src = AudioSource::new(self.player_eye(), 1.0);
            self.sfx.play(
                &mut self.audio.mixer_mut(),
                SfxKind::Hit,
                src,
                Channel::Sfx,
                false,
            );
            log::info!(
                "weapons: {} projectile hit(s), total_hits={}",
                hit_count,
                self.hits
            );
        }
    }
    /// 生成爆炸实体：AoE 伤害 + 径向击退（生成时一次性结算，逆序遍历避免下标回移）。
    /// - 伤害衰减复用 `simd::shockwave_pressure`（所有 NPC 一次批算，指令集选路可测）；
    /// - `knockback=true` 时对命中 NPC 施加径向推挤（advance_npc 每帧指数衰减）；
    /// - 玩家在冲击半径内 → 震屏（`camera_shake_offset` 每帧读取）。
    /// 🔴 **爆炸冲击波是否被障碍挡住**（2026-09-15 补：此前 AoE 只看距离，`ob` 后面的目标
    /// 与开阔地一样吃满伤害 —— 隔着掩体炸不死人，因为掩体不存在）。
    ///
    /// 判据取自**爆心几何**，不是"障碍在不在半径内"：
    /// - 只考虑**爆心到目标之间**的障碍 ⇒ 目标背后的掩体不替它挡（那是背向的，挡不住冲击波）；
    /// - **包含爆心或目标的障碍一律跳过** —— 手榴弹贴在掩体上炸时，爆心在障碍 AABB 内部，
    ///   而那一格障碍自己就是被炸的对象，不能反过来把爆心"堵死"（全图只有一格、
    ///   其余全被挡住是最坏的结果）；目标站在障碍里同理（那是生成/推挤的异常态，
    ///   由距离衰减负责，不该让 blast 判成"被自己挡住"）。
    ///
    /// 与 `npc_occluded` 同一套 `segment_hits_aabb`（同一份几何代码，避免两套求交漂移），
    /// 差别只在**扫描的是 `map.obstacles` 而不是 `world.bodies`**：前者是玩法障碍
    /// （有血量、可摧毁），后者是物理刚体，而 AoE 伤害结算的正是前者。
    pub(crate) fn obstacle_blocks_blast(&self, from: [f32; 3], to: [f32; 3]) -> bool {
        self.map
            .obstacles
            .iter()
            .any(|ob| Self::blast_sample_blocked(from, to, ob))
    }
    /// 单个障碍的判定（拆出来是为了能直测"包含端点就跳过"这条规则）
    pub(crate) fn blast_sample_blocked(from: [f32; 3], to: [f32; 3], ob: &MapObstacle) -> bool {
        let aabb = Self::obstacle_aabb(ob);
        // 端点落在盒内 ⇒ 这一格不是"隔在中间"，不构成遮挡
        if Self::aabb_contains_point(&aabb, from) || Self::aabb_contains_point(&aabb, to) {
            return false;
        }
        Self::segment_hits_aabb(
            from[0], from[1], from[2], to[0], to[1], to[2], &aabb,
        )
    }
    /// 障碍的碰撞 AABB（碰撞体始终是 AABB、与 `shape` 无关 —— 见 `MapObstacle::shape` 注释）
    pub(crate) fn obstacle_aabb(ob: &MapObstacle) -> crate::engine::physics::Aabb {
        use crate::engine::physics::Vec3;
        crate::engine::physics::Aabb {
            min: Vec3::new(ob.x - ob.half_w, ob.y - ob.half_h, ob.z - ob.half_d),
            max: Vec3::new(ob.x + ob.half_w, ob.y + ob.half_h, ob.z + ob.half_d),
        }
    }
    pub(crate) fn aabb_contains_point(a: &crate::engine::physics::Aabb, p: [f32; 3]) -> bool {
        p[0] >= a.min.x
            && p[0] <= a.max.x
            && p[1] >= a.min.y
            && p[1] <= a.max.y
            && p[2] >= a.min.z
            && p[2] <= a.max.z
    }
    pub(crate) fn spawn_explosion(&mut self, center: [f32; 3], radius: f32, damage: f32, knockback: bool) {
        log::info!(
            "explosion: at ({:.1}, {:.1}, {:.1}) radius={:.0} dmg={:.0} knockback={}",
            center[0],
            center[1],
            center[2],
            radius,
            damage,
            knockback
        );
        self.explosions.push(Explosion {
            center,
            radius,
            max_damage: damage,
            age: 0.0,
            lifetime: EXPLOSION_LIFETIME,
        });
        // 供本次 AoE 结算期间拼击杀文本用（爆炸无单一击杀者，见字段注释）
        self.last_blast_center = center;
        // 玩家震屏 + 自伤：随距离线性衰减，最近处满强度（与 NPC 是否在场无关）。
        // 玩家自伤伤害封顶（max_damage * SELF_DAMAGE_CAP），爆炸中心偏移保证不被自己秒杀
        // （手榴弹上抛飞行 ~0.4s + 引信 1.5s → 玩家通常已远离落地中心）。
        let eye = self.player_eye();
        let dx = eye.x - center[0];
        let dz = eye.z - center[2];
        let dist = (dx * dx + dz * dz).sqrt();
        if dist < SHAKE_RADIUS {
            self.shake_timer = SHAKE_DURATION;
            self.shake_strength = SHAKE_STRENGTH * (1.0 - dist / SHAKE_RADIUS).clamp(0.15, 1.0);
        }
        // 玩家自伤：仅在半径内、游戏进行中、且**爆心与玩家之间没有障碍**；伤害 = 距离衰减 × 封顶系数
        // （挡在掩体后面就吃不到 —— 否则"躲起来"对爆炸无效）
        let player_point = [eye.x, eye.y, eye.z];
        if damage > 0.0
            && dist < radius
            && self.game_state == GameState::Playing
            && self.hud.health > 0.0
            && !self.player_invincible
            && !self.obstacle_blocks_blast(center, player_point)
        {
            let fall = 1.0 - (dist / radius).clamp(0.0, 1.0);
            let self_dmg = (damage * fall * SELF_DAMAGE_FACTOR).min(SELF_DAMAGE_CAP);
            if self_dmg > 0.0 {
                self.hud.health = (self.hud.health - self_dmg).max(0.0);
                log::info!(
                    "explosion: 玩家自伤 {:.0}（dist {:.1}m fall {:.2}）→ hp {:.0}",
                    self_dmg,
                    dist,
                    fall,
                    self.hud.health
                );
                if self.hud.health <= 0.0 {
                    // 手榴弹炸死自己：普通模式 GameOver；survive 规则 Defeat
                    if self.is_survive_rule() {
                        if let Some(obj) = self.obj_state.as_mut() {
                            obj.won_team = Some(crate::engine::ai::Team::Red);
                        }
                        self.game_state = GameState::Defeat;
                    } else {
                        self.game_state = GameState::GameOver;
                    }
                    // 死亡补给：全部武器弹匣补满 + 备弹恢复初始
                    self.weapons.reset_all_ammo();
                    log::info!("explosion: 玩家被自己手榴弹炸死（弹药已重置）");
                }
            }
        }
        // 障碍 AoE 伤害：爆炸半径内障碍施加冲击伤害（复用 damage_obstacle 血量体系，
        // 可摧毁掩体；环带障碍与 TOML 关卡障碍一致生效）。按距离线性衰减（半径边缘 0）。
        if damage > 0.0 && !self.map.obstacles.is_empty() {
            let r2 = radius * radius;
            let mut i = self.map.obstacles.len();
            while i > 0 {
                i -= 1;
                let ob = self.map.obstacles[i];
                // 障碍中心到爆炸中心距离（水平）
                let dx = ob.x - center[0];
                let dz = ob.z - center[2];
                let d2 = dx * dx + dz * dz;
                if d2 > r2 {
                    continue;
                }
                let fall = 1.0 - (d2.sqrt() / radius).clamp(0.0, 1.0);
                self.damage_obstacle(i, damage * fall * EXPLOSION_OBSTACLE_FACTOR);
            }
        }
        if damage <= 0.0 || self.npcs.is_empty() {
            return;
        }
        let points: Vec<[f32; 3]> = self.npcs.iter().map(|n| n.position).collect();
        let mut falloff = vec![0.0f32; points.len()];
        crate::engine::simd::shockwave_pressure(center, radius, 1.0, &points, &mut falloff);
        let mut i = self.npcs.len();
        while i > 0 {
            i -= 1;
            let f = falloff[i];
            if f <= 0.0 {
                continue;
            }
            if knockback {
                let dx = self.npcs[i].position[0] - center[0];
                let dz = self.npcs[i].position[2] - center[2];
                let d = (dx * dx + dz * dz).sqrt().max(1e-4);
                self.npcs[i].knockback[0] += dx / d * KNOCKBACK_SPEED * f;
                self.npcs[i].knockback[1] += dz / d * KNOCKBACK_SPEED * f;
            }
            // 🔴 冲击波要绕开障碍：隔着掩体的 NPC 不该吃这一发（此前只看压力衰减、
            // 完全不管中间有没有墙 —— 掩体对爆炸等于不存在）。
            let npc_point = [
                self.npcs[i].position[0],
                self.npcs[i].position[1] + NPC_HIT_CENTER_Y,
                self.npcs[i].position[2],
            ];
            if self.obstacle_blocks_blast(center, npc_point) {
                continue; // 击退已施加（冲击波推开掩体后的人），伤害不给
            }
            self.damage_npc(i, damage * f, DamageSource::Blast);
        }
    }
    /// 推进爆炸实体生命周期（年龄 + 过期移除）与震屏衰减。每帧调用（update 尾部）。
    pub(crate) fn step_explosions(&mut self, dt: f32) {
        for ex in self.explosions.iter_mut() {
            ex.age += dt;
        }
        self.explosions.retain(|ex| ex.age < ex.lifetime);
        self.shake_timer = (self.shake_timer - dt).max(0.0);
    }
    /// 当前爆炸实体（main.rs 每帧生成膨胀淡出的闪光 marker）
    pub(crate) fn explosions(&self) -> &[Explosion] {
        &self.explosions
    }
    /// 本帧爆炸震屏偏移（世界 x/z 抖动，随剩余时间线性衰减）；无震屏时返回 (0, 0)。
    ///
    /// ## 频率与幅度都是被实机反馈逼出来的，不要凭直觉调回去
    /// 旧实现是 `sin(t*47.13)` / `cos(t*53.71)`，即 **7.5Hz 与 8.5Hz**、幅度 ±0.35m。
    /// 两个问题：
    /// 1. **7.5–8.5Hz 高于人眼把抖动读成"震动"的频段**，落在"画面在抖"与"画面在闪"
    ///    之间；而枪模是屏幕锁定的（`view_inv` 抵消 view），于是**世界高频抖、枪不动**，
    ///    玩家报的"移动时整个画面大量撕裂和线条""高频小幅度摆动加大量残影"就是这个
    ///    相对运动。真实后坐/爆炸震屏在 1.5–3Hz 量级。
    /// 2. **幅度 0.35m 恰好等于玩家胶囊半径 0.35m**（`physics.rs PlayerBody::radius`）。
    ///    背贴墙时眼睛被推到墙面甚至墙内，配合 `near = 0.1` 与背面剔除，近端面被裁掉、
    ///    远端面被剔除 → **瞬间看穿墙**（用户报的"透视"里最隐蔽的一条）。
    /// 现在频率降到 2.2/2.6Hz（两轴刻意不成整数比，避免合成周期性图案），幅度上限
    /// 压到 0.12m，留出 0.23m 的墙面余量。
    pub(crate) fn camera_shake_offset(&self) -> (f32, f32) {
        if self.shake_timer <= 0.0 {
            return (0.0, 0.0);
        }
        let s = self.shake_strength * (self.shake_timer / SHAKE_DURATION);
        let t = self.time;
        ((t * 13.82).sin() * s, (t * 16.34).cos() * s)
    }
    /// 冲击波压力场 SIMD 实测（默认关，见 RV3D_EXPLOSION_SIM）：
    /// 爆心沿确定性圆周轨迹扫掠，64×64=4096 采样点波前每帧推进一次；
    /// 每秒输出一次指令集加速比基准突发（65536 点 × 32 轮：单帧 4096 点太小，
    /// 时钟噪声会淹没真实差距；突发取平均才可测出 AVX-512/AVX2 的浮点收益）。
    pub(crate) fn step_explosion_sim(&mut self) {
        if self.shock_points.is_empty() {
            // 64×64 采样网格覆盖 512m 场地（-256..256，与实例场同域）
            let n = 64usize;
            let step = 512.0 / n as f32;
            for iz in 0..n {
                for ix in 0..n {
                    let x = (ix as f32 - (n as f32 - 1.0) * 0.5) * step;
                    let z = (iz as f32 - (n as f32 - 1.0) * 0.5) * step;
                    self.shock_points.push([x, 1.0, z]);
                }
            }
            self.shock_out = vec![0.0f32; self.shock_points.len()];
        }
        if self.bench_points.is_empty() {
            // 256×256=65536 采样点：与实例场同密度，覆盖 512m 场地
            let n = 256usize;
            let step = 512.0 / n as f32;
            for iz in 0..n {
                for ix in 0..n {
                    let x = (ix as f32 - (n as f32 - 1.0) * 0.5) * step;
                    let z = (iz as f32 - (n as f32 - 1.0) * 0.5) * step;
                    self.bench_points.push([x, 1.0, z]);
                }
            }
        }
        // 爆心：确定性圆周扫掠（不依赖玩家输入，基准可复现）
        let center = [self.time.sin() * 40.0, 1.0, self.time.cos() * 40.0];
        let t0 = std::time::Instant::now();
        self.explosion_path = crate::engine::simd::shockwave_pressure(
            center,
            60.0,
            1000.0,
            &self.shock_points,
            &mut self.shock_out,
        );
        self.stage_explosion_us = t0.elapsed().as_micros() as u64;
        if self.time - self.last_explosion_log >= 1.0 {
            self.last_explosion_log = self.time;
            // 基准突发：65536 点 × 32 轮，选路 vs 标量各跑一遍取平均
            let rounds = 32u32;
            let n = self.bench_points.len();
            let mut simd_out = vec![0.0f32; n];
            let mut scalar_out = vec![0.0f32; n];
            let t1 = std::time::Instant::now();
            for _ in 0..rounds {
                self.explosion_path = crate::engine::simd::shockwave_pressure(
                    center,
                    60.0,
                    1000.0,
                    &self.bench_points,
                    &mut simd_out,
                );
            }
            let simd_us = (t1.elapsed().as_micros() as u64) / rounds as u64;
            let t2 = std::time::Instant::now();
            for _ in 0..rounds {
                crate::engine::simd::shockwave_pressure_scalar(
                    center,
                    60.0,
                    1000.0,
                    &self.bench_points,
                    &mut scalar_out,
                );
            }
            let scalar_us = (t2.elapsed().as_micros() as u64) / rounds as u64;
            let eq = simd_out == scalar_out;
            let speedup = scalar_us as f64 / simd_us.max(1) as f64;
            log::info!(
                "simd: path={} bench_points={} rounds={} simd_us={} scalar_us={} speedup={:.2}x bitwise_eq={}",
                self.explosion_path,
                n,
                rounds,
                simd_us,
                scalar_us,
                speedup,
                eq
            );
        }
    }
}
