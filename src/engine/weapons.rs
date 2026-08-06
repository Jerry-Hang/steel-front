//! 武器系统模块
//!
//! - `Weapon` 特征：所有武器的通用属性（名称、伤害、射速、射程）与派生行为（开火间隔、射程判定、DPS）
//! - `MeleeWeapon`：近战武器，基于射程与挥击夹角判定命中，不产生投射物
//! - `ProjectileWeapon`：投射物武器，发射沿直线运动的 `Projectile`
//! - `Projectile`：投射物，直线运动 + 生命周期（飞行距离超过射程或存活超过寿命即销毁）
//!
//! 当前仅使用标准库，无第三方依赖；如将来需要新增依赖，在文件头部按
//! `// DEP: crate = version` 格式声明。

/// 武器特征：近战与投射物武器的公共接口
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
        }
    }

    /// 投射物飞行速度（米/秒）
    pub fn projectile_speed(&self) -> f32 {
        self.projectile_speed
    }

    /// 投射物最长存活时间（秒）
    pub fn projectile_lifetime(&self) -> f32 {
        self.projectile_lifetime
    }

    /// 发射一枚投射物：`origin` 为出膛位置，`direction` 为发射方向（自动归一化）
    pub fn fire(&self, origin: [f32; 3], direction: [f32; 3]) -> Projectile {
        Projectile::new(
            origin,
            direction,
            self.projectile_speed,
            self.range,
            self.projectile_lifetime,
            self.damage,
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
    /// 命中伤害
    pub damage: f32,
    /// 是否存活（射程/寿命耗尽后为 false）
    alive: bool,
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
            velocity: [dir[0] * speed, dir[1] * speed, dir[2] * speed],
            distance_traveled: 0.0,
            age: 0.0,
            max_lifetime,
            range,
            damage,
            alive: true,
        }
    }

    /// 是否仍存活（射程/寿命耗尽后返回 false）
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// 已飞行距离（米）
    pub fn distance_traveled(&self) -> f32 {
        self.distance_traveled
    }

    /// 已存活时间（秒）
    pub fn age(&self) -> f32 {
        self.age
    }

    /// 按时间步长推进：直线运动；飞行距离超过射程或存活超过寿命即销毁
    pub fn update(&mut self, dt: f32) {
        if !self.alive || dt <= 0.0 {
            return;
        }
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
}
