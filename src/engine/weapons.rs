//! 武器系统模块
//!
//! - `Weapon` 特征：所有武器的通用属性（名称、伤害、射速、射程）与派生行为（开火间隔、射程判定、DPS）
//! - `MeleeWeapon`：近战武器，基于射程与挥击夹角判定命中，不产生投射物
//! - `ProjectileWeapon`：投射物武器，发射沿直线运动的 `Projectile`
//! - `Projectile`：投射物，直线运动 + 生命周期（飞行距离超过射程或存活超过寿命即销毁）
//! - `Firearm`：弹匣武器状态机，封装弹匣/换弹/后坐力手感，集成时由游戏侧每帧驱动
//! - `thompson_smg` / `thompson_smg_firearm`：汤姆森冲锋枪工厂（低伤高射速中距压制）
//! - `WeaponRack`：多武器槽切换状态机，支持循环切换与切换计时
//! - `Grenade`：手榴弹投掷物（抛物线弹道 + 引信计时，到期爆炸）
//!
//! 当前仅使用标准库，无第三方依赖；如将来需要新增依赖，在文件头部按
//! `// DEP: crate = version` 格式声明。

/// 距离衰减查表：tiers 为 (距离上限米, 该档伤害)，按已飞距离返回档位伤害；
/// 空表返回基础伤害（兼容旧接口）；超出最后一档按最远档 60% 兜底。
pub fn tiered_damage(base: f32, tiers: &[(f32, f32)], dist: f32) -> f32 {
    if tiers.is_empty() {
        return base;
    }
    for (limit, dmg) in tiers {
        if dist <= *limit {
            return *dmg;
        }
    }
    let last = tiers.last().map(|(_, d)| *d).unwrap_or(base);
    (last * 0.6).max(1.0)
}

/// 武器特征：近战与投射物武器的公共接口
#[allow(dead_code)] // 完整接口预留：MeleeWeapon 未接线，主循环仅用 ProjectileWeapon::fire_interval/fire
pub trait Weapon {
    /// 武器名称
    fn name(&self) -> &'static str;
    /// 单次攻击造成的伤害
    fn damage(&self) -> f32;
    /// 射速：每秒攻击/发射次数
    fn fire_rate(&self) -> f32;
    /// 有效射程（米）
    fn range(&self) -> f32;

    /// 两次攻击之间的间隔（秒），由射速推导
    fn fire_interval(&self) -> f32 {
        1.0 / self.fire_rate().max(f32::EPSILON)
    }

    /// 目标距离是否处于有效射程内
    fn in_range(&self, distance: f32) -> bool {
        distance <= self.range()
    }

    /// 平均每秒伤害（DPS）= 伤害 × 射速
    fn dps(&self) -> f32 {
        self.damage() * self.fire_rate()
    }
}

/// 近战武器：挥击造成伤害，命中判定依赖射程与夹角
#[allow(dead_code)]
pub struct MeleeWeapon {
    name: &'static str,
    damage: f32,
    fire_rate: f32,
    range: f32,
    /// 挥击夹角（弧度），用于扇形命中判定
    arc: f32,
}

impl MeleeWeapon {
    /// 创建近战武器
    #[allow(dead_code)]
    pub fn new(name: &'static str, damage: f32, fire_rate: f32, range: f32, arc: f32) -> Self {
        Self {
            name,
            damage,
            fire_rate,
            range,
            arc,
        }
    }

    /// 挥击夹角（弧度）
    #[allow(dead_code)] // MeleeWeapon 预留，近战接线时启用
    pub fn arc(&self) -> f32 {
        self.arc
    }
}

impl Weapon for MeleeWeapon {
    fn name(&self) -> &'static str {
        self.name
    }

    fn damage(&self) -> f32 {
        self.damage
    }

    fn fire_rate(&self) -> f32 {
        self.fire_rate
    }

    fn range(&self) -> f32 {
        self.range
    }
}

/// 投射物武器：发射沿直线运动的投射物
#[allow(dead_code)]
pub struct ProjectileWeapon {
    name: &'static str,
    damage: f32,
    fire_rate: f32,
    range: f32,
    /// 投射物飞行速度（米/秒）
    projectile_speed: f32,
    /// 投射物最长存活时间（秒），超出即销毁
    projectile_lifetime: f32,
    /// 距离衰减档位：(距离上限米, 该档胸部伤害)；空表 = 恒伤害（兼容旧接口）
    damage_tiers: &'static [(f32, f32)],
}

impl ProjectileWeapon {
    /// 创建投射物武器
    #[allow(dead_code)]
    pub fn new(
        name: &'static str,
        damage: f32,
        fire_rate: f32,
        range: f32,
        projectile_speed: f32,
        projectile_lifetime: f32,
    ) -> Self {
        Self {
            name,
            damage,
            fire_rate,
            range,
            projectile_speed,
            projectile_lifetime,
            damage_tiers: &[],
        }
    }

    /// 创建带距离衰减档位的投射物武器：tiers 为 (距离上限, 该档伤害) 列表，
    /// 超出最后一档按最远档 60% 兜底（设计文档：超出有效射程快速衰减）。
    pub fn new_tiered(
        name: &'static str,
        damage: f32,
        fire_rate: f32,
        range: f32,
        projectile_speed: f32,
        projectile_lifetime: f32,
        damage_tiers: &'static [(f32, f32)],
    ) -> Self {
        Self {
            name,
            damage,
            fire_rate,
            range,
            projectile_speed,
            projectile_lifetime,
            damage_tiers,
        }
    }

    /// 投射物飞行速度（米/秒）
    #[allow(dead_code)] // getter 预留：fire() 内部直接读字段
    pub fn projectile_speed(&self) -> f32 {
        self.projectile_speed
    }

