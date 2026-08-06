//! 游戏运行时中枢
//!
//! 把 weapons / ai / physics / ui / audio / network 模块接进主循环：
//! - 每帧 `update(dt, camera, fire)` 推进物理、武器、AI、音频、网络
//! - 渲染前由 main.rs 取 HUD quad 列表与光照 uniform
//!
//! 本文件只做模块间编排与少量胶水逻辑，具体算法仍留在各模块内。

use std::sync::{Arc, Mutex};

use super::camera::Camera;
use super::physics::{self, Body, CollisionEvent, CollisionListener, SphereBody, Vec3 as Pv};
use super::weapons::{Projectile, ProjectileWeapon, Weapon};

/// 碰撞事件缓冲：监听者写入，Game 每帧 drain 取走
struct EventBuffer(Arc<Mutex<Vec<CollisionEvent>>>);

impl CollisionListener for EventBuffer {
    fn on_collision(&mut self, event: &CollisionEvent) {
        if let Ok(mut buf) = self.0.lock() {
            buf.push(*event);
        }
    }
}

/// 游戏运行时状态（随接线进度逐步扩展）
pub struct Game {
    /// 物理世界：重力积分、地面响应、刚体间碰撞
    pub world: physics::World,
    /// 本帧产生的碰撞事件（drain 后清空）
    collisions: Vec<CollisionEvent>,
    /// 碰撞事件累计数（供 UI/日志）
    total_collisions: u64,
    /// 累计运行时间（秒）
    pub time: f32,
    /// 最近一帧 dt
    pub last_dt: f32,
    /// 碰撞事件缓冲（与监听者共享）
    event_buf: Arc<Mutex<Vec<CollisionEvent>>>,
    /// 上次碰撞日志时间（限频用）
    last_event_log_time: f32,
    /// 主武器（投射物步枪）
    rifle: ProjectileWeapon,
    /// 在场投射物
    projectiles: Vec<Projectile>,
    /// 开火冷却剩余时间（秒）
    fire_cooldown: f32,
    /// 发射次数（累计）
    shots: u64,
    /// 命中次数（累计，供 UI/日志）
    hits: u64,
}

impl Game {
    /// 创建游戏中枢：初始化物理演示场景
    pub fn new() -> Self {
        let mut world = physics::World::new();
        world.gravity = 9.8;
        // 地形中央 60×60 区域已压平到 y=0，演示刚体放在该区域
        world.ground_y = 0.0;
        for (x, z) in [(-12.0, -8.0), (0.0, -8.0), (12.0, -8.0)] {
            world
                .bodies
                .push(Body::new(Pv::new(x, 3.0, z), Pv::new(1.2, 1.2, 1.2)));
        }
        for (x, z) in [(-6.0, 8.0), (6.0, 8.0)] {
            let mut sphere = SphereBody::new(Pv::new(x, 4.0, z), 1.0);
            sphere.restitution = 0.5;
            world.spheres.push(sphere);
        }
        let event_buf = Arc::new(Mutex::new(Vec::new()));
        world.add_listener(Box::new(EventBuffer(event_buf.clone())));
        log::info!(
            "physics: {} AABB bodies, {} spheres, gravity={}, ground_y={}",
            world.bodies.len(),
            world.spheres.len(),
            world.gravity,
            world.ground_y
        );
        Self {
            world,
            collisions: Vec::new(),
            total_collisions: 0,
            time: 0.0,
            last_dt: 0.0,
            event_buf,
            last_event_log_time: 0.0,
            rifle: ProjectileWeapon::new("M1 Rifle", 25.0, 3.0, 200.0, 60.0, 5.0),
            projectiles: Vec::new(),
            fire_cooldown: 0.0,
            shots: 0,
            hits: 0,
        }
    }

    /// 每帧推进所有已接入系统
    pub fn update(&mut self, dt: f32, _camera: &Camera) {
        self.last_dt = dt;
        self.time += dt;
        self.fire_cooldown = (self.fire_cooldown - dt).max(0.0);
        self.world.step(dt);
        self.drain_collisions();
        self.update_projectiles(dt);
    }

    /// 尝试开火（受射速冷却限制）。`origin`/`direction` 来自相机；返回是否真的开火。
    pub fn fire(&mut self, origin: [f32; 3], direction: [f32; 3]) -> bool {
        if self.fire_cooldown > 0.0 {
            return false;
        }
        self.fire_cooldown = self.rifle.fire_interval();
        self.projectiles.push(self.rifle.fire(origin, direction));
        self.shots += 1;
        log::info!(
            "weapons: shot #{} ({} alive)",
            self.shots,
            self.projectiles.len()
        );
        true
    }

