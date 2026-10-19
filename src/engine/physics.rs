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
    /// 静态刚体（2026-08-23）：地图障碍/地基——不受重力、地面钳位、对撞只推动动态方；
    /// 此前把障碍当动态刚体导致树桩沉地 1m、地基板沉入 4.5m、NPC 可撞动墙体
    pub static_body: bool,
}

impl Body {
    pub fn new(position: Vec3, half_extents: Vec3) -> Self {
        Self {
            position,
            half_extents,
            velocity: Vec3::ZERO,
            restitution: 0.0,
            grounded: false,
            static_body: false,
        }
    }

    /// 静态刚体（地图障碍：不随重力/对撞移动）
    pub fn new_static(position: Vec3, half_extents: Vec3) -> Self {
        Self {
            position,
            half_extents,
            velocity: Vec3::ZERO,
            restitution: 0.0,
            grounded: true,
            static_body: true,
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
        // 🔴🔴 2026-09-13：**加 Y 判据**。此前这里只看水平距离，`Aabb` 的 min/max.y
        // 完全没参与 —— 于是"不管你在多高，只要 XZ 落进盒子就被水平推开"：
        //   * 你永远没法站到任何东西上面（楼顶/集装箱/掩体顶）
        //   * 而垂直方向只有"贴地形高度"，没有任何顶面支撑
        // 两者合起来就是用户报的"嵌到地板里面/穿进墙里"那一类症状。
        //
        // 现在：只有玩家的**垂直区间**与盒子相交时才推挤。
        // 站在盒子上方（脚底 ≥ 盒子顶面）时不再被推开 —— 顶面支撑由
        // `support_height` 提供（见那里的注释）。
        let feet = self.pos.y;
        let head = self.pos.y + self.eye_height;
        if head <= aabb.min.y || feet >= aabb.max.y {
            return None; // 垂直方向无交集：完全在盒子上方或下方
        }
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

    /// 🔴 2026-09-13：**顶面支撑高度** —— 玩家脚下能站住的最高平面。
    ///
    /// 与 `push_out_of_aabb` 配套：那里加了 Y 判据后，站在盒子上方就不再被推开，
    /// 于是必须有东西接住玩家，否则会直接穿进盒子里掉下去。
    ///
    /// 规则：取所有**水平范围内、且顶面不高于 `pos.y + STEP_UP`** 的盒子的顶面最大值。
    /// - 水平范围按 `radius` 外扩，与推挤判据同源（脚站在边缘也算站住）
    /// - `STEP_UP` 是抬脚上限：路缘/台阶能上，1.5m 的护栏不能——必须跳
    /// - 没有任何盒子时为 `f32::NEG_INFINITY`，由调用方与地形高度取 max
    pub fn support_height(&self, world: &World, step_up: f32) -> f32 {
        let mut best = f32::NEG_INFINITY;
        let limit = self.pos.y + step_up;
        for body in &world.bodies {
            let a = body.aabb();
            if a.max.y > limit || a.max.y <= best {
                continue;
            }
            if self.pos.x > a.min.x - self.radius
                && self.pos.x < a.max.x + self.radius
                && self.pos.z > a.min.z - self.radius
                && self.pos.z < a.max.z + self.radius
            {
                best = a.max.y;
            }
        }
        best
    }

    /// 是否与任一刚体的 AABB（水平扩展 radius 后）重叠
    /// 玩家是否与任一刚体**在三维上**重叠。
    ///
    /// 🔴 **必须与 `push_out_of_aabb` 同一套判据**（2026-10-15，外部复核指出第 10 条，
    /// 已对着代码确认）：兄弟方法在 2026-09-13 专门加了 Y 轴判据
    /// （`if head <= aabb.min.y || feet >= aabb.max.y { return None; }`），
    /// 好让玩家**能站在矮箱/台基顶上而不被水平推开**；
    /// 而这里原来只查水平 ⇒ **站在箱顶时 `push_out_of_aabb` 不推、`collides` 却报"重叠"**,
    /// 两个方法对同一个几何状态给出相反答案。
    ///
    /// ⚠️ 现有调用点**全在测试里**（拿它当"推挤后是否还穿透墙"的判据），
    /// 所以这属于**语义不一致**而不是线上故障；但语义不一致本身就会让下一个人写错判据。
    pub fn collides(&self, world: &World) -> bool {
        let feet = self.pos.y;
        let head = self.pos.y + self.eye_height;
        world.bodies.iter().any(|body| {
            let a = body.aabb();
            // 垂直方向无交集 ⇒ 不算碰撞（与 `push_out_of_aabb` 逐字同源）
            if head <= a.min.y || feet >= a.max.y {
                return false;
            }
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
            if body.static_body {
                continue;
            }
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
    /// 宽相：找出所有 AABB 重叠的**无序对**，按 `(i, j)` 字典序返回。
    ///
    /// 纯函数（只看 `aabbs`），因此可以直接拿暴力双循环做等价性判据 —— 见
    /// `sweep_and_prune_matches_brute_force_pairs`。
    ///
    /// 算法：**扫掠-裁剪**。按 `min.x` 升序排一遍，然后对每个 i 只向后扫到
    /// `aabbs[j].min.x > aabbs[i].max.x` 就停 —— 两个 AABB 重叠 ⇒ x 区间必相交，
    /// 所以这一步**不丢任何对**。最后排序回 `(i, j)` 字典序，使调用方的访问顺序、
    /// 事件顺序与原来的暴力双循环**逐位一致**。
    ///
    /// 复杂度：稀疏分布下 O(n log n + k)（k = 实际重叠对数），而不是 O(n²)。
    /// 本仓城市地图有几千个静态障碍，实测这一处占了整帧的一半（见调用点的长注释）。
    fn overlapping_pairs(aabbs: &[Aabb]) -> Vec<(u32, u32)> {
        let n = aabbs.len();
        if n < 2 {
            return Vec::new();
        }
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_unstable_by(|&a, &b| {
            let (x, y) = (aabbs[a as usize].min.x, aabbs[b as usize].min.x);
            x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for oi in 0..n {
            let i = order[oi] as usize;
            let max_x = aabbs[i].max.x;
            for oj in (oi + 1)..n {
                let j = order[oj] as usize;
                if aabbs[j].min.x > max_x {
                    break;
                }
                if aabbs[i].overlaps(&aabbs[j]) {
                    pairs.push(if i < j {
                        (i as u32, j as u32)
                    } else {
                        (j as u32, i as u32)
                    });
                }
            }
        }
        pairs.sort_unstable();
        pairs
    }

    fn resolve_body_pairs(&mut self) {
        let n = self.bodies.len();
        if n < 2 {
            return;
        }
        // 🔴 宽相：**扫掠-裁剪（sweep-and-prune）**，取代原来的双重循环（2026-10-12）。
        //
        // 为什么改（实机实测，这是整帧最大的单一热点）：
        //   城市地图把每个障碍都建成静态刚体（`map.obstacles`，本机约 4~5 千个），
        //   而原实现是 `for i in 0..n { for j in (i+1)..n { ... } }` ⇒ **O(n²)**：
        //   每帧上千万次 `Aabb::overlaps`。实机 `game:` 行读到 `phys_us=3241`（整帧 6500µs），
        //   而 `physics: 1064 events (overlap=1064 resolved=0)` —— **1064 个事件全是
        //   静态-静态重叠**（相邻建筑本来就互相压着），它们**永远不需要解析**，
        //   却每帧都要被全量扫出来。修前/修后的成对集合**必须逐对一致**（见下）。
        //
        // 为什么这样改是**安全的逐位等价**（不是"近似"）：
        //   ① 配对集合**完全由快照 `aabbs` 决定** —— `overlaps()` 读的是快照，
        //      而下面的解析改的是活的 `self.bodies`。所以"哪些对重叠"是一个纯函数。
        //   ② 裁剪只按 x 区间：`aabbs[i]` 与 `aabbs[j]` 重叠 ⇒ 两者 x 区间必相交
        //      ⇒ 只要 `min.x` 已排序，扫到 `min.x > max_x` 就可以安全停。
        //   ③ 收集完统一 `sort_unstable()` 回 `(i, j)` 字典序 ⇒ **与暴力双循环的访问顺序、
        //      事件顺序、解析累加顺序完全一致**。
        //   判据：`sweep_and_prune_matches_brute_force_pairs`（随机 AABB 上逐对比对）。
        //
        // ⚠️ 不能改成"跳过静态-静态对"：那样会把 1064 个 `AabbOverlap` 事件一起消掉，
        //    属于**静默改变行为**。裁剪只减少"比较次数"，一个事件都不少。
        let aabbs: Vec<Aabb> = self.bodies.iter().map(Body::aabb).collect();
        let pairs = Self::overlapping_pairs(&aabbs);

        for &(pi, pj) in &pairs {
            let (i, j) = (pi as usize, pj as usize);
            self.emit(CollisionEvent::new(
                CollisionKind::AabbOverlap,
                Some(i),
                Some(j),
                Vec3::ZERO,
                0.0,
            ));
            if let Some((normal, penetration)) = aabb_separation(&aabbs[i], &aabbs[j]) {
                let (si, sj) = (self.bodies[i].static_body, self.bodies[j].static_body);
                // 2026-08-23 静态刚体：静态-静态不解析；静态-动态只推动态方（障碍不被撞动）
                if si && sj {
                    continue;
                }
                // 🔴 **叠在别的刚体上也要有"支撑"语义**（2026-10-19 外部复核缺陷 5-C，已确认）：
                // 旧实现只挪位置、从不置 `grounded`/清零 `velocity.y`；而 `step_bodies` 的
                // `grounded` 只听 `ground_y` ⇒ **放在静态台面上的动态刚体会一直受重力累加速度**
                // （10 秒后 velocity.y ≈ -98 m/s）⇒ 某帧位移超过台面厚度 ⇒ **直接穿透掉到底面**。
                // 修法：谁被**沿 +Y 推开**（normal.y 方向判定见下），谁就算 grounded，并把向下速度清零。
                // 支撑判定：被推开的位移方向 `up = ±normal`，`up.y > 0.5` 才算"踩在上面"。
                let mut grounded_now: [bool; 2] = [false, false];
                if si {
                    self.bodies[j].position -= normal * penetration;
                    grounded_now[1] = -normal.y > 0.5;
                } else if sj {
                    self.bodies[i].position += normal * penetration;
                    grounded_now[0] = normal.y > 0.5;
                } else {
                    let half = penetration * 0.5;
                    self.bodies[i].position += normal * half;
                    self.bodies[j].position -= normal * half;
                    grounded_now[0] = normal.y > 0.5;
                    grounded_now[1] = -normal.y > 0.5;
                }
                for (k, up) in [(i, grounded_now[0]), (j, grounded_now[1])] {
                    if !up || self.bodies[k].static_body {
                        continue;
                    }
                    self.bodies[k].grounded = true;
                    if self.bodies[k].velocity.y < 0.0 {
                        self.bodies[k].velocity.y = 0.0;
                    }
                }
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

    // ------------------------------------------------------------------
    // 退化输入 / NaN 不变量（2026-09-23 复查补）
    //
    // 为什么专门测这个：碰撞解析里一旦出现 NaN，**所有比较都会变成 false** ——
    // 物体既不相交也不落地，从此静默穿墙/穿地，**不 panic、不报错、日志里什么都没有**。
    // 现有实现是安全的（法向走轴对齐 ±1 或带 `dist_sq > 1e-12` 守卫），但此前**没有任何测试锁住它**：
    // 谁把法向"简化"成一句 `delta.normalized()`（中心重合时 0 * inf = NaN）就会静默退化。
    // 这一组就是那条锁。
    // ------------------------------------------------------------------

    fn assert_finite3(v: Vec3, what: &str) {
        assert!(
            v.x.is_finite() && v.y.is_finite() && v.z.is_finite(),
            "{what} 必须是有限值，实际 {v:?}"
        );
    }

    #[test]
    fn normalized_zero_vector_is_zero_not_nan() {
        let n = Vec3::new(0.0, 0.0, 0.0).normalized();
        assert_eq!(n, Vec3::ZERO, "零向量归一化必须回零向量（否则是 NaN 源）");
        assert_finite3(n, "零向量归一化结果");
    }

    #[test]
    fn aabb_separation_handles_identical_and_degenerate_boxes() {
        // ① 完全重合（同位置同尺寸）：法向必须是**轴对齐单位向量**，穿透有限
        let a = aabb(0.0, 0.0, 0.0, 2.0, 2.0, 2.0);
        let (normal, pen) = aabb_separation(&a, &a).expect("完全重合也应当有解析结果");
        assert_finite3(normal, "重合盒法向");
        assert!(pen.is_finite() && pen > 0.0, "穿透必须有限且为正: {pen}");
        let comps = [normal.x, normal.y, normal.z];
        let nonzero: Vec<f32> = comps.iter().copied().filter(|v| v.abs() > 0.0).collect();
        // ⚠️ 判据必须是"恰好一个分量为 ±1"：写成 `.filter(..).all(..)` 时，
        // **零法向会空集通过**（`all` 对空迭代器恒真）—— 那正是"推不动"的退化解，必须红。
        assert_eq!(
            nonzero.len(),
            1,
            "法向必须恰好沿一个轴（零法向 = 推不动 ⇒ 退化）: {normal:?}"
        );
        assert!(
            (nonzero[0].abs() - 1.0).abs() < 1e-6,
            "该轴分量必须是 ±1: {normal:?}"
        );

        // ② 零尺寸盒（min == max）嵌在大盒内部：不得产生 NaN
        let mut point = aabb(1.0, 1.0, 1.0, 1.0, 1.0, 1.0);
        let big = aabb(0.0, 0.0, 0.0, 2.0, 2.0, 2.0);
        let event = point.resolve(&big).expect("零尺寸盒在内部时仍应解析");
        assert!(
            event.penetration.is_finite() && event.penetration > 0.0,
            "零尺寸盒穿透必须有限且为正: {}",
            event.penetration
        );
        assert_finite3(point.min, "零尺寸盒解析后的 min");
        assert_finite3(point.max, "零尺寸盒解析后的 max");
    }

    #[test]
    fn sphere_resolve_handles_coincident_centers() {
        // 中心完全重合：按文档沿 +X 推出，穿透 = r1 + r2，**绝不能是 NaN**
        let mut a = Sphere::new(Vec3::new(3.0, 4.0, 5.0), 1.0);
        let b = Sphere::new(Vec3::new(3.0, 4.0, 5.0), 0.5);
        assert!(a.intersects(&b), "前置：中心重合必然相交");
        let event = a.resolve(&b).expect("重合球应有解析结果");
        assert_eq!(event.normal, Vec3::new(1.0, 0.0, 0.0), "重合时按文档沿 +X");
        assert!((event.penetration - 1.5).abs() < 1e-6, "穿透应为 r1+r2");
        assert_finite3(a.center, "重合球解析后的圆心");
        assert!(!a.intersects(&b), "解析后不应再相交");
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

    /// 🔴 **叠在静态刚体上的动态刚体必须有支撑语义**（2026-10-19 外部复核缺陷 5-C，已确认）。
    ///
    /// 旧实现 `resolve_body_pairs` 只挪位置，从不置 `grounded`/清零 `velocity.y`；
    /// 而 `step_bodies` 的 `grounded` 只听 `ground_y` ⇒ 台面上的动态体一直累加向下速度，
    /// 直到单帧位移超过台面厚度 ⇒ **直接穿透掉到底面**。
    /// 反向验证：删掉 `grounded_now` 那段 ⇒ 本判据红（速度会一路变负）。
    #[test]
    fn body_resting_on_static_platform_stays_supported() {
        let mut world = World::new();
        world.gravity = 9.8;
        // 静态台面：中心 y=0.5、半高 0.5 ⇒ 顶面 y=1.0
        let mut platform = Body::new(Vec3::new(0.0, 0.5, 0.0), Vec3::new(2.0, 0.5, 2.0));
        platform.static_body = true;
        world.bodies.push(platform);
        // 动态盒：底面刚好落在台面上方一点，带向下初速度
        let mut box_body = Body::new(Vec3::new(0.0, 1.3, 0.0), Vec3::new(0.2, 0.2, 0.2));
        box_body.velocity = Vec3::new(0.0, -3.0, 0.0);
        world.bodies.push(box_body);

        for _ in 0..180 {
            world.step(1.0 / 60.0);
        }
        let b = &world.bodies[1];
        let platform_top = 1.0;
        let bottom = b.position.y - b.half_extents.y;
        assert!(
            bottom > platform_top - 0.05,
            "动态盒穿透进台面了：底面 {bottom} < 台面顶 {platform_top}"
        );
        assert!(b.grounded, "叠在静态台面上应判为 grounded");
        assert!(
            b.velocity.y > -0.5,
            "支撑后向下速度应被清零（实际 {} —— 说明`grounded` 从未被置位，重力在无界累加）",
            b.velocity.y
        );
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
        let mut player = PlayerBody::new(Vec3::new(-0.5, 0.0, 0.0), 0.4, 1.6);
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
        let mut player = PlayerBody::new(Vec3::new(0.0, 0.0, 0.0), 0.4, 1.6);
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
        let mut player = PlayerBody::new(Vec3::new(-0.5, 0.0, 0.0), 0.4, 1.6);
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
        // 墙的 y 范围 [-2, 2]，玩家脚底 0.0 ⇒ 垂直区间相交 ⇒ 水平推挤生效
        world
            .bodies
            .push(Body::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 2.0, 1.0)));
        let mut player = PlayerBody::new(Vec3::new(-0.5, 0.0, 0.0), 0.4, 1.6);
        player.try_move(&world, 0.8, 0.0);
        assert!(!player.collides(&world), "推挤后不应穿透墙体");
        assert_eq!(player.pos.y, 0.0, "碰撞推挤不应改动 y（y 由地形高度决定）");
    }

    /// 🔴 2026-09-13 新增：**碰撞现在是 Y 感知的**。
    ///
    /// 旧契约（本条测试的前身曾明确断言）："墙的 y 范围不包含玩家 y，
    /// **但水平碰撞仍应生效**" —— 也就是不管你在多高，只要 XZ 落进盒子就被水平推开。
    /// 那条契约的直接后果是**永远站不到任何东西上面**，而且与"垂直方向只贴地形高度、
    /// 没有顶面支撑"合起来，就是用户报的"嵌进地板/穿进墙里"。
    /// 现改为：垂直区间不相交就不推挤，顶面支撑交给 `support_height`。
    #[test]
    fn player_above_a_box_is_not_pushed_and_can_stand_on_it() {
        let mut world = World::new();
        // 一个箱子：x ∈ [0,2]，z ∈ [-1,1]，**顶面 y = 1.0**
        world
            .bodies
            .push(Body::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0)));

        // 站在箱子上方（脚底 1.0 = 顶面）：不应被水平推开
        let mut above = PlayerBody::new(Vec3::new(1.0, 1.0, 0.0), 0.4, 1.6);
        let moved = above.try_move(&world, 0.3, 0.0);
        assert!(
            (moved.0 - 0.3).abs() < 1e-4,
            "站在箱子顶上时水平移动不应被截断，实际位移 {}",
            moved.0
        );

        // 而 `support_height` 应当把箱顶报成可站立面
        let support = above.support_height(&world, 0.45);
        assert!(
            (support - 1.0).abs() < 1e-4,
            "箱子顶面应被报为支撑面，实际 {}",
            support
        );

        // 站在箱子**旁边**且垂直区间相交：仍应被推开（墙还是墙）
        let mut beside = PlayerBody::new(Vec3::new(-0.5, 0.0, 0.0), 0.4, 1.6);
        let moved_b = beside.try_move(&world, 0.8, 0.0);
        assert!(
            moved_b.0 < 0.8,
            "垂直区间相交时仍应被挡住，实际位移 {}",
            moved_b.0
        );
    }

    /// `support_height` 的抬脚上限：高过 `step_up` 的面不能被当作支撑，
    /// 否则玩家会"贴"着 1.5m 护栏一路走上去。
    #[test]
    fn support_height_respects_step_up() {
        let mut world = World::new();
        // 顶面 y = 2.0 的高台
        world
            .bodies
            .push(Body::new(Vec3::new(0.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 1.0)));
        let low = PlayerBody::new(Vec3::new(0.0, 0.0, 0.0), 0.4, 1.6);
        assert!(
            low.support_height(&world, 0.45).is_infinite()
                && low.support_height(&world, 0.45) < 0.0,
            "2.0m 的高台不该在 0.45m 抬脚上限内成为支撑面"
        );
        // 已经站到上面时就该是支撑面
        let high = PlayerBody::new(Vec3::new(0.0, 2.0, 0.0), 0.4, 1.6);
        assert!((high.support_height(&world, 0.45) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn player_collides_uses_expanded_aabb() {
        let mut world = World::new();
        world
            .bodies
            .push(Body::new(Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.5, 2.0, 0.5)));
        // 🔴 玩家 y 从 5.0 改成 1.0（2026-10-15）：箱子 aabb 是 y∈[-2,2]，
        // 而 `collides` 现在**与 `push_out_of_aabb` 一样带 Y 判据**
        // ⇒ y=5.0 时垂直方向无交集、任何水平位置都不算碰撞（那样这条测试就成了恒假）。
        // 本测试要量的是**水平外扩**，所以把玩家放进箱子的垂直区间里。
        // 箱面 z = 1.5，玩家圆心 z = 1.7：距箱面 0.2 < radius 0.4 → 碰撞
        let near = PlayerBody::new(Vec3::new(0.0, 1.0, 1.7), 0.4, 1.6);
        assert!(near.collides(&world), "距箱面小于 radius 时应判定碰撞");
        // 圆心 z = 1.1：距箱面 0.4 == radius → 相切不算碰撞
        let touching = PlayerBody::new(Vec3::new(0.0, 1.0, 1.1), 0.4, 1.6);
        assert!(!touching.collides(&world), "相切不应判定为碰撞");
        // 圆心 z = 1.0：距箱面 0.5 > radius → 分离
        let far = PlayerBody::new(Vec3::new(0.0, 1.0, 1.0), 0.4, 1.6);
        assert!(!far.collides(&world), "分离时不应判定为碰撞");

        // 🔴 **区分度档**：水平完全重叠、但玩家**站在箱子顶上**
        // ⇒ `collides` 必须为 false（与 `push_out_of_aabb` 同一套判据）。
        // 这一档正是原实现漏掉的：它只查水平 ⇒ 会报"站在箱顶 = 重叠"。
        let on_top = PlayerBody::new(Vec3::new(0.0, 2.01, 2.0), 0.4, 1.6);
        assert!(
            !on_top.collides(&world),
            "站在箱子顶上（垂直无交集）不应判定为碰撞 —— 否则与 `push_out_of_aabb` 的\n\
             「不推开、可以站在上面」直接矛盾"
        );
    }

    #[test]
    fn player_collide_world_reports_push() {
        let mut world = World::new();
        world
            .bodies
            .push(Body::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 2.0, 1.0)));
        let mut player = PlayerBody::new(Vec3::new(-0.5, 0.0, 0.0), 0.4, 1.6);
        player.pos.x += 0.3; // 进入重叠：距墙面 0.2 < radius
        assert!(player.collide_world(&world), "发生推挤时应返回 true");
        assert!(!player.collides(&world), "推挤后不应再重叠");
        assert!(!player.collide_world(&world), "无重叠时不应返回 true");
    }
    /// 🔴 **扫掠-裁剪必须与暴力双循环给出逐对完全一致的配对集合**。
    ///
    /// 存在理由（2026-10-12）：城市地图有几千个静态障碍，原实现是 O(n²) 的
    /// `for i { for j>i { overlaps() } }`，实机实测 `phys_us=3241`（整帧 6500µs），
    /// 且每帧 1064 个事件**全是静态-静态重叠**（永不需要解析）。
    /// 改成扫掠-裁剪后**不允许少一对、也不允许多一对**，更不允许换顺序 ——
    /// 调用方按 `(i, j)` 顺序 emit 并累加位置修正，顺序变了结果就不逐位一致。
    ///
    /// 判据覆盖三类分布（都要）：稀疏均匀 / 大量重叠 / 全部同 x（裁剪最弱的情形）。
    #[test]
    fn sweep_and_prune_matches_brute_force_pairs() {
        // 确定性 LCG，避免引入 dev-dependency
        let mut seed: u64 = 0x5EED_2026;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as u32 as f32) / (u32::MAX as f32 / 2.0) - 1.0
        };

        for case in 0..6 {
            let n = match case {
                0 => 0usize,
                1 => 1,
                2 => 300,
                3 => 800,
                4 => 400,
                _ => 600,
            };
            let mut aabbs: Vec<Aabb> = Vec::with_capacity(n);
            for i in 0..n {
                let f = i as f32;
                let (cx, cy, cz, half) = match case {
                    // 稀疏均匀：几乎不重叠（裁剪收益最大的情形）
                    2 => (next() * 500.0, next() * 20.0, next() * 500.0, 1.0f32),
                    // 全部同 x：x 裁剪完全失效（最坏情形，必须仍然正确）
                    3 => (0.0, next() * 20.0, next() * 50.0, 2.0f32),
                    // 🔴 精确网格相切：相邻盒的 max.x 恰好等于下一个的 min.x。
                    //    这一档专门抓"裁剪条件写成 >= 而不是 >" —— 那会丢掉所有相切对，
                    //    而随机数据几乎永不产生精确相等，所以**必须**有这一档。
                    4 => ((f % 20.0) * 2.0, 0.0, (f / 20.0).floor() * 2.0, 1.0f32),
                    // 🔴 x 顺序与下标顺序**相反**：抓"忘了最后那次排序"（顺序会错）。
                    5 => ((600.0 - f) * 3.0, 0.0, 0.0, 2.0f32),
                    // 大量重叠
                    _ => (next() * 20.0, next() * 5.0, next() * 20.0, 4.0f32),
                };
                aabbs.push(Aabb::new(
                    Vec3::new(cx - half, cy - half, cz - half),
                    Vec3::new(cx + half, cy + half, cz + half),
                ));
            }

            // 暴力双循环（参照实现，逐字对应改动前的代码）
            let mut brute: Vec<(u32, u32)> = Vec::new();
            for i in 0..n {
                for j in (i + 1)..n {
                    if aabbs[i].overlaps(&aabbs[j]) {
                        brute.push((i as u32, j as u32));
                    }
                }
            }

            let swept = World::overlapping_pairs(&aabbs);
            assert_eq!(
                swept.len(),
                brute.len(),
                "case {case}: 配对数不一致（扫掠 {} vs 暴力 {}）",
                swept.len(),
                brute.len()
            );
            assert_eq!(swept, brute, "case {case}: 配对集合或顺序不一致");
            // 顺带钉住"必须是 (i, j) 升序"这件事本身
            assert!(
                swept.windows(2).all(|w| w[0] < w[1]),
                "case {case}: 返回顺序不是 (i, j) 字典序"
            );
        }
    }
}