    /// 投射物最长存活时间（秒）
    #[allow(dead_code)] // getter 预留：fire() 内部直接读字段
    pub fn projectile_lifetime(&self) -> f32 {
        self.projectile_lifetime
    }

    /// 发射一枚投射物：`origin` 为出膛位置，`direction` 为发射方向（自动归一化）
    pub fn fire(&self, origin: [f32; 3], direction: [f32; 3]) -> Projectile {
        Projectile::new_tiered(
            origin,
            direction,
            self.projectile_speed,
            self.range,
            self.projectile_lifetime,
            self.damage,
            self.damage_tiers,
        )
    }
}

impl Weapon for ProjectileWeapon {
    fn name(&self) -> &'static str {
        self.name
    }

    fn damage(&self) -> f32 {
        self.damage
    }

    fn fire_rate(&self) -> f32 {
        self.fire_rate
    }

    fn range(&self) -> f32 {
        self.range
    }
}

/// 投射物：沿直线匀速运动，超过射程或寿命后自动销毁
#[allow(dead_code)]
pub struct Projectile {
    /// 当前位置（x, y, z）
    pub position: [f32; 3],
    /// 速度向量（米/秒）
    velocity: [f32; 3],
    /// 已飞行距离（米）
    distance_traveled: f32,
    /// 已存活时间（秒）
    age: f32,
    /// 最长存活时间（秒）
    max_lifetime: f32,
    /// 最大射程（米）
    range: f32,
    /// 命中伤害（基础档；距离衰减查表见 damage_at_distance）
    pub damage: f32,
    /// 距离衰减档位：(距离上限米, 该档伤害)；空表 = 恒伤害
    damage_tiers: &'static [(f32, f32)],
    /// 是否存活（射程/寿命耗尽后为 false）
    alive: bool,
    /// 上一帧位置（segment 命中检测用；高速弹避免跳过小目标）
    prev_position: [f32; 3],
}

impl Projectile {
    /// 创建投射物：`origin` 为起点，`direction` 为发射方向（自动归一化）
    #[allow(dead_code)]
    pub fn new(
        origin: [f32; 3],
        direction: [f32; 3],
        speed: f32,
        range: f32,
        max_lifetime: f32,
        damage: f32,
    ) -> Self {
        let length = (direction[0] * direction[0]
            + direction[1] * direction[1]
            + direction[2] * direction[2])
            .sqrt();
        let inv = if length > f32::EPSILON {
            1.0 / length
        } else {
            0.0
        };
        let dir = [direction[0] * inv, direction[1] * inv, direction[2] * inv];
        Self {
            position: origin,
            prev_position: origin,
            velocity: [dir[0] * speed, dir[1] * speed, dir[2] * speed],
            distance_traveled: 0.0,
            age: 0.0,
            max_lifetime,
            range,
            damage,
            damage_tiers: &[],
            alive: true,
        }
    }

    /// 创建带距离衰减档位的投射物（见 ProjectileWeapon::new_tiered）
    pub fn new_tiered(
        origin: [f32; 3],
        direction: [f32; 3],
        speed: f32,
        range: f32,
        max_lifetime: f32,
        damage: f32,
        damage_tiers: &'static [(f32, f32)],
    ) -> Self {
        let mut p = Self::new(origin, direction, speed, range, max_lifetime, damage);
        p.damage_tiers = damage_tiers;
        p
    }

    /// 按已飞行距离查表得到应结算伤害（含距离衰减；空表返回基础伤害）
    pub fn damage_at_distance(&self) -> f32 {
        tiered_damage(self.damage, self.damage_tiers, self.distance_traveled)
    }

    /// 是否仍存活（射程/寿命耗尽后返回 false）
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// 上一帧位置（segment 命中检测用：高速弹避免隧道效应跳过目标）
    pub fn prev_position(&self) -> [f32; 3] {
        self.prev_position
    }

    /// 已飞行距离（米）
    #[allow(dead_code)] // 调试/诊断预留 getter
    pub fn distance_traveled(&self) -> f32 {
        self.distance_traveled
    }

    /// 速度向量（米/秒；供 AI 火力威胁感知判断子弹是否朝自身飞来）
    pub fn velocity(&self) -> [f32; 3] {
        self.velocity
    }

    /// 已存活时间（秒）
    #[allow(dead_code)] // 调试/诊断预留 getter
    pub fn age(&self) -> f32 {
        self.age
    }

    /// 按时间步长推进：直线运动；飞行距离超过射程或存活超过寿命即销毁
    pub fn update(&mut self, dt: f32) {
        if !self.alive || dt <= 0.0 {
            return;
        }
        self.prev_position = self.position;
        self.position = [
            self.position[0] + self.velocity[0] * dt,
            self.position[1] + self.velocity[1] * dt,
            self.position[2] + self.velocity[2] * dt,
        ];
        let speed = (self.velocity[0] * self.velocity[0]
            + self.velocity[1] * self.velocity[1]
            + self.velocity[2] * self.velocity[2])
            .sqrt();
        self.distance_traveled += speed * dt;
        self.age += dt;
        if self.distance_traveled >= self.range || self.age >= self.max_lifetime {
            self.alive = false;
        }
    }
}

/// 弹匣武器状态机：在 `ProjectileWeapon` 之上封装弹匣、换弹与后坐力手感
///
/// 集成时由游戏侧每帧驱动：扣扳机调用 `try_fire`，每帧调用 `update` 推进换弹，
/// 开火后读取 `current_kick` 施加相机后坐力。
pub struct Firearm {
    /// 底层投射物武器（伤害/射速/射程/投射物属性）
    weapon: ProjectileWeapon,
    /// 当前弹匣内弹药
    magazine: u32,
    /// 弹匣容量
    max_magazine: u32,
    /// 备弹（弹匣外）
    reserve: u32,
    /// 初始备弹（死亡重置弹药时恢复到此值）
    reserve_max: u32,
    /// 换弹所需时间（秒）
    reload_time: f32,
    /// 换弹剩余时间（秒）
    reload_timer: f32,
    /// 是否正在换弹
    reloading: bool,
    /// 每发上跳后坐力（弧度），默认 0.014 ≈ 0.8°
    kick_pitch: f32,
    /// 每发水平后坐力（弧度），默认 0.004
    kick_yaw: f32,
    /// 累计发射数，用于生成确定性的后坐力微扰
    shots_fired: u32,
}

