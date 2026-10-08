// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Game {
    /// 投射物是否命中物理刚体/球体（命中即销毁，不产生击杀）。
    /// V3.0 高速弹（710m/s → 每帧 11.8m）不能用单点采样（会跳过小掩体/球体），
    /// 改为上一帧→当前位置的线段与 AABB/球体求交。
    pub(crate) fn collide_physics(&self, p: &Projectile) -> bool {
        let (ax, ay, az) = (p.prev_position()[0], p.prev_position()[1], p.prev_position()[2]);
        let (bx, by, bz) = (p.position[0], p.position[1], p.position[2]);
        for body in &self.world.bodies {
            let aabb = body.aabb();
            if Self::segment_hits_aabb(ax, ay, az, bx, by, bz, &aabb) {
                return true;
            }
        }
        for s in &self.world.spheres {
            if Self::segment_hits_sphere(ax, ay, az, bx, by, bz, s) {
                return true;
            }
        }
        false
    }
    /// 累计碰撞事件数（供 UI / 日志）
    pub(crate) fn total_collisions(&self) -> u64 {
        self.total_collisions
    }
    /// 取走本帧碰撞事件并累计计数（限频打一条日志）
    pub(crate) fn drain_collisions(&mut self) {
        if let Ok(mut buf) = self.event_buf.lock() {
            self.collisions = std::mem::take(&mut *buf);
        }
        if self.collisions.is_empty() {
            return;
        }
        self.total_collisions += self.collisions.len() as u64;
        if self.time - self.last_event_log_time >= 0.5 {
            self.last_event_log_time = self.time;
            // 直方图（不再全量枚举/格式化，消除逐帧日志刷屏）
            let (mut ov, mut rs, mut si, mut sr, mut gh) = (0usize, 0usize, 0usize, 0usize, 0usize);
            for e in &self.collisions {
                match e.kind {
                    physics::CollisionKind::AabbOverlap => ov += 1,
                    physics::CollisionKind::AabbResolved => rs += 1,
                    physics::CollisionKind::SphereIntersect => si += 1,
                    physics::CollisionKind::SphereResolved => sr += 1,
                    physics::CollisionKind::GroundHit => gh += 1,
                }
            }
            log::info!(
                "physics: {} events (overlap={} resolved={} sphere={} sphere_res={} ground={})",
                self.collisions.len(),
                ov,
                rs,
                si,
                sr,
                gh
            );
        }
    }
}
