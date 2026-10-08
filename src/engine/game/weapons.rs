// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Game {
    /// 尝试开火（受射速冷却限制）。`origin`/`direction` 来自相机；返回是否真的开火。
    /// 内部单发：不做冷却检查，直接扣弹发射并结算后坐（含连发热量压制）/音效/日志。
    /// 返回是否发射成功。冷却与连发节奏由 fire / fire_burst 控制。
    pub(crate) fn fire_shot(&mut self, origin: [f32; 3], direction: [f32; 3], from_player: bool) -> bool {
        let active_spec = ALL_WEAPONS
            .iter()
            .find(|s| s.name_zh == self.weapons.active_name());
        // V3.0 散射密度（MOA → 弧度）：半角半径 × spread_scale（开镜缩小散布）。
        // 均匀矩形扰动 ±half；霰弹全部 8 弹丸共用同一散布锥。
        let moa_half = active_spec
            .map(|s| s.spread_moa * 0.000_291_f32 * 0.5 * self.spread_scale)
            .unwrap_or(0.0);
        let fire_dir = if moa_half > 0.0 {
            let seed = self.shots.wrapping_mul(2_654_435_761) + 17;
            let r1 = ((seed % 10_000) as f32 / 10_000.0) - 0.5;
            let r2 = (((seed.wrapping_mul(4_052_877)) % 10_000) as f32 / 10_000.0) - 0.5;
            Self::spread_direction(direction, r1 * 2.0 * moa_half, r2 * 2.0 * moa_half)
        } else {
            direction
        };
        match self.weapons.active_firearm().try_fire(origin, fire_dir) {
            Some(mut projectile) => {
                if from_player {
                    projectile = projectile.with_player();
                }
                // 连发热量：连续射击累积（0..1），压制后续枪口上扬——前几发上扬大，
                // 连发后期上扬趋于稳定（模拟控枪）；停火后 update 中按速率衰减
                let heat = self.auto_heat;
                self.auto_heat = (self.auto_heat + 0.22).min(1.0);
                let recoil_comp = 1.0 - heat * 0.55;
                let (kick_pitch, kick_yaw) = self.weapons.active_firearm_ref().current_kick();
                self.pending_kick.0 += kick_pitch * recoil_comp;
                self.pending_kick.1 += kick_yaw * recoil_comp;
                self.projectiles.push(projectile);
                // 霰弹：每发 8 弹丸，只消耗 1 发弹药；其余弹丸带确定性锥形散布（±2.8°）
                if let Some(spec) = active_spec {
                    if spec.pellets > 1 {
                        let weapon = self.weapons.active_firearm_ref().weapon_ref();
                        for k in 1..spec.pellets {
                            let seed = self.shots * 31 + (k as u64) * 7;
                            let r1 = ((seed * 2_654_435_761) % 10_000) as f32 / 10_000.0;
                            let r2 = ((seed * 4_052_877 % 10_000) + 1) as f32 / 10_000.0;
                            let dir = Self::spread_direction(
                                fire_dir,
                                (r1 - 0.5) * 2.0 * moa_half.max(0.000_5),
                                (r2 - 0.5) * 2.0 * moa_half.max(0.000_5),
                            );
                            let mut sp = weapon.fire(origin, dir);
                            if from_player {
                                sp = sp.with_player();
                            }
                            self.projectiles.push(sp);
                        }
                    }
                }
                self.shots += 1;
                // 🔴 2026-09-25：**这一枪瞄得准不准**（`RV3D_PROJ_DIAG=1`）——与角度最小的
                // NPC 胸口之间的夹角。这是"空放"归因的最后一格：夹角普遍很小 ⇒ 子弹在飞、
                // 是**命中体/时序**问题；夹角普遍很大 ⇒ 目标根本不在准星附近（harness 侧）。
                if from_player && proj_diag_on() {
                    let mut best = (f32::MAX, -1i32, 0.0f32);
                    for n in &self.npcs {
                        let v = [
                            n.position[0] - origin[0],
                            n.position[1] + 1.2 - origin[1],
                            n.position[2] - origin[2],
                        ];
                        let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                        if d < 1e-3 {
                            continue;
                        }
                        let cos = (v[0] * fire_dir[0] + v[1] * fire_dir[1] + v[2] * fire_dir[2]) / d;
                        let ang = cos.clamp(-1.0, 1.0).acos().to_degrees();
                        if ang < best.0 {
                            best = (ang, n.id as i32, d);
                        }
                    }
                    if best.1 >= 0 {
                        log::info!(
                            "shot-aim: npc=#{} ang={:.1}deg dist={:.0}m",
                            best.1,
                            best.0,
                            best.2
                        );
                    }
                }
                // 程序化枪声：按武器类别选音色（步枪/冲锋/狙击/机枪/霰弹/手枪），
                // 带确定性音量抖动（0.95..=1.0）避免机械重复
                let shot_scale = 0.95 + 0.05 * ((self.shots % 5) as f32 / 4.0);
                let shot_params = match active_spec.map(|s| s.sound) {
                    Some(crate::engine::weapon_data::SoundKind::Smg) => crate::audio::THOMPSON_SHOT,
                    Some(crate::engine::weapon_data::SoundKind::Sniper) => crate::audio::SNIPER_SHOT,
                    Some(crate::engine::weapon_data::SoundKind::Lmg) => crate::audio::LMG_SHOT,
                    Some(crate::engine::weapon_data::SoundKind::Shotgun) => crate::audio::SHOTGUN_SHOT,
                    Some(crate::engine::weapon_data::SoundKind::Pistol) => crate::audio::PISTOL_SHOT,
                    _ => crate::audio::M1_SHOT,
                };
                self.audio.synth_mut().play_shot_with(
                    glam::Vec3::new(origin[0], origin[1], origin[2]),
                    shot_scale,
                    shot_params,
                );
                log::info!(
                    "weapons: shot #{} ({} alive) [{}]",
                    self.shots,
                    self.projectiles.len(),
                    self.weapons.active_name()
                );
                true
            }
            None => {
                // 空弹匣自动换弹（try_fire 内部触发）或换弹中：换弹提示音
                if self.weapons.active_firearm_ref().is_reloading() {
                    let src = AudioSource::new(
                        glam::Vec3::new(origin[0], origin[1], origin[2]),
                        1.0,
                    );
                    self.sfx.play(
                        &mut self.audio.mixer_mut(),
                        SfxKind::Reload,
                        src,
                        Channel::Sfx,
                        false,
                    );
                }
                false
            }
        }
    }
    /// 单发开火（受冷却与切枪计时约束）；返回是否发射。
    /// 内部路径（AI/网络/测试）发射的弹不带玩家标记（无友军伤害豁免）。
    pub(crate) fn fire(&mut self, origin: [f32; 3], direction: [f32; 3]) -> bool {
        if self.fire_cooldown > 0.0 {
            return false;
        }
        // 切枪计时中禁止开火（纯计时器，无动画）
        if self.weapons.is_switching() {
            return false;
        }
        let ok = self.fire_shot(origin, direction, false);
        if ok {
            self.fire_cooldown = self.weapons.active_firearm_ref().fire_interval();
        }
        ok
    }
    /// 玩家单发开火（main.rs 调用）：弹带玩家标记，命中友军（同阵营 NPC）穿身而过不造成伤害
    pub(crate) fn fire_player(&mut self, origin: [f32; 3], direction: [f32; 3]) -> bool {
        if self.fire_cooldown > 0.0 {
            return false;
        }
        if self.weapons.is_switching() {
            return false;
        }
        let ok = self.fire_shot(origin, direction, true);
        if ok {
            self.fire_cooldown = self.weapons.active_firearm_ref().fire_interval();
        }
        ok
    }
    /// 连发（Burst2/Burst3 共用这一条路径）：无视冷却快速连打 `rounds` 发，
    /// 之后强制冷却 `rounds ×` 间隔；返回实际发射数（弹匣打空即停）。
    /// 切枪计时中返回 0。`rounds` 由调用方给（`FireMode::burst_rounds()`）。
    /// `from_player` = true 时弹带玩家标记（友军豁免）；AI/网络侧传 false。
    ///
    /// 🔴 2026-09-23 复查：此前 `fire_burst` 与 `fire_burst_player` 是**两份逐行重复的函数体**
    /// （只差 `fire_shot(.., false)` / `fire_shot(.., true)`），而前者**没有任何调用方** ——
    /// 它靠一句 `#[allow(dead_code)]` 压着，旧注释还写着"AI/网络/测试用"（照它去找调用方会白找）。
    /// 现在合成一条路径 + 一个薄入口：重复没了，`from_player` 的语义写在签名上，
    /// 那条 `#[allow(dead_code)]` 也一并删掉（它能被删掉本身就是"没有重复"的证明）。
    pub(crate) fn fire_burst(
        &mut self,
        origin: [f32; 3],
        direction: [f32; 3],
        rounds: u32,
        from_player: bool,
    ) -> u32 {
        if self.weapons.is_switching() {
            return 0;
        }
        let mut n = 0u32;
        let rounds = rounds.max(1);
        for _ in 0..rounds {
            if self.fire_shot(origin, direction, from_player) {
                n += 1;
            } else {
                break;
            }
        }
        if n > 0 {
            self.fire_cooldown = self.weapons.active_firearm_ref().fire_interval() * rounds as f32;
        }
        n
    }
    /// 玩家三连发（main.rs 调用）：= [`Game::fire_burst`] 带玩家标记（友军豁免）
    pub(crate) fn fire_burst_player(&mut self, origin: [f32; 3], direction: [f32; 3], rounds: u32) -> u32 {
        self.fire_burst(origin, direction, rounds, true)
    }
    /// 设置散布缩放（main.rs 每帧按 ADS 混合更新：1.0 腰射 → 0.3 开镜）
    pub(crate) fn set_spread_scale(&mut self, scale: f32) {
        self.spread_scale = scale.clamp(0.1, 1.0);
    }
    /// 循环切换开火模式（B 键）：单发 → 三连发 → 连发
    /// X 键：开始打药。满血 / 没药 / 已在打药时**不消耗**（避免误按白扔一个包）。
    pub(crate) fn use_medkit(&mut self) {
        // 生命值放在 HudState（`hud.health` / `hud.max_health`），这是玩家血量的唯一来源 ——
        // 不要再往 Game 上加一个 hp 字段，那就是两套状态源。
        if self.heal_timer > 0.0 || self.medkits == 0 || self.hud.health >= self.hud.max_health {
            return;
        }
        self.medkits -= 1;
        self.heal_timer = HEAL_TIME;
        log::info!(
            "heal: 开始打药（剩余 {} 个，当前 hp={:.0}）",
            self.medkits,
            self.hud.health
        );
    }
    /// 打药进度 0..=1（未打药为 0；HUD 用它画进度）
    pub(crate) fn heal_progress(&self) -> f32 {
        if self.heal_timer <= 0.0 {
            0.0
        } else {
            (1.0 - self.heal_timer / HEAL_TIME).clamp(0.0, 1.0)
        }
    }
    /// 打药推进：计时归零**那一帧**一次性回血。
    /// 不做逐帧回血 —— 那样 HUD 没有明确的"完成"时刻，测试也不好断言。
    pub(crate) fn update_heal(&mut self, dt: f32) {
        if self.heal_timer <= 0.0 {
            return;
        }
        self.heal_timer -= dt;
        if self.heal_timer <= 0.0 {
            self.heal_timer = 0.0;
            let before = self.hud.health;
            self.hud.health = (before + HEAL_AMOUNT).min(self.hud.max_health);
            log::info!("heal: 完成 hp {:.0} -> {:.0}", before, self.hud.health);
        }
    }
    pub(crate) fn cycle_fire_mode(&mut self) {
        let modes = self.supported_fire_modes();
        self.fire_mode = next_supported_fire_mode(self.fire_mode(), modes);
        log::info!("weapons: 开火模式切换为 {}", self.fire_mode().label());
    }
    /// 当前开火模式
    /// 当前武器支持的档位表（由射速派生，见 `fire_modes_for`）
    pub(crate) fn supported_fire_modes(&self) -> &'static [FireMode] {
        let interval = self.weapons.active_firearm_ref().fire_interval();
        let rpm = if interval > 0.0 { 60.0 / interval } else { 0.0 };
        fire_modes_for(rpm)
    }
    /// 当前**生效**档位：始终夹进本武器支持的集合里。
    ///
    /// 故意做成"读的时候派生"而不是"切枪时重置一个字段"：后者需要一个切枪钩子，
    /// 一旦漏挂就会留下"拿着栓动狙击却还开着连发"的状态 —— 那正是两套状态源。
    /// 腰射准星扩散量 0..=1（HUD 按它缩放十字）。
    ///
    /// 2026-09-12 第⑤条：**这是本仓第一次给玩家"散布反馈"** ——
    /// 在此之前准星是固定 8px 半长，玩家读不出自己当前的散布状态，
    /// 而移动/姿态/连发恰恰是散布的三个主要来源。
    /// 三个输入量全都已经存在（`stance` / `sprinting()` / `fire_cooldown`），
    /// 所以这里不新增状态，只做一次纯函数式的合成。
    pub(crate) fn crosshair_spread(&self) -> f32 {
        // 站姿基准 0.30；蹲/趴收拢（更稳），冲刺张开（最不稳）
        let stance = match self.stance {
            Stance::Standing => 0.30,
            Stance::Crouching => 0.18,
            Stance::Prone => 0.10,
        };
        let sprint = if self.sprinting() { 0.25 } else { 0.0 };
        // 开火后坐期：fire_cooldown 是剩余秒数，按它线性张开，封顶 0.45
        let fire = (self.fire_cooldown * 3.0).clamp(0.0, 0.45);
        let base = (stance + sprint + fire).clamp(0.08, 1.0);
        // 🔴 2026-09-12 第⑤条补：**必须乘 `spread_scale`**。
        // `main.rs` 每帧按开镜混合度写它（`1.0 - ads_blend*0.7`，即开镜后弹道收拢 70%），
        // 而 `fire_dir` 的散射半角也乘了它。初版忘了这一项 ⇒
        // **玩家开镜后弹道已经收拢，准星却纹丝不动** —— 准星与实际散布不一致。
        (base * self.spread_scale).clamp(0.08, 1.0)
    }
    pub(crate) fn fire_mode(&self) -> FireMode {
        let modes = self.supported_fire_modes();
        if modes.contains(&self.fire_mode) {
            self.fire_mode
        } else {
            modes[0]
        }
    }
    /// 累计命中数（供 UI / 日志）
    pub(crate) fn hits(&self) -> u64 {
        self.hits
    }
    /// 是否处于开火后坐期（fire_cooldown > 0 = 刚开火，枪模后坐动画用）
    #[allow(dead_code)] // 历史接口：枪模后坐已改为一次性脉冲（main.rs 用 last_shot_at）
    pub(crate) fn is_firing(&self) -> bool {
        self.fire_cooldown > 0.0
    }
    /// 当前激活武器键名（weapon_data::WeaponSpec::key，第一人称枪模选择用）
    pub(crate) fn active_weapon_key(&self) -> &'static str {
        ALL_WEAPONS
            .get(self.weapons.active_index())
            .map(|s| s.key)
            .unwrap_or("hk416")
    }
    /// 切换武器（数字键/命令窗口/滚轮）：切换到指定槽位；切枪计时中忽略重复切换。
    /// 越界输入优雅回退：记录日志并忽略，不 panic。
    pub(crate) fn switch_weapon(&mut self, index: usize) {
        // 槽位越界防御（WeaponRack::len 暴露武器数量）
        if index >= self.weapons.len() {
            log::warn!(
                "weapons: 切枪回退——槽位 {} 越界（共 {} 槽），忽略本次切换",
                index,
                self.weapons.len()
            );
            return;
        }
        let prev = self.weapons.active_index();
        self.weapons.switch_to(index);
        self.log_switch_if_changed(prev, "切枪");
    }
    /// 切枪日志（两条路共用）：**只在真的换成了才打**，且 `tag` 区分是哪条路。
    ///
    /// 🔴 2026-09-26 补：滚轮那条路（`cycle_weapon`）原来**绕过这里直接调 rack**，
    /// 于是滚轮切枪在日志里**完全隐形** —— 当天写 `scripts/run_weapon_probe.ps1` 时
    /// 就据此得出了"滚轮没生效"的结论，其实是我的尺子量不到（教训 27 的又一例：
    /// **量不到 ≠ 现象不存在**）。现在两条路都留痕且标签不同，探针能分别计数。
    pub(crate) fn log_switch_if_changed(&self, prev: usize, tag: &str) {
        let now = self.weapons.active_index();
        if now != prev {
            log::info!(
                "weapons: {} {} -> {} ({})",
                tag,
                prev,
                now,
                self.weapons.active_name()
            );
        }
    }
    /// 循环切换武器（滚轮向上 = 下一把，向下 = 上一把；末尾回到 0 / 开头回到末尾）
    pub(crate) fn cycle_weapon(&mut self, delta: i32) {        let prev = self.weapons.active_index();
        if delta > 0 {
            self.weapons.switch_next();
        } else if delta < 0 {
            self.weapons.switch_prev();
        }
        self.log_switch_if_changed(prev, "滚轮切枪");
    }
    /// 🔴 **切枪动画进度** 0..=1（0 = 刚换手、枪在最低点；1 = 抬回瞄准位）。
    ///
    /// 2026-09-15 加：切枪此前只有"禁止开火"的计时器，**没有任何动作** ——
    /// 按下数字键后枪是直接跳变的。枪模（`main.rs::fp_gun_matrix`）每帧读这个值
    /// 做下坠/侧转。取不到武器（空架）时返回 1.0（= 无位移），与"没在切枪"同义。
    pub(crate) fn weapon_switch_progress(&self) -> f32 {
        self.weapons.switch_progress()
    }
    /// 请求换弹（R 键）；已在换弹/满弹匣/无备弹/切枪中时无副作用
    pub(crate) fn request_reload(&mut self) {
        if self.weapons.is_switching() {
            return;
        }
        let was_reloading = self.weapons.active_firearm_ref().is_reloading();
        self.weapons.active_firearm().start_reload();
        if !was_reloading && self.weapons.active_firearm_ref().is_reloading() {
            let src = AudioSource::new(self.player_eye(), 1.0);
            self.sfx.play(
                &mut self.audio.mixer_mut(),
                SfxKind::Reload,
                src,
                Channel::Sfx,
                false,
            );
        }
    }
    /// 调试补给（设置面板 N 键）：当前武器弹匣补满 + 手榴弹补满 + 提示音
    pub(crate) fn give_ammo(&mut self) {
        self.weapons.active_firearm().reset();
        self.grenades = self.grenades_max;
        self.medkits = self.medkits_max;
        self.heal_timer = 0.0;
        let src = AudioSource::new(self.player_eye(), 1.0);
        self.sfx.play(
            &mut self.audio.mixer_mut(),
            SfxKind::UiBlip,
            src,
            Channel::Sfx,
            false,
        );
    }
    /// 线段 [A,B] 与 AABB 求交，返回**入口参数 t ∈ [0,1] 与入口面所在轴**（0=x / 1=y / 2=z）。
    ///
    /// 只有这一份 slab 求交代码：`segment_hits_aabb` 是它的 `is_some()` 包装。
    /// 弹孔要的是"打中了没有"**和**"打在哪一面上"，两件事分开实现一定会漂移。
    pub(crate) fn segment_aabb_entry(
        ax: f32, ay: f32, az: f32,
        bx: f32, by: f32, bz: f32,
        aabb: &crate::engine::physics::Aabb,
    ) -> Option<(f32, usize)> {
        let (dx, dy, dz) = (bx - ax, by - ay, bz - az);
        let mut tmin: f32 = 0.0;
        let mut tmax: f32 = 1.0;
        let mut axis = 0usize;
        for (i, (d, a, lo, hi)) in [
            (dx, ax, aabb.min.x, aabb.max.x),
            (dy, ay, aabb.min.y, aabb.max.y),
            (dz, az, aabb.min.z, aabb.max.z),
        ]
        .into_iter()
        .enumerate()
        {
            if d.abs() < 1e-9 {
                if a < lo || a > hi {
                    return None;
                }
            } else {
                let t1 = (lo - a) / d;
                let t2 = (hi - a) / d;
                let (lo_t, hi_t) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
                if lo_t > tmin {
                    tmin = lo_t;
                    axis = i;
                }
                tmax = tmax.min(hi_t);
                if tmin > tmax {
                    return None;
                }
            }
        }
        Some((tmin, axis))
    }
    /// 线段 [A,B] 与 AABB 求交（slab 法，参数 t ∈ [0,1]）
    pub(crate) fn segment_hits_aabb(
        ax: f32, ay: f32, az: f32,
        bx: f32, by: f32, bz: f32,
        aabb: &crate::engine::physics::Aabb,
    ) -> bool {
        Self::segment_aabb_entry(ax, ay, az, bx, by, bz, aabb).is_some()
    }
    /// 线段命中的**第一个**障碍刚体 → (下标, 弹着点, 表面法线)。
    ///
    /// 扫描顺序与 `hit_obstacle_index` 一致（同序 `world.bodies`），两者用同一份求交，
    /// 所以"子弹被谁挡下"与"弹孔画在哪堵墙"永不会给出不同答案。法线取**入口面**的轴向。
    ///
    /// 🔴 弹着点就取碰撞 AABB 的入口面：2026-09-17 起 marker 的**可见尺寸 == 碰撞 AABB**
    /// （判据见 `geom::Shape::template_half_extent`），AABB 面就是画出来的那层面。
    /// 在此之前这里必须乘一个 `visual_half_gain = 2.0` 才能把弹孔推到墙皮上 ——
    /// 那是同一个"可见尺寸翻倍"约定的另一半，不一起改就会把弹孔埋进几何里
    /// （不崩不报，只是"打了枪墙上没有孔"，2026-09-15 实测踩过）。
    pub(crate) fn first_obstacle_hit(&self, p: &Projectile) -> Option<(usize, [f32; 3], [f32; 3])> {
        let a = p.prev_position();
        let b = p.position;
        // 🔴 取**参数 t 最小**的那个，不能取"列表里第一个命中的"：
        // `world.bodies` 的顺序是建关顺序，不是距离顺序 —— 列表里的远处障碍完全可能排在
        // 近处障碍前面。取错了不会报错，只是弹孔贴在一块**被前面那根柱子挡住**的面上，
        // 表现成"打了枪墙上没有孔"（2026-09-15 实测踩到，找了整整一轮）。
        let mut best: Option<(f32, usize, [f32; 3], [f32; 3])> = None;
        for (i, body) in self.world.bodies.iter().enumerate() {
            let aabb = body.aabb();
            let Some((t, axis)) =
                Self::segment_aabb_entry(a[0], a[1], a[2], b[0], b[1], b[2], &aabb)
            else {
                continue;
            };
            // t == 0：线段起点已经在盒内，**没有入口面**，也就没有可贴的表面。
            // 硬算会拿"起点 + 任意轴"当弹孔 —— 效果是一块悬在半空的暗方块。
            if t <= 0.0 {
                continue;
            }
            if best.is_some_and(|(bt, ..)| bt <= t) {
                continue;
            }
            let lo = [aabb.min.x, aabb.min.y, aabb.min.z];
            let hi = [aabb.max.x, aabb.max.y, aabb.max.z];
            let mut point = [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ];
            // 来射方向沿该轴为正 ⇒ 从负侧面进入 ⇒ 法线朝负方向（背向子弹来向）
            let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]][axis];
            let mut normal = [0.0f32; 3];
            normal[axis] = if d > 0.0 { -1.0 } else { 1.0 };
            // 弹着点取入口面本身：另外两轴保持命中点（弹孔不会横向漂移）。
            // 可见尺寸 == 碰撞 AABB 之后，AABB 面就是画出来的那层墙皮，不需要任何补偿倍率。
            point[axis] = if normal[axis] > 0.0 { hi[axis] } else { lo[axis] };
            best = Some((t, i, point, normal));
        }
        best.map(|(_, i, point, normal)| (i, point, normal))
    }
    /// 线段 [A,B] 与球体求交（二次方程判别式，参数 t ∈ [0,1]）
    pub(crate) fn segment_hits_sphere(
        ax: f32, ay: f32, az: f32,
        bx: f32, by: f32, bz: f32,
        s: &crate::engine::physics::SphereBody,
    ) -> bool {
        let (fx, fy, fz) = (ax - s.center.x, ay - s.center.y, az - s.center.z);
        let (dx, dy, dz) = (bx - ax, by - ay, bz - az);
        let a = dx * dx + dy * dy + dz * dz;
        let r2 = s.radius * s.radius;
        if a < 1e-9 {
            return fx * fx + fy * fy + fz * fz <= r2;
        }
        let b = 2.0 * (fx * dx + fy * dy + fz * dz);
        let c = fx * fx + fy * fy + fz * fz - r2;
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            return false;
        }
        let sq = disc.sqrt();
        let t1 = (-b - sq) / (2.0 * a);
        let t2 = (-b + sq) / (2.0 * a);
        t2 >= 0.0 && t1 <= 1.0
    }
    /// 投射物命中的障碍刚体下标（球体/未命中返回 None）。
    /// 下标与 map.obstacles/world.bodies 严格对应（apply_level 同序构建、同序移除）。
    /// 与 collide_physics 一致使用线段求交（高速弹不穿透掩体）。
    pub(crate) fn hit_obstacle_index(&self, p: &Projectile) -> Option<usize> {
        let (ax, ay, az) = (p.prev_position()[0], p.prev_position()[1], p.prev_position()[2]);
        let (bx, by, bz) = (p.position[0], p.position[1], p.position[2]);
        self.world.bodies.iter().position(|body| {
            let aabb = body.aabb();
            Self::segment_hits_aabb(ax, ay, az, bx, by, bz, &aabb)
        })
    }
    /// 障碍受伤结算：扣血至 0 → 摧毁（从物理刚体/AI 网格/渲染 marker 中移除）。
    /// 渲染侧无需改动：main.rs 每帧按 `map_obstacles()` 生成 marker，摧毁后自动不再绘制。
    pub(crate) fn damage_obstacle(&mut self, idx: usize, dmg: f32) {
        let Some(ob) = self.map.obstacles.get_mut(idx) else {
            return;
        };
        ob.hp = (ob.hp - dmg).max(0.0);
        if ob.hp > 0.0 {
            return;
        }
        let ob = *ob; // Copy：记录位置/尺寸供解除阻挡
        let kind = ob.kind;
        // 解除 AI 网格阻挡：NPC 寻路可穿过缺口，掩体点随之消失
        let g0 = world_to_grid(ob.x - ob.half_w, ob.z - ob.half_d);
        let g1 = world_to_grid(ob.x + ob.half_w, ob.z + ob.half_d);
        for gx in g0.x..=g1.x {
            for gz in g0.y..=g1.y {
                let pos = GridPos::new(gx, gz);
                if self.grid.in_bounds(pos) {
                    self.grid.clear(pos);
                }
            }
        }
        log::info!(
            "obstacle: #{} {:?} destroyed at ({:.1}, {:.1})",
            idx,
            kind,
            ob.x,
            ob.z
        );
        // world.bodies 与 map.obstacles 按序一一对应（程序化/关卡生成时同步建刚体）；
        // 测试注入的障碍无对应刚体 → 容忍 idx 越界（跳过刚体移除）
        if idx < self.world.bodies.len() {
            self.world.bodies.remove(idx);
        }
        self.map.obstacles.remove(idx);
    }
    /// 记入本轮阵亡（`round_kills_*` 的**唯一写入口**）。
    ///
    /// 🔴 2026-09-26 修：阵亡有**两条路** —— `damage_npc`（子弹/爆炸的主路径，当场把阵亡者移出
    /// `npcs`）与 `update_stress_respawns` 的兜底扫描（扫「还在数组里的 `hp <= 0`」）。以前只有
    /// 后一条在计数 ⇒ 主路径一个人都没计：170 秒 128v127 会战实测**自报阵亡 35、实际 68**
    /// （蓝营 79 vs 123；实际值由连强度反推：128 − 60）。指挥军情的 `kills` 字段直接读这个数，
    /// 于是司令看到的伤亡只有真相的一半。判据 = `every_death_path_is_counted_in_the_round_tally`。
    pub(crate) fn tally_round_deaths(&mut self, team: Team, count: u32) {
        if count == 0 {
            return;
        }
        match team {
            Team::Red => self.round_kills_red += count,
            Team::Blue => self.round_kills_blue += count,
        }
    }
    /// NPC 受伤结算：扣血至 0 → 移除 + 计分 + 任务目标推进；返回是否击杀。
    /// 调用方保证 `idx` 有效；下标移除后不再回移（调用方按逆序遍历或立即退出）。
    pub(crate) fn damage_npc(&mut self, idx: usize, dmg: f32, source: DamageSource) -> bool {
        let id = self.npcs[idx].id;
        // 受击反馈：命中瞬间闪白（0.15s 衰减）
        self.npc_hit_flash.insert(id, 0.15);
        let npc = &mut self.npcs[idx];
        npc.hp -= dmg;
        if npc.hp > 0.0 {
            return false;
        }
        let victim_team = npc.team;
        self.tally_round_deaths(victim_team, 1);
        self.npcs.remove(idx);
        // 🔴 2026-09-13 修：**只有击杀敌方才算战果**。
        // 原先这三处对**任何**击杀都生效（`score += KILL_SCORE`、`objective.progress(1)`、
        // `objective_register_kill()`），于是**打死队友也推进"歼灭敌人"**。
        // 用户 2026-09-13 实机截图就是这条：右上角 feed 里明明有 `YOU KILLED BLUE #174`
        // （自己人），HUD 却已经 `歼灭敌人 128/128` 并弹出 `VICTORY`，
        // 而小地图上红方还剩一大片 —— 计数把友军伤亡算成了战果。
        let is_enemy = victim_team != crate::engine::ai::Team::Blue;
        if is_enemy {
            self.score += KILL_SCORE;
        }
        // 击杀提示（右上角 feed）：敌我**都**提示 —— 打死自己人是需要立刻看见的事故。
        // 🔴 2026-09-15：现在**带上击杀者**（此前只报死者，玩家看不出是谁干的）。
        let victim = format!("{} #{id}", team_name(victim_team));
        let line = match source {
            DamageSource::Player => kill_line(KillerLabel::You, &victim, ""),
            DamageSource::Blast => blast_kill_line(self.last_blast_center, &victim),
        };
        self.hud.push_kill(line);
        log::info!(
            "kill: npc #{} eliminated (wave {}) team={:?} enemy={} score={}",
            id,
            self.wave,
            victim_team,
            is_enemy,
            self.score
        );
        // 任务目标：歼灭数推进（达成 → 胜利横幅/日志，波次推进不受影响）
        // ⚠️ 仅敌方：友军伤亡若也计入，一轮会在敌人尚存时提前"胜利"
        if is_enemy {
            if self.objective.progress(1) {
                self.on_objective_complete();
            }
            // 关卡系统：击杀计数（KillCount 规则用）
            self.objective_register_kill();
        }
        true
    }
    /// 投射物命中的 NPC 下标（segment-sphere 相交：上一帧位置→当前位置连线与命中球求交，
    /// 命中球中心在 NPC 头顶 +0.8、半径 0.8；高速弹（200m/s 每帧 3.3m）避免隧道效应漏判）
    /// 命中检测：返回 (NPC 下标, 命中点相对地面的高度)。
    /// 高度用于部位倍率判定（头 1.5 / 胸 1.0 / 臂 0.8 / 腿 0.6，见设计文档）。
    pub(crate) fn hit_npc_index(&self, p: &Projectile) -> Option<(usize, f32)> {
        let (ax, ay, az) = (p.prev_position()[0], p.prev_position()[1], p.prev_position()[2]);
        let (bx, by, bz) = (p.position[0], p.position[1], p.position[2]);
        let (dx, dy, dz) = (bx - ax, by - ay, bz - az);
        let len2 = dx * dx + dy * dy + dz * dz;
        if len2 < 1e-9 {
            return None;
        }
        for (i, npc) in self.npcs.iter().enumerate() {
            let cx = npc.position[0];
            // 命中球：中心 +1.0m、半径 1.05m → 覆盖 -0.05..2.05m（含头+余量）。
            // 2026-08-19 修复：玩家眼位 y≈2.0，水平弹道从旧球顶（1.85m）上方掠过
            // → 瞄准身体也打不中（头顶掠过 miss）——加大后水平弹道可命中。
            // 注意：球心高度与下方 hit_height 的 +NPC_HIT_CENTER_Y 必须一致。
            let cy = npc.position[1] + NPC_HIT_CENTER_Y;
            let cz = npc.position[2];
            let r = 1.05;
            // 点到射线最近点参数 t（clamp 到 [0,1] 段内），再算距离²
            let fx = ax - cx;
            let fy = ay - cy;
            let fz = az - cz;
            let t = -(fx * dx + fy * dy + fz * dz) / len2;
            let t = t.clamp(0.0, 1.0);
            let qx = fx + t * dx;
            let qy = fy + t * dy;
            let qz = fz + t * dz;
            if qx * qx + qy * qy + qz * qz <= r * r {
                // 命中点世界高度 = 球心 + 最近点相对偏移 qy + NPC 地面 y
                let hit_height = qy + npc.position[1] + NPC_HIT_CENTER_Y;
                return Some((i, hit_height));
            }
        }
        None
    }
    /// 部位伤害倍率（固定阈值回退表：头部 ×1.5、胸部 ×1.0、手臂 ×0.8、腿部 ×0.6）。
    /// 命中高度 1.45 以上为头、0.95~1.45 胸、0.6~0.95 臂、以下为腿（NPC 身高 ~1.78m）。
    /// V3.0 起生产路径走 Projectile::part_multiplier（按武器分段表）；此表仅供测试与
    /// 无分段投射物（旧接口）回退参考。
    #[allow(dead_code)]
    pub(crate) fn part_multiplier(hit_height: f32, npc_ground_y: f32) -> f32 {
        let h = hit_height - npc_ground_y;
        if h >= 1.45 {
            1.5
        } else if h >= 0.95 {
            1.0
        } else if h >= 0.6 {
            0.8
        } else {
            0.6
        }
    }
    /// 在瞄准方向上加锥形散布：dx/dy 为相对方向的偏移角（弧度）。
    /// 以方向为 Z 轴构建正交基（right/up），返回归一化后的散布方向。
    pub(crate) fn spread_direction(direction: [f32; 3], dx: f32, dy: f32) -> [f32; 3] {
        let len = (direction[0] * direction[0]
            + direction[1] * direction[1]
            + direction[2] * direction[2])
            .sqrt()
            .max(1e-6);
        let d = [direction[0] / len, direction[1] / len, direction[2] / len];
        // right = normalize(cross(d, up))，up 退化（垂直射）时退回 +X
        let up = [0.0f32, 1.0, 0.0];
        let mut rx = d[1] * up[2] - d[2] * up[1];
        let mut ry = d[2] * up[0] - d[0] * up[2];
        let mut rz = d[0] * up[1] - d[1] * up[0];
        let rl = (rx * rx + ry * ry + rz * rz).sqrt();
        if rl < 1e-6 {
            rx = 1.0;
            ry = 0.0;
            rz = 0.0;
        } else {
            rx /= rl;
            ry /= rl;
            rz /= rl;
        }
        // up2 = cross(right, d)
        let ux = ry * d[2] - rz * d[1];
        let uy = rz * d[0] - rx * d[2];
        let uz = rx * d[1] - ry * d[0];
        let mut ox = d[0] + rx * dx + ux * dy;
        let mut oy = d[1] + ry * dx + uy * dy;
        let mut oz = d[2] + rz * dx + uz * dy;
        let ol = (ox * ox + oy * oy + oz * oz).sqrt().max(1e-6);
        ox /= ol;
        oy /= ol;
        oz /= ol;
        [ox, oy, oz]
    }
}