impl Firearm {
    /// 创建弹匣武器：初始弹匣装满 `max_magazine` 发，`reserve` 为备弹
    pub fn new(
        weapon: ProjectileWeapon,
        max_magazine: u32,
        reserve: u32,
        reload_time: f32,
        kick_pitch: f32,
        kick_yaw: f32,
    ) -> Self {
        Self {
            weapon,
            magazine: max_magazine,
            max_magazine,
            reserve,
            reserve_max: reserve,
            reload_time,
            reload_timer: 0.0,
            reloading: false,
            kick_pitch,
            kick_yaw,
            shots_fired: 0,
        }
    }

    /// 是否可开火：未在换弹且弹匣内有弹药
    pub fn can_fire(&self) -> bool {
        !self.reloading && self.magazine > 0
    }

    /// 底层投射物武器引用（霰弹复刻弹丸、调试用）
    pub fn weapon_ref(&self) -> &ProjectileWeapon {
        &self.weapon
    }

    /// 当前弹匣内弹药
    pub fn magazine(&self) -> u32 {
        self.magazine
    }

    /// 弹匣容量
    pub fn max_magazine(&self) -> u32 {
        self.max_magazine
    }

    /// 备弹（弹匣外）
    pub fn reserve(&self) -> u32 {
        self.reserve
    }

    /// 是否正在换弹
    pub fn is_reloading(&self) -> bool {
        self.reloading
    }

    /// 重置为满弹匣、非换弹状态（新一局/复活用）
    pub fn reset(&mut self) {
        self.magazine = self.max_magazine;
        self.reloading = false;
        self.reload_timer = 0.0;
        self.shots_fired = 0;
    }

    /// 重置全部弹药：弹匣补满 + 备弹恢复初始值 + 取消换弹（死亡复活补给用）
    pub fn reset_ammo(&mut self) {
        self.magazine = self.max_magazine;
        self.reserve = self.reserve_max;
        self.reloading = false;
        self.reload_timer = 0.0;
        self.shots_fired = 0;
    }

    /// 尝试发射：可开火时扣弹并返回投射物；空弹匣自动开始换弹并返回 None；
    /// 换弹期间返回 None 且不打断换弹
    pub fn try_fire(&mut self, origin: [f32; 3], direction: [f32; 3]) -> Option<Projectile> {
        if !self.can_fire() {
            // 空弹匣（且未在换弹）自动开始换弹；换弹中不重复触发
            if self.magazine == 0 && !self.reloading {
                self.start_reload();
            }
            return None;
        }
        self.magazine -= 1;
        self.shots_fired += 1;
        Some(self.weapon.fire(origin, direction))
    }

    /// 开始换弹：弹匣未满、有备弹且未在换弹时才生效
    pub fn start_reload(&mut self) {
        if self.magazine < self.max_magazine && self.reserve > 0 && !self.reloading {
            self.reloading = true;
            self.reload_timer = self.reload_time;
        }
    }

    /// 按时间步长推进换弹；完成后从备弹补满弹匣（备弹不足则只补剩余）
    pub fn update(&mut self, dt: f32) {
        if !self.reloading || dt <= 0.0 {
            return;
        }
        self.reload_timer -= dt;
        if self.reload_timer <= 0.0 {
            let take = (self.max_magazine - self.magazine).min(self.reserve);
            self.magazine += take;
            self.reserve -= take;
            self.reloading = false;
        }
    }

    /// 本次射击应施加的相机后坐力：上跳 + 确定性微扰，水平后坐力
    pub fn current_kick(&self) -> (f32, f32) {
        let perturbation = (self.shots_fired % 5) as f32 * 0.001;
        (self.kick_pitch + perturbation, self.kick_yaw)
    }

    /// 弹匣余量比例（0.0 ~ 1.0）
    pub fn ammo_ratio(&self) -> f32 {
        self.magazine as f32 / self.max_magazine.max(1) as f32
    }

    /// 换弹进度（reload_timer / reload_time，从 1.0 递减到 0.0）；未换弹返回 1.0
    pub fn reload_progress(&self) -> f32 {
        if !self.reloading || self.reload_time <= 0.0 {
            return 1.0;
        }
        (self.reload_timer / self.reload_time).clamp(0.0, 1.0)
    }

    /// 射速冷却间隔（秒），转发底层武器语义
    pub fn fire_interval(&self) -> f32 {
        self.weapon.fire_interval()
    }
}

/// 汤姆森冲锋枪（Thompson SMG）工厂：低伤高射速的中距压制武器
///
/// 与 M1 Rifle（25 伤 × 3/s 远距精准）差异化：单发 12 伤 × 10/s（DPS 更高、
/// 压制力强，但单发停火能力弱）；弹匣/备弹/后坐力由 `thompson_smg_firearm` 封装。
#[allow(dead_code)] // 历史工厂：保留供测试与诊断使用（现行武器走 weapon_data 数据表）
pub fn thompson_smg() -> ProjectileWeapon {
    ProjectileWeapon::new(
        "Thompson SMG",
        12.0,  // 单发伤害
        10.0,  // 射速 10 发/秒
        140.0, // 有效射程（中距离）
        120.0, // 投射物速度 120 米/秒
        1.2,   // 投射物寿命 1.2 秒（120×1.2=144m ≥ 射程 140m，射程先耗尽）
    )
}

