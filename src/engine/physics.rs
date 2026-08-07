//! 物理碰撞系统模块
//!
//! - AABB 碰撞检测：重叠判断（`Aabb::overlaps`）+ 相交解析（`Aabb::resolve`，沿最小穿透轴推出）
//! - 球体碰撞检测：`Sphere::intersects` / `Sphere::resolve`
//! - 简单重力 + 地面碰撞响应：`World::step` 逐帧积分，落体撞地后按恢复系数静止/反弹
//! - `CollisionEvent` 回调机制：`World::add_listener` 注册 `CollisionListener`，碰撞发生时通知监听者
//!
//! 本模块仅使用 `std`，不引入外部依赖；如将来需要新依赖，在文件头部按 `// DEP: crate = version` 声明。
//! 尚未接入 main.rs 主循环，整体允许 dead_code 警告。

#![allow(dead_code)]

/// 三维向量（std 轻量实现，避免引入线性代数依赖）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn length_sq(self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    pub fn normalized(self) -> Self {
        let len = self.length_sq().sqrt();
        if len > 1e-12 {
            Self::new(self.x / len, self.y / len, self.z / len)
        } else {
            Self::ZERO
        }
    }
}

impl std::ops::Add for Vec3 {
    type Output = Vec3;

    fn add(self, rhs: Vec3) -> Vec3 {
        Vec3::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl std::ops::AddAssign for Vec3 {
    fn add_assign(&mut self, rhs: Vec3) {
        self.x += rhs.x;
        self.y += rhs.y;
        self.z += rhs.z;
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Vec3;

    fn sub(self, rhs: Vec3) -> Vec3 {
        Vec3::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl std::ops::SubAssign for Vec3 {
    fn sub_assign(&mut self, rhs: Vec3) {
        self.x -= rhs.x;
        self.y -= rhs.y;
        self.z -= rhs.z;
    }
}

impl std::ops::Mul<f32> for Vec3 {
    type Output = Vec3;

    fn mul(self, rhs: f32) -> Vec3 {
        Vec3::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

/// 轴对齐包围盒（AABB）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// 是否与 `other` 重叠（边缘相贴不算重叠）
    pub fn overlaps(&self, other: &Aabb) -> bool {
        self.min.x < other.max.x
            && self.max.x > other.min.x
            && self.min.y < other.max.y
            && self.max.y > other.min.y
            && self.min.z < other.max.z
            && self.max.z > other.min.z
    }

    /// 相交解析：沿最小穿透轴把 self 推出 other，返回碰撞事件（含法向与穿透深度）。
    /// 不相交时返回 None。
    pub fn resolve(&mut self, other: &Aabb) -> Option<CollisionEvent> {
        let (normal, penetration) = aabb_separation(self, other)?;
        self.min += normal * penetration;
        self.max += normal * penetration;
        Some(CollisionEvent::new(
            CollisionKind::AabbResolved,
            None,
            None,
            normal,
            penetration,
        ))
    }
}

/// 计算把 `a` 与 `b` 分离所需的最小穿透轴。
/// 返回 (法向：从 b 指向 a 的单位向量, 穿透深度)；不相交返回 None。
fn aabb_separation(a: &Aabb, b: &Aabb) -> Option<(Vec3, f32)> {
    if !a.overlaps(b) {
        return None;
    }
    let pen_x = (a.max.x - b.min.x).min(b.max.x - a.min.x);
    let pen_y = (a.max.y - b.min.y).min(b.max.y - a.min.y);
    let pen_z = (a.max.z - b.min.z).min(b.max.z - a.min.z);

    let (axis, penetration) = if pen_x <= pen_y && pen_x <= pen_z {
        (0, pen_x)
    } else if pen_y <= pen_z {
        (1, pen_y)
    } else {
        (2, pen_z)
    };

    let normal = match axis {
        0 => Vec3::new(if a.center().x >= b.center().x { 1.0 } else { -1.0 }, 0.0, 0.0),
        1 => Vec3::new(0.0, if a.center().y >= b.center().y { 1.0 } else { -1.0 }, 0.0),
        _ => Vec3::new(0.0, 0.0, if a.center().z >= b.center().z { 1.0 } else { -1.0 }),
    };
    Some((normal, penetration))
}

/// 球体
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sphere {
    pub center: Vec3,
    pub radius: f32,
}

impl Sphere {
    pub fn new(center: Vec3, radius: f32) -> Self {
        Self { center, radius }
    }

    /// 两球是否相交（相切不算相交）
    pub fn intersects(&self, other: &Sphere) -> bool {
        let delta = self.center - other.center;
        let r = self.radius + other.radius;
        delta.length_sq() < r * r
    }

    /// 相交解析：沿中心连线把 self 推出 other（中心重合时沿 +X），返回碰撞事件。
    /// 不相交时返回 None。
    pub fn resolve(&mut self, other: &Sphere) -> Option<CollisionEvent> {
        if !self.intersects(other) {
            return None;
        }
        let delta = self.center - other.center;
        let dist_sq = delta.length_sq();
        let penetration = self.radius + other.radius - dist_sq.sqrt();
        let normal = if dist_sq > 1e-12 {
            delta.normalized()
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        self.center += normal * penetration;
        Some(CollisionEvent::new(
            CollisionKind::SphereResolved,
            None,
            None,
            normal,
            penetration,
        ))
    }
}

/// 碰撞类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionKind {
    /// 两个 AABB 发生重叠（尚未解析）
    AabbOverlap,
    /// AABB 相交解析完成
    AabbResolved,
    /// 两个球体相交（尚未解析）
    SphereIntersect,
    /// 球体相交解析完成
    SphereResolved,
    /// 物体撞击地面
    GroundHit,
}

/// 碰撞事件（碰撞发生时回调给监听者）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollisionEvent {
    pub kind: CollisionKind,
    /// 第一个参与物体索引（独立检测时为 None）
    pub body_a: Option<usize>,
    /// 第二个参与物体索引（地面 / 独立检测时为 None）
    pub body_b: Option<usize>,
    /// 碰撞法向（从 B 指向 A，单位向量；GroundHit 恒为 +Y）
    pub normal: Vec3,
    /// 穿透深度
    pub penetration: f32,
}

impl CollisionEvent {
    pub fn new(
        kind: CollisionKind,
        body_a: Option<usize>,
        body_b: Option<usize>,
        normal: Vec3,
        penetration: f32,
    ) -> Self {
        Self {
            kind,
            body_a,
            body_b,
            normal,
            penetration,
        }
    }
}

/// 碰撞监听者：注册到 `World` 后，每次碰撞都会收到 `CollisionEvent`
pub trait CollisionListener {
    fn on_collision(&mut self, event: &CollisionEvent);
}

/// 参与物理模拟的 AABB 刚体
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    /// 中心位置
    pub position: Vec3,
    /// 半尺寸（AABB = position ± half_extents）
    pub half_extents: Vec3,
    /// 速度（单位/秒）
    pub velocity: Vec3,
    /// 恢复系数（0 = 完全非弹性，撞地即静止；1 = 完全弹性反弹）
    pub restitution: f32,
    /// 是否已静止在地面上
    pub grounded: bool,
}

impl Body {
    pub fn new(position: Vec3, half_extents: Vec3) -> Self {
        Self {
            position,
            half_extents,
            velocity: Vec3::ZERO,
            restitution: 0.0,
            grounded: false,
        }
    }

    pub fn aabb(&self) -> Aabb {
        Aabb::new(
            self.position - self.half_extents,
            self.position + self.half_extents,
        )
    }
}

/// 参与物理模拟的球体刚体
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SphereBody {
    pub center: Vec3,
    pub radius: f32,
    pub velocity: Vec3,
    pub restitution: f32,
    pub grounded: bool,
}

impl SphereBody {
    pub fn new(center: Vec3, radius: f32) -> Self {
        Self {
            center,
            radius,
            velocity: Vec3::ZERO,
            restitution: 0.0,
            grounded: false,
        }
    }

    pub fn sphere(&self) -> Sphere {
        Sphere::new(self.center, self.radius)
    }
}

/// FPS 玩家碰撞体：以脚底位置为圆心、`radius` 为半径的水平圆柱（高 `eye_height`）。
/// 只处理水平（x/z）碰撞，y 由地形高度在游戏侧决定，本实现不改变 `pos.y`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerBody {
    /// 脚底位置（x/z 参与碰撞，y 由地形高度决定）
    pub pos: Vec3,
    /// 水平碰撞半径
    pub radius: f32,
    /// 眼睛高度（身高）
    pub eye_height: f32,
    /// 水平速度（单位/秒）
    pub vel: Vec3,
    /// 是否着地
    pub grounded: bool,
}

impl PlayerBody {
    pub fn new(pos: Vec3, radius: f32, eye_height: f32) -> Self {
        Self {
            pos,
            radius,
            eye_height,
            vel: Vec3::ZERO,
            grounded: false,
        }
    }

    pub fn pos(&self) -> Vec3 {
        self.pos
    }

    pub fn radius(&self) -> f32 {
        self.radius
    }

    pub fn eye_height(&self) -> f32 {
        self.eye_height
    }

    pub fn vel(&self) -> Vec3 {
        self.vel
    }

    pub fn grounded(&self) -> bool {
        self.grounded
    }

    /// 把玩家从单个 AABB 中水平推出（圆-AABB 最近点推挤）。
    /// 圆心 clamp 到 AABB 的 [min,max] 得到最近点，距离 < radius 即重叠；
    /// 推挤方向为 圆心→最近点（水平投影重合时取最小穿透轴，参考 `aabb_separation` 风格）。
    /// 发生推挤时返回穿透深度，否则返回 None。只改 pos.x/z，不碰 pos.y。
    fn push_out_of_aabb(&mut self, aabb: &Aabb) -> Option<f32> {
        let cx = self.pos.x;
        let cz = self.pos.z;
        let closest_x = cx.clamp(aabb.min.x, aabb.max.x);
        let closest_z = cz.clamp(aabb.min.z, aabb.max.z);
        let dx = cx - closest_x;
        let dz = cz - closest_z;
        let dist_sq = dx * dx + dz * dz;
        if dist_sq >= self.radius * self.radius {
            return None; // 相切/分离不算碰撞
        }
        // 推挤时多推 1e-6，避免浮点误差残留导致边界反复判定重叠
        let eps = 1e-6;
        let (nx, nz, penetration) = if dist_sq > 1e-12 {
            // 圆心在 AABB 外：沿 圆心→最近点 方向推出
            let dist = dist_sq.sqrt();
            (dx / dist, dz / dist, self.radius - dist + eps)
        } else {
            // 圆心水平投影在 AABB 内部：取最小穿透轴（X 或 Z），朝较近的面推出
            let pen_x = (cx - aabb.min.x).min(aabb.max.x - cx);
            let pen_z = (cz - aabb.min.z).min(aabb.max.z - cz);
            if pen_x <= pen_z {
                if cx - aabb.min.x <= aabb.max.x - cx {
                    (-1.0, 0.0, pen_x + self.radius + eps)
                } else {
                    (1.0, 0.0, pen_x + self.radius + eps)
                }
            } else if cz - aabb.min.z <= aabb.max.z - cz {
                (0.0, -1.0, pen_z + self.radius + eps)
            } else {
                (0.0, 1.0, pen_z + self.radius + eps)
            }
        };
        self.pos.x += nx * penetration;
        self.pos.z += nz * penetration;
        Some(penetration)
    }

    /// 对 `world.bodies` 中每个 AABB 做水平推挤（内部 1-2 次迭代，
    /// 避免一次推挤把玩家挤进相邻刚体）。发生任何推挤时返回 true。
    pub fn collide_world(&mut self, world: &World) -> bool {
        let mut pushed = false;
        for _ in 0..2 {
            let mut any = false;
            for body in &world.bodies {
                if self.push_out_of_aabb(&body.aabb()).is_some() {
                    any = true;
                }
            }
            if !any {
                break;
            }
            pushed = true;
        }
        pushed
    }

    /// 请求移动 (dx, dz)：先叠加位移，再 `collide_world` 推回，
    /// 返回实际位移 (f32, f32)。y 始终不被改动。
    pub fn try_move(&mut self, world: &World, dx: f32, dz: f32) -> (f32, f32) {
        let old_x = self.pos.x;
        let old_z = self.pos.z;
        self.pos.x += dx;
        self.pos.z += dz;
        self.collide_world(world);
        (self.pos.x - old_x, self.pos.z - old_z)
    }

    /// 是否与任一刚体的 AABB（水平扩展 radius 后）重叠
    pub fn collides(&self, world: &World) -> bool {
        world.bodies.iter().any(|body| {
            let a = body.aabb();
            self.pos.x > a.min.x - self.radius
                && self.pos.x < a.max.x + self.radius
                && self.pos.z > a.min.z - self.radius
                && self.pos.z < a.max.z + self.radius
        })
    }
}

/// 物理世界：重力积分、地面碰撞响应、物体间碰撞检测与事件回调
pub struct World {
    /// 重力加速度（Y 轴向下，正值）
    pub gravity: f32,
    /// 地面高度（Y）
    pub ground_y: f32,
    /// AABB 刚体列表
    pub bodies: Vec<Body>,
    /// 球体刚体列表
    pub spheres: Vec<SphereBody>,
    listeners: Vec<Box<dyn CollisionListener>>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        Self {
            gravity: 9.8,
            ground_y: 0.0,
            bodies: Vec::new(),
            spheres: Vec::new(),
            listeners: Vec::new(),
        }
    }

    /// 注册碰撞监听者（碰撞发生时收到 CollisionEvent）
    pub fn add_listener(&mut self, listener: Box<dyn CollisionListener>) {
        self.listeners.push(listener);
    }

    fn emit(&mut self, event: CollisionEvent) {
        for listener in self.listeners.iter_mut() {
            listener.on_collision(&event);
        }
    }

    /// 推进一帧：重力积分 → 地面碰撞响应 → 物体间碰撞检测与解析
    pub fn step(&mut self, dt: f32) {
        self.step_bodies(dt);
        self.step_spheres(dt);
        self.resolve_body_pairs();
        self.resolve_sphere_pairs();
    }

    /// AABB 刚体：重力积分 + 地面碰撞响应
    fn step_bodies(&mut self, dt: f32) {
        let n = self.bodies.len();
        for i in 0..n {
            let body = &mut self.bodies[i];
            if !body.grounded {
                body.velocity.y -= self.gravity * dt;
            }
            body.position += body.velocity * dt;
        }
        for i in 0..n {
            let bottom = self.bodies[i].position.y - self.bodies[i].half_extents.y;
            if bottom < self.ground_y {
                self.bodies[i].position.y = self.ground_y + self.bodies[i].half_extents.y;
                if self.bodies[i].velocity.y < 0.0 {
                    self.bodies[i].velocity.y *= -self.bodies[i].restitution;
                }
                if !self.bodies[i].grounded {
                    self.emit(CollisionEvent::new(
                        CollisionKind::GroundHit,
                        Some(i),
                        None,
                        Vec3::new(0.0, 1.0, 0.0),
                        self.ground_y - bottom,
                    ));
                }
                // 静止阈值 = 一帧重力冲量：反弹速度低于它无法离地，视为静止
                let rest_eps = (self.gravity * dt).max(1e-4);
                if self.bodies[i].velocity.y.abs() < rest_eps {
                    self.bodies[i].velocity.y = 0.0;
                }
                self.bodies[i].grounded = self.bodies[i].velocity.y.abs() < rest_eps;
            }
        }
    }

    /// 球体刚体：重力积分 + 地面碰撞响应
    fn step_spheres(&mut self, dt: f32) {
        let n = self.spheres.len();
        for i in 0..n {
            let sphere = &mut self.spheres[i];
            if !sphere.grounded {
                sphere.velocity.y -= self.gravity * dt;
            }
            sphere.center += sphere.velocity * dt;
        }
        for i in 0..n {
            let bottom = self.spheres[i].center.y - self.spheres[i].radius;
            if bottom < self.ground_y {
                self.spheres[i].center.y = self.ground_y + self.spheres[i].radius;
                if self.spheres[i].velocity.y < 0.0 {
                    self.spheres[i].velocity.y *= -self.spheres[i].restitution;
                }
                if !self.spheres[i].grounded {
                    self.emit(CollisionEvent::new(
                        CollisionKind::GroundHit,
                        Some(i),
                        None,
                        Vec3::new(0.0, 1.0, 0.0),
                        self.ground_y - bottom,
                    ));
                }
                // 静止阈值 = 一帧重力冲量：反弹速度低于它无法离地，视为静止
                let rest_eps = (self.gravity * dt).max(1e-4);
                if self.spheres[i].velocity.y.abs() < rest_eps {
                    self.spheres[i].velocity.y = 0.0;
                }
                self.spheres[i].grounded = self.spheres[i].velocity.y.abs() < rest_eps;
            }
        }
    }

    /// 物体间 AABB 碰撞：重叠检测 + 各推一半解析（按帧初快照的 AABB 结算）
    fn resolve_body_pairs(&mut self) {
        let n = self.bodies.len();
        if n < 2 {
            return;
        }
        let aabbs: Vec<Aabb> = self.bodies.iter().map(Body::aabb).collect();
        for i in 0..n {
            for j in (i + 1)..n {
                if !aabbs[i].overlaps(&aabbs[j]) {
                    continue;
                }
                self.emit(CollisionEvent::new(
                    CollisionKind::AabbOverlap,
                    Some(i),
                    Some(j),
                    Vec3::ZERO,
                    0.0,
                ));
                if let Some((normal, penetration)) = aabb_separation(&aabbs[i], &aabbs[j]) {
                    let half = penetration * 0.5;
                    self.bodies[i].position += normal * half;
                    self.bodies[j].position -= normal * half;
                    self.emit(CollisionEvent::new(
                        CollisionKind::AabbResolved,
                        Some(i),
                        Some(j),
                        normal,
                        penetration,
                    ));
                }
            }
        }
    }

    /// 物体间球体碰撞：相交检测 + 各推一半解析
    fn resolve_sphere_pairs(&mut self) {
        let n = self.spheres.len();
        if n < 2 {
            return;
        }
        for i in 0..n {
            for j in (i + 1)..n {
                let (a, b) = (self.spheres[i].sphere(), self.spheres[j].sphere());
                if !a.intersects(&b) {
                    continue;
                }
                let delta = a.center - b.center;
                let dist_sq = delta.length_sq();
                let penetration = a.radius + b.radius - dist_sq.sqrt();
                let normal = if dist_sq > 1e-12 {
                    delta.normalized()
                } else {
                    Vec3::new(1.0, 0.0, 0.0)
                };
                self.emit(CollisionEvent::new(
                    CollisionKind::SphereIntersect,
                    Some(i),
                    Some(j),
                    normal,
                    penetration,
                ));
                let half = penetration * 0.5;
                self.spheres[i].center += normal * half;
                self.spheres[j].center -= normal * half;
                self.emit(CollisionEvent::new(
                    CollisionKind::SphereResolved,
                    Some(i),
                    Some(j),
                    normal,
                    penetration,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn aabb(x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32) -> Aabb {
        Aabb::new(Vec3::new(x0, y0, z0), Vec3::new(x1, y1, z1))
    }

    #[test]
    fn aabb_overlap() {
        let a = aabb(0.0, 0.0, 0.0, 2.0, 2.0, 2.0);
        let b = aabb(1.0, 1.0, 1.0, 3.0, 3.0, 3.0);
        assert!(a.overlaps(&b), "部分重叠应判定为碰撞");
    }

    #[test]
    fn aabb_separate() {
        let a = aabb(0.0, 0.0, 0.0, 2.0, 2.0, 2.0);
        // 完全分离
        let b = aabb(3.0, 0.0, 0.0, 5.0, 2.0, 2.0);
        assert!(!a.overlaps(&b), "分离的 AABB 不应判定为碰撞");
        // 仅边缘相贴也不算重叠
        let c = aabb(2.0, 0.0, 0.0, 4.0, 2.0, 2.0);
        assert!(!a.overlaps(&c), "边缘相贴不应判定为重叠");
    }

    #[test]
    fn aabb_resolve_separates_on_minimal_axis() {
        let mut a = aabb(0.0, 0.0, 0.0, 2.0, 2.0, 2.0);
        let b = aabb(1.5, 0.0, 0.0, 3.5, 2.0, 2.0); // X 方向穿透 0.5，为最小穿透轴
        let event = a.resolve(&b).expect("重叠时应有解析结果");
        assert_eq!(event.kind, CollisionKind::AabbResolved);
        assert_eq!(event.normal, Vec3::new(-1.0, 0.0, 0.0));
        assert!((event.penetration - 0.5).abs() < 1e-5);
        assert!(!a.overlaps(&b), "解析后不应再重叠");
        assert!(
            (a.max.x - b.min.x).abs() < 1e-5,
            "解析后 a 应沿 -X 推出并贴住 b 的左侧"
        );
    }

    #[test]
    fn aabb_resolve_none_when_separated() {
        let mut a = aabb(0.0, 0.0, 0.0, 2.0, 2.0, 2.0);
        let b = aabb(10.0, 0.0, 0.0, 12.0, 2.0, 2.0);
        assert!(a.resolve(&b).is_none(), "分离时不应产生解析事件");
    }

    #[test]
    fn sphere_intersect() {
        let a = Sphere::new(Vec3::new(0.0, 0.0, 0.0), 1.0);
        let b = Sphere::new(Vec3::new(1.5, 0.0, 0.0), 1.0); // 球心距 1.5 < 2
        assert!(a.intersects(&b), "球心距小于半径和应相交");
        let c = Sphere::new(Vec3::new(3.0, 0.0, 0.0), 1.0); // 球心距 3 == 半径和
        assert!(!c.intersects(&a), "相切不算相交");
        let d = Sphere::new(Vec3::new(5.0, 0.0, 0.0), 1.0); // 分离
        assert!(!a.intersects(&d), "分离的球体不应判定为相交");
    }

    #[test]
    fn sphere_resolve() {
        let mut a = Sphere::new(Vec3::new(0.0, 0.0, 0.0), 1.0);
        let b = Sphere::new(Vec3::new(1.5, 0.0, 0.0), 1.0);
        let event = a.resolve(&b).expect("相交时应能解析");
        assert_eq!(event.kind, CollisionKind::SphereResolved);
        assert!((event.penetration - 0.5).abs() < 1e-5);
        assert!(!a.intersects(&b), "解析后不应再相交");
    }

    #[test]
    fn gravity_fall_and_ground_rest() {
        let mut world = World::new();
        world.gravity = 9.8;
        world
            .bodies
            .push(Body::new(Vec3::new(0.0, 10.0, 0.0), Vec3::new(1.0, 1.0, 1.0)));
        for _ in 0..300 {
            world.step(1.0 / 60.0);
        }
        let body = &world.bodies[0];
        let rest_y = world.ground_y + body.half_extents.y;
        assert!(body.grounded, "足够时间后应静止在地面");
        assert!(
            (body.position.y - rest_y).abs() < 1e-3,
            "物体应停在 y = {rest_y}，实际 y = {actual}",
            rest_y = rest_y,
            actual = body.position.y
        );
        assert!(body.velocity.y.abs() < 1e-3, "静止时速度应为 0");
    }

    #[test]
    fn ground_bounce_with_restitution() {
        let mut world = World::new();
        world.gravity = 9.8;
        let mut body = Body::new(Vec3::new(0.0, 5.0, 0.0), Vec3::new(0.5, 0.5, 0.5));
        body.restitution = 0.8;
        world.bodies.push(body);

        let mut bounced = false;
        for _ in 0..120 {
            world.step(1.0 / 60.0);
            let b = &world.bodies[0];
            if !b.grounded && b.velocity.y > 0.01 {
                bounced = true;
                break;
            }
        }
        assert!(bounced, "带恢复系数的物体撞地后应反弹");

        for _ in 0..2000 {
            world.step(1.0 / 60.0);
        }
        let body = &world.bodies[0];
        assert!(body.grounded, "多次反弹后最终应静止");
        assert!(
            (body.position.y - (world.ground_y + body.half_extents.y)).abs() < 1e-3,
            "最终应停在地面上"
        );
    }

    #[test]
    fn collision_event_callback() {
        #[derive(Default)]
        struct Counter {
            events: RefCell<Vec<CollisionKind>>,
        }
        impl CollisionListener for Rc<Counter> {
            fn on_collision(&mut self, event: &CollisionEvent) {
                self.events.borrow_mut().push(event.kind);
            }
        }

        let mut world = World::new();
        world.gravity = 0.0; // 关闭重力，避免地面干扰
        world
            .bodies
            .push(Body::new(Vec3::new(0.0, 2.0, 0.0), Vec3::new(1.0, 1.0, 1.0)));
        world
            .bodies
            .push(Body::new(Vec3::new(1.5, 2.0, 0.0), Vec3::new(1.0, 1.0, 1.0)));

        let counter = Rc::new(Counter::default());
        world.add_listener(Box::new(Rc::clone(&counter)));
        world.step(1.0 / 60.0);

        let kinds = counter.events.borrow();
        assert!(
            kinds.contains(&CollisionKind::AabbOverlap),
            "重叠时应发出 AabbOverlap 事件"
        );
        assert!(
            kinds.contains(&CollisionKind::AabbResolved),
            "解析后应发出 AabbResolved 事件"
        );
    }

    #[test]
    fn player_body_new_and_getters() {
        let p = PlayerBody::new(Vec3::new(1.0, 2.0, 3.0), 0.4, 1.6);
        assert_eq!(p.pos(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(p.radius(), 0.4);
        assert_eq!(p.eye_height(), 1.6);
        assert_eq!(p.vel(), Vec3::ZERO);
        assert!(!p.grounded(), "新建玩家默认未着地");
    }

    #[test]
    fn player_free_move_when_far_from_bodies() {
        let mut world = World::new();
        world
            .bodies
            .push(Body::new(Vec3::new(100.0, 0.0, 100.0), Vec3::new(1.0, 1.0, 1.0)));
        let mut player = PlayerBody::new(Vec3::new(0.0, 3.0, 0.0), 0.4, 1.6);
        let moved = player.try_move(&world, 1.5, -2.0);
        assert_eq!(moved, (1.5, -2.0), "远离刚体时实际位移应等于请求位移");
        assert_eq!(player.pos(), Vec3::new(1.5, 3.0, -2.0));
        assert!(!player.collides(&world), "远离刚体时不应判定碰撞");
    }

    #[test]
    fn player_pushed_out_of_aabb() {
        let mut world = World::new();
        // 一堵墙：x ∈ [0, 2]，z ∈ [-1, 1]
        world
            .bodies
            .push(Body::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 2.0, 1.0)));
        let mut player = PlayerBody::new(Vec3::new(-0.5, 5.0, 0.0), 0.4, 1.6);
        let moved = player.try_move(&world, 0.4, 0.0);
        assert!(!player.collides(&world), "被推挤后不应穿透墙体");
        assert!(moved.0 < 0.4, "撞墙后 X 位移应被截断");
        assert!(
            (player.pos.x + player.radius).abs() < 1e-4,
            "圆右缘应贴住墙面 x = 0，实际 x = {}",
            player.pos.x
        );
        assert_eq!(moved.0, player.pos.x - (-0.5), "返回的实际位移应与位置变化一致");
    }

    #[test]
    fn player_diagonal_move_keeps_free_axis() {
        let mut world = World::new();
        // 一堵宽墙横在 +Z 方向：x ∈ [-2, 2]，z ∈ [2, 3]
        world
            .bodies
            .push(Body::new(Vec3::new(0.0, 0.0, 2.5), Vec3::new(2.0, 2.0, 0.5)));
        let mut player = PlayerBody::new(Vec3::new(0.0, 5.0, 0.0), 0.4, 1.6);
        let (mx, mz) = player.try_move(&world, 0.5, 2.5);
        assert!(mx > 0.49, "斜向移动时 X 方向不应被阻挡");
        assert!(mz < 2.5, "斜向移动时 Z 方向应被墙阻挡");
        assert!(!player.collides(&world), "推挤后不应穿透墙体");
        assert!(
            (player.pos.z + player.radius - 2.0).abs() < 1e-4,
            "圆前缘应贴住墙面 z = 2，实际 z = {}",
            player.pos.z
        );
        assert!((player.pos.x - 0.5).abs() < 1e-4, "X 分量应保持请求位移");
    }

    #[test]
    fn player_pushed_out_of_stacked_aabbs() {
        let mut world = World::new();
        // 两个 X 方向叠放的箱子（先放远的 B：x ∈ [0.5, 2]，再放近的 A：x ∈ [0, 1.5]），
        // 玩家一次撞进两个箱子，应经两次推挤后被完整推出
        world
            .bodies
            .push(Body::new(Vec3::new(1.25, 0.0, 0.0), Vec3::new(0.75, 2.0, 0.3)));
        world
            .bodies
            .push(Body::new(Vec3::new(0.75, 0.0, 0.0), Vec3::new(0.75, 2.0, 0.3)));
        let mut player = PlayerBody::new(Vec3::new(-0.5, 5.0, 0.0), 0.4, 1.6);
        let (mx, _) = player.try_move(&world, 0.8, 0.0);
        assert!(!player.collides(&world), "多个 AABB 叠放时也应被推出，不穿透");
        assert!(mx < 0.8, "X 位移应被截断");
        assert!(
            (player.pos.x + player.radius).abs() < 1e-4,
            "圆右缘应贴住最近的箱子左缘 x = 0，实际 x = {}",
            player.pos.x
        );
    }

    #[test]
    fn player_y_untouched_by_collision() {
        let mut world = World::new();
        // 墙的 y 范围 [−2, 2] 不包含玩家 y，但水平碰撞仍应生效
        world
            .bodies
            .push(Body::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 2.0, 1.0)));
        let mut player = PlayerBody::new(Vec3::new(-0.5, 7.25, 0.0), 0.4, 1.6);
        player.try_move(&world, 0.8, 0.0);
        assert!(!player.collides(&world), "推挤后不应穿透墙体");
        assert_eq!(player.pos.y, 7.25, "碰撞推挤不应改动 y（y 由地形高度决定）");
    }

    #[test]
    fn player_collides_uses_expanded_aabb() {
        let mut world = World::new();
        world
            .bodies
            .push(Body::new(Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.5, 2.0, 0.5)));
        // 箱面 z = 1.5，玩家圆心 z = 1.7：距箱面 0.2 < radius 0.4 → 碰撞
        let near = PlayerBody::new(Vec3::new(0.0, 5.0, 1.7), 0.4, 1.6);
        assert!(near.collides(&world), "距箱面小于 radius 时应判定碰撞");
        // 圆心 z = 1.1：距箱面 0.4 == radius → 相切不算碰撞
        let touching = PlayerBody::new(Vec3::new(0.0, 5.0, 1.1), 0.4, 1.6);
        assert!(!touching.collides(&world), "相切不应判定为碰撞");
        // 圆心 z = 1.0：距箱面 0.5 > radius → 分离
        let far = PlayerBody::new(Vec3::new(0.0, 5.0, 1.0), 0.4, 1.6);
        assert!(!far.collides(&world), "分离时不应判定为碰撞");
    }

    #[test]
    fn player_collide_world_reports_push() {
        let mut world = World::new();
        world
            .bodies
            .push(Body::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 2.0, 1.0)));
        let mut player = PlayerBody::new(Vec3::new(-0.5, 5.0, 0.0), 0.4, 1.6);
        player.pos.x += 0.3; // 进入重叠：距墙面 0.2 < radius
        assert!(player.collide_world(&world), "发生推挤时应返回 true");
        assert!(!player.collides(&world), "推挤后不应再重叠");
        assert!(!player.collide_world(&world), "无重叠时不应返回 true");
    }
}
