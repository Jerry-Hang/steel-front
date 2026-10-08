// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Stance {
    /// 水平移动速度倍率（相对站立）
    pub(crate) fn speed_mul(self) -> f32 {
        match self {
            Stance::Standing => 1.0,
            Stance::Crouching => 0.45,
            Stance::Prone => 0.18,
        }
    }

    /// 相机视高（米，相对脚底）。站立值与物理体默认一致（1.6）。
    pub(crate) fn eye_height(self) -> f32 {
        match self {
            Stance::Standing => 1.60,
            Stance::Crouching => 1.02,
            Stance::Prone => 0.42,
        }
    }
}
impl FireMode {
    /// 每次按下要连打的发数。双发与三连发**共用同一条连打路径**（`fire_burst*`），
    /// 所以档位只在这里描述一次，别在开火代码里再写死 2 或 3。
    pub(crate) fn burst_rounds(self) -> u32 {
        match self {
            FireMode::Semi | FireMode::Auto => 1,
            FireMode::Burst2 => 2,
            FireMode::Burst3 => 3,
        }
    }

    /// 下一个模式（B 键循环）
    pub(crate) fn next(self) -> FireMode {
        match self {
            FireMode::Semi => FireMode::Burst2,
            FireMode::Burst2 => FireMode::Burst3,
            FireMode::Burst3 => FireMode::Auto,
            FireMode::Auto => FireMode::Semi,
        }
    }

    /// 中文显示名（HUD）
    pub(crate) fn label(self) -> &'static str {
        match self {
            FireMode::Semi => "单发",
            FireMode::Burst2 => "双发",
            FireMode::Burst3 => "三连发",
            FireMode::Auto => "连发",
        }
    }
}
impl Default for AiTierParams {
    fn default() -> Self {
        Self { near_radius: 100.0 }
    }
}
impl MapObstacle {
    /// 贴地障碍构造（y = half_h = 1.2，tint = 种类默认）
    pub(crate) fn new(kind: ObstacleKind, x: f32, z: f32, half_w: f32, half_d: f32) -> Self {
        let max_hp = obstacle_max_hp(kind);
        MapObstacle {
            x,
            z,
            half_w,
            half_d,
            y: MAP_BLOCK_HEIGHT * 0.5,
            half_h: MAP_BLOCK_HEIGHT * 0.5,
            kind,
            tint: None,
            max_hp,
            hp: max_hp,
            shape: Shape::Legacy,
        }
    }

    /// 指定半高/中心高/tint（城市手绘布局用）
    pub(crate) fn shaped(mut self, half_h: f32, y: f32, tint: Option<[f32; 3]>) -> Self {
        self.half_h = half_h;
        self.y = y;
        self.tint = tint;
        self
    }

    /// 指定几何形状（只改渲染模板；碰撞仍是 AABB，见 [`MapObstacle::shape`] 的说明）。
    pub(crate) fn geom(mut self, shape: Shape) -> Self {
        self.shape = shape;
        self
    }
}
impl LevelMap {
    /// 渲染/路径追踪视角：参与绘制的一切几何（障碍 + 装饰）。
    /// 顺序固定为 obstacles 在前、decor 在后，两条路径共用同一索引基准。
    pub(crate) fn render_geometry(&self) -> impl Iterator<Item = &MapObstacle> {
        self.obstacles.iter().chain(self.decor.iter())
    }
}
impl MissionObjective {
    /// 新建任务目标
    pub(crate) fn new(target: u32) -> Self {
        Self {
            target,
            eliminated: 0,
            done: false,
        }
    }

    /// 登记 `kills` 名敌人被歼灭；返回本次调用是否首次达成目标
    pub(crate) fn progress(&mut self, kills: u32) -> bool {
        if self.done {
            return false;
        }
        self.eliminated = (self.eliminated + kills).min(self.target);
        if self.target > 0 && self.eliminated >= self.target {
            self.done = true;
            true
        } else {
            false
        }
    }
}
impl CollisionListener for EventBuffer {
    fn on_collision(&mut self, event: &CollisionEvent) {
        if let Ok(mut buf) = self.0.lock() {
            buf.push(*event);
        }
    }
}
impl ImpactMark {
    /// 尺寸包络 0..1：正常为 1，生命最后 [`IMPACT_MARK_FADE`] 秒线性收缩到 0。
    pub(crate) fn size_envelope(&self) -> f32 {
        ((IMPACT_MARK_LIFE - self.age) / IMPACT_MARK_FADE).clamp(0.0, 1.0)
    }

    /// 局部 +Z 对齐 `normal` 的**右手**正交基 `(tangent, up, normal)`。
    ///
    /// 右手（行列式 +1）是硬要求：弹孔是个方片，基一旦左手化就是正面绕序反掉 ——
    /// **整片黑掉且不报错**（见 AGENTS 铁律 B）。参考轴取与法线最不平行的坐标轴，
    /// 保证叉积不退化。
    pub(crate) fn basis(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let n = glam::Vec3::from(self.normal);
        let n = if n.length_squared() > 1e-12 {
            n.normalize()
        } else {
            glam::Vec3::Z
        };
        let a = if n.x.abs() <= n.y.abs() && n.x.abs() <= n.z.abs() {
            glam::Vec3::X
        } else if n.y.abs() <= n.z.abs() {
            glam::Vec3::Y
        } else {
            glam::Vec3::Z
        };
        let t = a.cross(n).normalize();
        let u = n.cross(t);
        (t.into(), u.into(), n.into())
    }
}