/// 汤姆森冲锋枪弹匣武器：弹匣 30 / 备弹 120，换弹 2.2s，
/// 后坐力大于 M1（kick_pitch 0.014 / kick_yaw 0.004）以体现中距离散布略大。
#[allow(dead_code)] // 历史工厂：保留供测试与诊断使用（现行武器走 weapon_data 数据表）
pub fn thompson_smg_firearm() -> Firearm {
    Firearm::new(
        thompson_smg(),
        30,   // 弹匣容量
        120,  // 备弹
        2.2,  // 换弹时间（秒）
        0.02, // 每发上跳后坐力（弧度）
        0.006, // 每发水平后坐力（弧度）
    )
}

/// 多武器槽切换状态机：持有若干 (名称, Firearm) 武器槽，支持循环切换与切换计时。
///
/// 集成语义：切换期间（`is_switching` 为 true）游戏侧禁止开火/换弹；
/// 每帧调用 `update` 推进切换计时。纯计时器，无动画。
pub struct WeaponRack {
    /// 武器槽列表（名称, 弹匣武器）
    weapons: Vec<(String, Firearm)>,
    /// 当前激活槽索引
    active: usize,
    /// 切换剩余时间（秒）
    switch_timer: f32,
    /// 单次切换所需时间（秒）
    switch_time: f32,
}

impl WeaponRack {
    /// 创建武器架：初始激活 0 号槽，未在切换中
    pub fn new(pairs: Vec<(String, Firearm)>, switch_time: f32) -> Self {
        Self {
            weapons: pairs,
            active: 0,
            switch_timer: 0.0,
            switch_time: switch_time.max(0.0),
        }
    }

    /// 当前激活槽的武器名（空架返回空串）
    pub fn active_name(&self) -> &str {
        self.weapons
            .get(self.active)
            .map(|(name, _)| name.as_str())
            .unwrap_or("")
    }

    /// 当前激活武器的可变引用（开火/换弹/补给）
    pub fn active_firearm(&mut self) -> &mut Firearm {
        &mut self.weapons[self.active].1
    }

    /// 当前激活武器的不可变引用
    pub fn active_firearm_ref(&self) -> &Firearm {
        &self.weapons[self.active].1
    }

    /// 当前激活槽索引（供第一人称枪模按槽位取武器规格）
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// 切换到指定槽位：索引有效且非当前才生效，并启动切换计时
    pub fn switch_to(&mut self, index: usize) {
        if index < self.weapons.len() && index != self.active {
            self.active = index;
            self.switch_timer = self.switch_time;
        }
    }

    /// 循环切换到下一个槽位（末尾回到 0）
    pub fn switch_next(&mut self) {
        if !self.weapons.is_empty() {
            self.switch_to((self.active + 1) % self.weapons.len());
        }
    }

    /// 循环切换到上一个槽位（开头回到末尾）
    pub fn switch_prev(&mut self) {
        if !self.weapons.is_empty() {
            self.switch_to((self.active + self.weapons.len() - 1) % self.weapons.len());
        }
    }

    /// 按时间步长推进切换计时（向 0 收敛；dt<=0 不推进）+ 当前武器的换弹/冷却计时
    pub fn update(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        self.switch_timer = (self.switch_timer - dt).max(0.0);
        if let Some((_, firearm)) = self.weapons.get_mut(self.active) {
            firearm.update(dt);
        }
    }

    /// 是否正在切换（切换期间禁止开火/换弹）
    pub fn is_switching(&self) -> bool {
        self.switch_timer > 0.0
    }

    /// 重置全部槽位弹药：每把枪弹匣补满 + 备弹恢复初始（死亡补给/重开一局用）
    pub fn reset_all_ammo(&mut self) {
        for (_, firearm) in self.weapons.iter_mut() {
            firearm.reset_ammo();
        }
    }

    /// 武器槽数量
    pub fn len(&self) -> usize {
        self.weapons.len()
    }
}

/// 手榴弹初速（米/秒），供主会话投掷时使用
pub const GRENADE_SPEED: f32 = 18.0;
/// 引信最短时长（秒）
pub const GRENADE_FUSE_MIN: f32 = 1.5;
/// 引信最长时长（秒）
pub const GRENADE_FUSE_MAX: f32 = 2.5;
/// 手榴弹重力加速度（米/秒²）
pub const GRENADE_GRAVITY: f32 = 9.8;

/// 手榴弹投掷物：抛物线弹道 + 引信计时，引信到期即 `exploded`（爆炸效果由游戏侧
/// `spawn_explosion` 触发）。
pub struct Grenade {
    /// 当前位置（x, y, z）
    pos: [f32; 3],
    /// 速度向量（米/秒），垂直分量每帧受重力修正
    vel: [f32; 3],
    /// 引信剩余时间（秒）
    fuse: f32,
    /// 引信总时长（秒），与传入 fuse 一致，供 HUD/进度展示
    fuse_max: f32,
    /// 重力加速度（米/秒²）
    gravity: f32,
}

impl Grenade {
    /// 创建手榴弹：`origin` 为出手位置，`dir` 为投掷方向（自动归一化），
    /// `speed` 为初速，`fuse` 为引信时长（调用方在 1.5~2.5s 区间取值，本模块原样使用）
    pub fn new(origin: [f32; 3], dir: [f32; 3], speed: f32, fuse: f32) -> Self {
        let length = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        let inv = if length > f32::EPSILON {
            1.0 / length
        } else {
            0.0
        };
        let fuse = fuse.max(0.0);
        Self {
            pos: origin,
            vel: [dir[0] * inv * speed, dir[1] * inv * speed, dir[2] * inv * speed],
            fuse,
            fuse_max: fuse,
            gravity: GRENADE_GRAVITY,
        }
    }