    /// 累计命中数（供 UI / 日志）
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// 投射物推进 + 与物理刚体碰撞检测（命中即销毁）
    fn update_projectiles(&mut self, dt: f32) {
        for p in self.projectiles.iter_mut() {
            if p.is_alive() {
                p.update(dt);
            }
        }
        let mut hit_count = 0u32;
        let old = std::mem::take(&mut self.projectiles);
        let mut alive = Vec::with_capacity(old.len());
        for p in old {
            if !p.is_alive() {
                continue;
            }
            if self.collide_projectile(&p) {
                hit_count += 1;
            } else {
                alive.push(p);
            }
        }
        self.projectiles = alive;
        self.hits += hit_count as u64;
        if hit_count > 0 {
            log::info!(
                "weapons: {} projectile hit(s), total_hits={}",
                hit_count,
                self.hits
            );
        }
    }

    /// 投射物点是否落在某个 AABB 刚体内或球体内
    fn collide_projectile(&self, p: &Projectile) -> bool {
        let (px, py, pz) = (p.position[0], p.position[1], p.position[2]);
        for body in &self.world.bodies {
            let aabb = body.aabb();
            if px >= aabb.min.x
                && px <= aabb.max.x
                && py >= aabb.min.y
                && py <= aabb.max.y
                && pz >= aabb.min.z
                && pz <= aabb.max.z
            {
                return true;
            }
        }
        for s in &self.world.spheres {
            let dx = px - s.center.x;
            let dy = py - s.center.y;
            let dz = pz - s.center.z;
            if dx * dx + dy * dy + dz * dz <= s.radius * s.radius {
                return true;
            }
        }
        false
    }

    /// 累计碰撞事件数（供 UI / 日志）
    pub fn total_collisions(&self) -> u64 {
        self.total_collisions
    }

    /// 取走本帧碰撞事件并累计计数（限频打一条日志）
    fn drain_collisions(&mut self) {
        if let Ok(mut buf) = self.event_buf.lock() {
            self.collisions = std::mem::take(&mut *buf);
        }
        if self.collisions.is_empty() {
            return;
        }
        self.total_collisions += self.collisions.len() as u64;
        if self.time - self.last_event_log_time >= 0.5 {
            self.last_event_log_time = self.time;
            let kinds: Vec<String> = self
                .collisions
                .iter()
                .map(|e| format!("{:?}", e.kind))
                .collect();
            log::info!("physics: {} events this frame ({})", self.collisions.len(), kinds.join(", "));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 演示刚体应在地面附近落地并静止
    #[test]
    fn physics_demo_bodies_settle() {
        let mut game = Game::new();
        assert_eq!(game.world.bodies.len(), 3);
        assert_eq!(game.world.spheres.len(), 2);
        for _ in 0..120 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        for body in &game.world.bodies {
            assert!(body.grounded, "body should be grounded");
            let bottom = body.position.y - body.half_extents.y;
            assert!((bottom - game.world.ground_y).abs() < 0.01, "body should rest on ground");
        }
    }

    /// 开火产生投射物，命中物理刚体后销毁并计数
    #[test]
    fn weapon_fire_hits_physics_body() {
        let mut game = Game::new();
        for _ in 0..120 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        let body = game.world.bodies[0];
        let target = [body.position.x, body.position.y, body.position.z];
        // 从目标正上方竖直向下射击
        assert!(game.fire([target[0], target[1] + 50.0, target[2]], [0.0, -1.0, 0.0]));
        assert_eq!(game.shots, 1);
        for _ in 0..200 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(game.hits() >= 1, "projectile should hit the body");
        assert!(game.projectiles.is_empty(), "hit projectile should be removed");
    }

    /// 射速冷却：连续开火被限流
    #[test]
    fn weapon_fire_rate_limits_shots() {
        let mut game = Game::new();
        let origin = [0.0, 5.0, 0.0];
        let dir = [0.0, 0.0, -1.0];
        assert!(game.fire(origin, dir));
        // 冷却期内再开火应被拒绝
        for _ in 0..10 {
            assert!(!game.fire(origin, dir));
            game.update(1.0 / 240.0, &Camera::new());
        }
        assert_eq!(game.shots, 1);
    }
}