    /// 按时间步长推进：水平匀速、垂直受重力加速（抛物线），引信同步倒数
    pub fn update(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        self.pos[0] += self.vel[0] * dt;
        self.pos[1] += self.vel[1] * dt;
        self.pos[2] += self.vel[2] * dt;
        self.vel[1] -= self.gravity * dt;
        self.fuse -= dt;
    }

    /// 引信是否已到期（到期即应触发爆炸）
    pub fn exploded(&self) -> bool {
        self.fuse <= 0.0
    }

    /// 当前位置
    pub fn position(&self) -> [f32; 3] {
        self.pos
    }

    /// 初始引信时长（秒，供爆炸结算/日志参考）
    pub fn fuse_max(&self) -> f32 {
        self.fuse_max
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn melee_weapon_trait_behavior() {
        let knife = MeleeWeapon::new("匕首", 25.0, 1.5, 2.0, 60.0_f32.to_radians());

        assert_eq!(knife.name(), "匕首");
        assert_eq!(knife.damage(), 25.0);
        assert_eq!(knife.fire_rate(), 1.5);
        assert_eq!(knife.range(), 2.0);

        // 派生行为：开火间隔与 DPS
        assert!((knife.fire_interval() - (1.0 / 1.5)).abs() < 1e-6);
        assert!((knife.dps() - 37.5).abs() < 1e-6);

        // 射程判定
        assert!(knife.in_range(1.9));
        assert!(!knife.in_range(2.1));
        assert!((knife.arc() - 60.0_f32.to_radians()).abs() < 1e-6);
    }

    #[test]
    fn projectile_weapon_trait_behavior() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 200.0, 300.0, 2.0);

        assert_eq!(rifle.name(), "步枪");
        assert_eq!(rifle.damage(), 50.0);
        assert_eq!(rifle.fire_rate(), 2.0);
        assert_eq!(rifle.range(), 200.0);
        assert_eq!(rifle.projectile_speed(), 300.0);
        assert_eq!(rifle.projectile_lifetime(), 2.0);

        assert!((rifle.fire_interval() - 0.5).abs() < 1e-6);
        assert!((rifle.dps() - 100.0).abs() < 1e-6);
        assert!(rifle.in_range(199.0));
        assert!(!rifle.in_range(201.0));
    }

    #[test]
    fn projectile_update_advances_in_straight_line() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 1000.0, 300.0, 5.0);
        let mut proj = rifle.fire([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);

        assert!(proj.is_alive());
        proj.update(0.5);
        assert!(proj.is_alive());

        // 0.5 秒 × 300 米/秒 = 150 米，沿 x 轴直线推进
        assert!((proj.position[0] - 150.0).abs() < 1e-3);
        assert_eq!(proj.position[1], 1.0);
        assert_eq!(proj.position[2], 0.0);
        assert!((proj.distance_traveled() - 150.0).abs() < 1e-3);
        assert!((proj.age() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn projectile_update_normalizes_direction() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 1000.0, 10.0, 5.0);
        // 未归一化的方向应自动归一化：|(2,0,0)| = 2 → 实际速度仍为 10 米/秒
        let mut proj = rifle.fire([0.0; 3], [2.0, 0.0, 0.0]);

        proj.update(0.5);
        assert!((proj.position[0] - 5.0).abs() < 1e-4);
    }

    #[test]
    fn projectile_destroyed_when_range_exceeded() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 10.0, 300.0, 10.0);
        let mut proj = rifle.fire([0.0; 3], [1.0, 0.0, 0.0]);

        proj.update(0.02); // 6 米，仍在射程内
        assert!(proj.is_alive());

        proj.update(0.02); // 累计 12 米，超过射程 10 米 → 销毁
        assert!(!proj.is_alive());

        // 销毁后不再推进
        let x = proj.position[0];
        proj.update(1.0);
        assert_eq!(proj.position[0], x);
        assert!(!proj.is_alive());
    }

    #[test]
    fn projectile_destroyed_when_lifetime_exceeded() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 1000.0, 100.0, 1.0);
        let mut proj = rifle.fire([0.0; 3], [0.0, 0.0, 1.0]);

        proj.update(0.6); // 存活 0.6 秒
        assert!(proj.is_alive());

        proj.update(0.5); // 存活 1.1 秒，超过寿命 1.0 秒 → 销毁
        assert!(!proj.is_alive());
    }

    #[test]
    fn firearm_full_magazine_fires_and_consumes() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 200.0, 300.0, 2.0);
        let mut gun = Firearm::new(rifle, 3, 12, 1.5, 0.014, 0.004);

        assert!(gun.can_fire());
        assert!((gun.ammo_ratio() - 1.0).abs() < 1e-6);

        let proj = gun.try_fire([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        assert!(proj.is_some());
        // 满弹开火扣 1 发：3 → 2
        assert!((gun.ammo_ratio() - (2.0 / 3.0)).abs() < 1e-6);
        assert!(gun.can_fire());
    }

    #[test]
    fn firearm_empty_magazine_auto_reloads_and_cannot_fire() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 200.0, 300.0, 2.0);
        let mut gun = Firearm::new(rifle, 3, 12, 1.5, 0.014, 0.004);

        // 打空弹匣：3 发全部返回投射物
        for _ in 0..3 {
            assert!(gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]).is_some());
        }
        assert_eq!(gun.ammo_ratio(), 0.0);
        assert!(!gun.can_fire());
        assert!((gun.reload_progress() - 1.0).abs() < 1e-6); // 尚未开始换弹
        gun.update(0.5);
        assert!((gun.reload_progress() - 1.0).abs() < 1e-6); // 仅打空弹匣不会自动换弹

        // 空弹匣再扣扳机：自动开始换弹并返回 None
        assert!(gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]).is_none());
        assert!(!gun.can_fire());
        gun.update(0.5); // 换弹进度开始推进 → 证明已进入换弹
        assert!(gun.reload_progress() < 1.0);
    }

    #[test]
    fn firearm_reload_advances_and_refills_from_reserve() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 200.0, 300.0, 2.0);
        let mut gun = Firearm::new(rifle, 3, 12, 1.5, 0.014, 0.004);

        for _ in 0..3 {
            let _ = gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]);
        }
        let _ = gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]); // 触发自动换弹

        // 换弹中途：进度减半，仍不可开火
        gun.update(0.75);
        assert!((gun.reload_progress() - 0.5).abs() < 1e-6);
        assert!(!gun.can_fire());
        // 换弹期间扣扳机：返回 None 且不重置/打断换弹
        assert!(gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]).is_none());
        assert!((gun.reload_progress() - 0.5).abs() < 1e-6);

        // 完成换弹：弹匣补满 3 发（reserve 12 → 9）
        gun.update(0.75);
        assert!(gun.can_fire());
        assert!((gun.reload_progress() - 1.0).abs() < 1e-6);
        assert!((gun.ammo_ratio() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn firearm_reload_refills_only_remaining_reserve() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 200.0, 300.0, 2.0);
        let mut gun = Firearm::new(rifle, 3, 2, 1.0, 0.014, 0.004);

        for _ in 0..3 {
            let _ = gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]);
        }
        let _ = gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]); // 自动换弹
        gun.update(1.0);

        // reserve 只有 2 发：只补 2 发，弹匣不再满
        assert!((gun.ammo_ratio() - (2.0 / 3.0)).abs() < 1e-6);
        assert!(gun.can_fire());

        // 打光后 reserve 为 0：扣扳机不会进入换弹，也无法开火
        for _ in 0..2 {
            let _ = gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]);
        }
        assert!(!gun.can_fire());
        assert!(gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]).is_none());
        assert!((gun.reload_progress() - 1.0).abs() < 1e-6); // 无备弹，未进入换弹
    }

    #[test]
    fn firearm_fire_interval_forwards_to_weapon() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 200.0, 300.0, 2.0);
        let expected_interval = rifle.fire_interval();
        let mut gun = Firearm::new(rifle, 5, 20, 1.5, 0.014, 0.004);

        // 射速冷却语义由 ProjectileWeapon 推导，Firearm 仅转发
        assert!((gun.fire_interval() - 0.5).abs() < 1e-6);
        assert!((gun.fire_interval() - expected_interval).abs() < 1e-6);

        // Firearm 不重复实现冷却：连续扣扳机均能正常发射直至打空
        let mut shots = 0;
        while gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]).is_some() {
            shots += 1;
        }
        assert_eq!(shots, 5);
    }

    #[test]
    fn firearm_current_kick_deterministic_and_bounded() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 200.0, 300.0, 2.0);
        let mut gun = Firearm::new(rifle, 10, 30, 1.5, 0.014, 0.004);

        // 未开火：微扰为 0，返回基础后坐力
        let (pitch0, yaw0) = gun.current_kick();
        assert!((pitch0 - 0.014).abs() < 1e-6);
        assert_eq!(yaw0, 0.004);

        // 每次开火 shots_fired+1，微扰按 (shots_fired % 5) * 0.001 递增
        let mut last = (0.0, 0.0);
        for i in 1..=5 {
            let _ = gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]);
            let (pitch, yaw) = gun.current_kick();
            let expected_pitch = 0.014 + ((i % 5) as f32) * 0.001;
            assert!((pitch - expected_pitch).abs() < 1e-6);
            assert_eq!(yaw, 0.004);
            last = (pitch, yaw);
        }

        // 确定性：同一次射击多次读取结果一致；微扰范围 0.0 ~ 0.004
        let (pitch, yaw) = gun.current_kick();
        assert_eq!((pitch, yaw), last);
        assert!((0.014..=0.018).contains(&pitch));
    }

    #[test]
    fn firearm_ammo_ratio_and_reload_progress_bounds() {
        let rifle = ProjectileWeapon::new("步枪", 50.0, 2.0, 200.0, 300.0, 2.0);
        let mut gun = Firearm::new(rifle, 3, 9, 1.0, 0.014, 0.004);

        // 满弹夹且未换弹：两个比值均为 1.0；满弹夹时手动换弹无效
        assert!((gun.ammo_ratio() - 1.0).abs() < 1e-6);
        assert!((gun.reload_progress() - 1.0).abs() < 1e-6);
        gun.start_reload();
        assert!((gun.reload_progress() - 1.0).abs() < 1e-6);

        // 打空并触发换弹：弹夹比 0.0，换弹进度从 1.0 开始
        for _ in 0..3 {
            let _ = gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]);
        }
        assert_eq!(gun.ammo_ratio(), 0.0);
        let _ = gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]);
        assert!((gun.reload_progress() - 1.0).abs() < 1e-6);

        // 进度过半且被 clamp 在 (0, 1)；超时一次性完成
        gun.update(0.3);
        let mid = gun.reload_progress();
        assert!(mid > 0.0 && mid < 1.0);
        gun.update(10.0);
        assert!((gun.reload_progress() - 1.0).abs() < 1e-6);
        assert!((gun.ammo_ratio() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn thompson_smg_params_locked() {
        let smg = thompson_smg();
        assert_eq!(smg.name(), "Thompson SMG");
        assert_eq!(smg.damage(), 12.0);
        assert_eq!(smg.fire_rate(), 10.0);
        assert_eq!(smg.range(), 140.0);
        assert_eq!(smg.projectile_speed(), 120.0);
        assert_eq!(smg.projectile_lifetime(), 1.2);

        // 低伤高射速差异化锁定：射速 ≥ 8/s、伤害 10~15、中距离射程
        assert!(smg.fire_rate() >= 8.0);
        assert!((10.0..=15.0).contains(&smg.damage()));
        assert!((50.0..=300.0).contains(&smg.range()));

        // 有效射程内投射物可达：120 m/s × 1.2 s = 144 m ≥ 射程 140 m
        assert!(smg.projectile_speed() * smg.projectile_lifetime() >= smg.range());
    }

    #[test]
    fn thompson_smg_firearm_magazine_reserve_and_recoil() {
        let mut gun = thompson_smg_firearm();
        assert_eq!(gun.max_magazine(), 30);
        assert_eq!(gun.magazine(), 30);
        assert_eq!(gun.reserve(), 120);
        assert!(gun.can_fire());

        // 后坐力略大于 M1 基线（kick_pitch 0.014 / kick_yaw 0.004）
        let (pitch, yaw) = gun.current_kick();
        assert!(pitch > 0.014);
        assert!(yaw > 0.004);

        // 弹匣 30 发全部可发射
        let mut shots = 0;
        while gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]).is_some() {
            shots += 1;
        }
        assert_eq!(shots, 30);
        assert_eq!(gun.magazine(), 0);
    }

    /// 死亡补给：reset_ammo 弹匣补满 + 备弹恢复初始 + 取消换弹
    #[test]
    fn reset_ammo_restores_magazine_and_reserve() {
        let mut gun = thompson_smg_firearm();
        // 打光弹匣 30 发 → 自动换弹（备弹 120 → 90 补满弹匣）
        while gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]).is_some() {}
        gun.update(5.0);
        assert_eq!(gun.magazine(), 30);
        assert_eq!(gun.reserve(), 90);
        // 再打 5 发
        for _ in 0..5 {
            gun.try_fire([0.0; 3], [1.0, 0.0, 0.0]);
        }
        assert_eq!(gun.magazine(), 25);
        assert_eq!(gun.reserve(), 90);
        // 死亡补给：弹匣满 + 备弹恢复 120
        gun.reset_ammo();
        assert_eq!(gun.magazine(), 30);
        assert_eq!(gun.reserve(), 120);
        assert!(!gun.is_reloading());
        assert!(gun.can_fire());
    }

    /// 武器架全量补给：每个槽位弹匣/备弹一并恢复
    #[test]
    fn rack_reset_all_ammo_refills_every_slot() {
        let mut rack = sample_rack();
        // 0 号槽打 5 发
        for _ in 0..5 {
            rack.active_firearm().try_fire([0.0; 3], [1.0, 0.0, 0.0]);
        }
        assert_eq!(rack.active_firearm_ref().magazine(), 3);
        // 切到 1 号槽（Thompson 30 发）打 2 发
        rack.switch_to(1);
        for _ in 0..2 {
            rack.active_firearm().try_fire([0.0; 3], [1.0, 0.0, 0.0]);
        }
        assert_eq!(rack.active_firearm_ref().magazine(), 28);
        // 全量补给：两个槽位都恢复满弹匣 + 初始备弹
        rack.reset_all_ammo();
        assert_eq!(rack.active_firearm_ref().magazine(), 30);
        assert_eq!(rack.active_firearm_ref().reserve(), 120);
        rack.switch_to(0);
        assert_eq!(rack.active_firearm_ref().magazine(), 8);
        assert_eq!(rack.active_firearm_ref().reserve(), 40);
    }

    fn sample_rack() -> WeaponRack {
        WeaponRack::new(
            vec![
                (
                    "M1 Rifle".to_string(),
                    Firearm::new(
                        ProjectileWeapon::new("M1", 25.0, 3.0, 300.0, 300.0, 2.0),
                        8,
                        40,
                        1.5,
                        0.014,
                        0.004,
                    ),
                ),
                ("Thompson SMG".to_string(), thompson_smg_firearm()),
            ],
            0.4,
        )
    }

    #[test]
    fn weapon_rack_new_starts_at_zero() {
        let rack = sample_rack();
        assert_eq!(rack.len(), 2);
        assert_eq!(rack.active_name(), "M1 Rifle");
        assert!(!rack.is_switching());
        assert_eq!(rack.active_firearm_ref().max_magazine(), 8);
    }

    #[test]
    fn weapon_rack_switch_to_valid_and_ignores_invalid() {
        let mut rack = sample_rack();

        // 有效切换：激活槽改变、进入切换状态
        rack.switch_to(1);
        assert_eq!(rack.active_name(), "Thompson SMG");
        assert!(rack.is_switching());
        assert_eq!(rack.active_firearm_ref().max_magazine(), 30);

        // 无效索引：忽略，仍指向当前槽
        rack.switch_to(99);
        assert_eq!(rack.active_name(), "Thompson SMG");

        // 同索引切换：忽略且不重置计时
        rack.update(0.2);
        assert!(rack.is_switching());
        rack.switch_to(1);
        rack.update(0.2);
        assert!(!rack.is_switching());
    }

    #[test]
    fn weapon_rack_switch_cycles_next_and_prev() {
        let mut rack = sample_rack();
        rack.switch_next(); // 0 → 1
        assert_eq!(rack.active_name(), "Thompson SMG");
        rack.switch_next(); // 1 → 0（循环）
        assert_eq!(rack.active_name(), "M1 Rifle");
        rack.switch_prev(); // 0 → 1（循环回末尾）
        assert_eq!(rack.active_name(), "Thompson SMG");
        rack.switch_prev(); // 1 → 0
        assert_eq!(rack.active_name(), "M1 Rifle");
    }

    #[test]
    fn weapon_rack_update_timer_and_switch_finishes() {
        let mut rack = WeaponRack::new(
            vec![
                ("A".to_string(), thompson_smg_firearm()),
                (
                    "B".to_string(),
                    Firearm::new(
                        ProjectileWeapon::new("B", 25.0, 3.0, 300.0, 300.0, 2.0),
                        8,
                        40,
                        1.5,
                        0.014,
                        0.004,
                    ),
                ),
                ("C".to_string(), thompson_smg_firearm()),
            ],
            1.0,
        );
        rack.switch_to(2);
        assert_eq!(rack.active_name(), "C");
        assert!(rack.is_switching());

        // 切换中途：计时递减、active 保持指向目标槽
        rack.update(0.25);
        assert!(rack.is_switching());
        assert_eq!(rack.active_name(), "C");

        // 计时耗尽：切换完成
        rack.update(0.75);
        assert!(!rack.is_switching());
        assert_eq!(rack.active_name(), "C");

        // dt<=0 不推进计时
        rack.switch_to(1);
        rack.update(0.0);
        assert!(rack.is_switching());
        rack.update(-1.0);
        assert!(rack.is_switching());
    }

    #[test]
    fn weapon_rack_empty_is_safe_for_switch() {
        let mut rack = WeaponRack::new(vec![], 0.5);
        assert_eq!(rack.len(), 0);
        assert_eq!(rack.active_name(), "");
        rack.switch_to(0);
        rack.switch_next();
        rack.switch_prev();
        rack.update(0.5);
        assert!(!rack.is_switching());
    }

    #[test]
    fn weapon_rack_active_firearm_mutates_active_slot() {
        let mut rack = sample_rack();
        // 可变引用作用于当前槽：打一发，弹匣 8 → 7
        assert!(rack
            .active_firearm()
            .try_fire([0.0; 3], [1.0, 0.0, 0.0])
            .is_some());
        assert_eq!(rack.active_firearm_ref().magazine(), 7);

        // 切到 Thompson 后，可变引用操作的是另一把枪（弹匣仍满 30）
        rack.switch_to(1);
        assert_eq!(rack.active_firearm_ref().magazine(), 30);
        rack.active_firearm().start_reload(); // 满弹匣换弹无效
        assert!(!rack.active_firearm_ref().is_reloading());
    }

    #[test]
    fn grenade_constants_locked() {
        assert_eq!(GRENADE_GRAVITY, 9.8);
        assert_eq!(GRENADE_FUSE_MIN, 1.5);
        assert_eq!(GRENADE_FUSE_MAX, 2.5);
        assert!(GRENADE_SPEED > 0.0);
        assert!(GRENADE_FUSE_MIN < GRENADE_FUSE_MAX);
    }

    #[test]
    fn grenade_parabola_horizontal_and_vertical() {
        // 水平抛出：水平匀速（x = vx*t），垂直按 0.5*g*t² 下落（小步长离散误差 < 0.01）
        let mut g = Grenade::new([0.0, 5.0, 0.0], [1.0, 0.0, 0.0], GRENADE_SPEED, 2.0);
        let dt = 0.001_f32;
        let steps = 500; // 总时长 0.5s
        for _ in 0..steps {
            g.update(dt);
        }
        let pos = g.position();
        let t = dt * steps as f32;
        assert!((pos[0] - GRENADE_SPEED * t).abs() < 1e-3); // 水平匀速
        assert_eq!(pos[2], 0.0);
        let drop = 0.5 * GRENADE_GRAVITY * t * t;
        assert!((pos[1] - (5.0 - drop)).abs() < 0.01); // 垂直符合 0.5gt²
        assert!(pos[1] < 5.0); // 确实在下落

        // 上抛：垂直位移 = vy*t - 0.5*g*t²
        let mut up = Grenade::new([0.0; 3], [0.0, 1.0, 0.0], 20.0, 2.0);
        for _ in 0..steps {
            up.update(dt);
        }
        let expected_y = 20.0 * t - 0.5 * GRENADE_GRAVITY * t * t;
        assert!((up.position()[1] - expected_y).abs() < 0.01);
    }

    #[test]
    fn grenade_fuse_countdown_and_expiry() {
        let mut g = Grenade::new([0.0; 3], [1.0, 0.0, 0.0], 10.0, 1.5);
        assert_eq!(g.fuse_max, 1.5);
        assert!(!g.exploded());

        g.update(0.5);
        assert!(!g.exploded()); // 引信未到期
        assert!((g.fuse - 1.0).abs() < 1e-6); // 剩余 1.0s

        g.update(1.0);
        assert!(g.exploded()); // 1.5s 到期

        // 到期后继续 update：exploded 保持 true（爆炸由游戏侧处理）
        g.update(0.1);
        assert!(g.exploded());
    }

    #[test]
    fn grenade_initial_velocity_uses_normalized_dir_times_speed() {
        // 未归一化方向 (2,0,0)：归一化后初速应为 (speed, 0, 0)
        let g = Grenade::new([1.0, 2.0, 3.0], [2.0, 0.0, 0.0], 18.0, 2.0);
        assert_eq!(g.vel, [18.0, 0.0, 0.0]);
        assert_eq!(g.position(), [1.0, 2.0, 3.0]); // 位置记录起点

        // 斜向 45°：水平/垂直分量 = speed/√2
        let diag = Grenade::new([0.0; 3], [1.0, 1.0, 0.0], 20.0, 2.0);
        let c = 20.0 / 2.0_f32.sqrt();
        assert!((diag.vel[0] - c).abs() < 1e-4);
        assert!((diag.vel[1] - c).abs() < 1e-4);
        assert_eq!(diag.vel[2], 0.0);
    }

    #[test]
    fn grenade_uses_passed_fuse_value() {
        // 引信由调用方在 GRENADE_FUSE_MIN/MAX 区间取值，本模块原样使用
        let g = Grenade::new([0.0; 3], [0.0, 0.0, 1.0], 10.0, GRENADE_FUSE_MAX);
        assert_eq!(g.fuse, GRENADE_FUSE_MAX);
        assert_eq!(g.fuse_max, GRENADE_FUSE_MAX);
        assert!(!g.exploded());
    }
}
