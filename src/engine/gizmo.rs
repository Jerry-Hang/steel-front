//! 三轴 gizmo 的取点与拖动（引擎化 P2，见 `docs/ENGINE-PLAN.md`）。
//!
//! 编辑器里"抓住 X 轴把手拖一下"要解决的只有一件事：**把鼠标射线投到那条轴上**，
//! 拿到一个沿轴的一维参数 `t`（单位：米，因为轴是单位向量）。两次取点的 `t` 相减
//! 就是这次拖动应该产生的位移 —— 与屏幕分辨率、相机距离、FOV 都无关，
//! 所以同一个实现既能给鼠标用，也能给 AI 接口用（铁律 4）。
//!
//! 为什么不用屏幕空间近似：透视投影下"屏幕上拖 10 像素"在不同深度对应的世界位移差很多，
//! 近处会拖飞、远处拖不动。投影到轴上是唯一在任意视角下都稳定的做法。

use glam::Vec3;

/// 三条世界轴。**不做局部轴**（那是 P3 有旋转后的下一步）：
/// 现在场景里的实体只有平移有意义，先把它做对。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    /// 单位方向。
    pub fn dir(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }

    /// 从命令参数解析（`x`/`X` 都收）。
    pub fn parse(s: &str) -> Option<Axis> {
        match s.trim().to_ascii_uppercase().as_str() {
            "X" => Some(Axis::X),
            "Y" => Some(Axis::Y),
            "Z" => Some(Axis::Z),
            _ => None,
        }
    }
}

/// 射线与"过 `point`、方向 `axis`"的直线**最近点的轴参数**。
///
/// 记 `w0 = point - ray_o`，`b = axis·ray_d`，`d = axis·w0`，`e = ray_d·w0`
/// （两条方向都是单位向量 ⇒ `a = c = 1`）：
/// ```text
/// t = (b·e - d) / (1 - b²)
/// ```
/// 分母趋零 = 射线与轴几乎平行 ⇒ 这种情况下"最近点"对数值偏差极敏感（视线上任意点都
/// 近似最近），编辑器必须**拒绝取点**而不是给一个乱动的值。
///
/// ⚠️ `w0` 的方向不能写反：写成 `ray_o - point` 会让结果的符号整体反掉，
/// 而"拖动方向相反"在编辑器里很难立刻看出是公式错还是轴反了。
pub fn axis_param(ray_o: Vec3, ray_d: Vec3, point: Vec3, axis: Vec3) -> Option<f32> {
    let v = ray_d.normalize_or_zero();
    let u = axis.normalize_or_zero();
    if v == Vec3::ZERO || u == Vec3::ZERO {
        return None;
    }
    let w0 = point - ray_o;
    let b = u.dot(v);
    let d = u.dot(w0);
    let e = v.dot(w0);
    let denom = 1.0 - b * b;
    if denom.abs() < PARALLEL_EPS {
        return None;
    }
    Some((b * e - d) / denom)
}

/// 判定"射线与轴平行"的分母阈值：`1 - b² = sin²θ`，取 1e-4 约合夹角 0.57°。
/// 比这更小就算平行 —— 此时继续拖动只会让物体以巨大倍率乱跳。
const PARALLEL_EPS: f32 = 1e-6;

/// 一次拖动：记住按下时在轴上的参数与当时的平移，之后每次移动都**从起点重算**。
///
/// 为什么从起点重算而不是累加：累加会把每次取点的数值偏差累积起来（特别是接近平行时），
/// 而"从起点重算"永远只受当前这一帧的精度影响 —— 松手时的位置与手的手感一致。
#[derive(Debug, Clone, Copy)]
pub struct TranslateDrag {
    axis: Axis,
    start_t: f32,
    start_translation: Vec3,
}

impl TranslateDrag {
    /// 按下时调用；射线与轴太平时返回 `None`（不开始拖动）。
    pub fn begin(
        axis: Axis,
        entity_translation: Vec3,
        ray_o: Vec3,
        ray_d: Vec3,
    ) -> Option<Self> {
        let start_t = axis_param(ray_o, ray_d, entity_translation, axis.dir())?;
        Some(TranslateDrag {
            axis,
            start_t,
            start_translation: entity_translation,
        })
    }

    /// 移动时调用；返回**新的平移**（不是增量，调用方直接写回 Transform）。
    pub fn drag(&self, ray_o: Vec3, ray_d: Vec3) -> Option<Vec3> {
        let t = axis_param(ray_o, ray_d, self.start_translation, self.axis.dir())?;
        Some(self.start_translation + self.axis.dir() * (t - self.start_t))
    }

    pub fn axis(&self) -> Axis {
        self.axis
    }
}



/// 相机视角快照（取点与 gizmo 都要它；**只读**，避免把整个 Camera 塞进插件上下文）。
#[derive(Debug, Clone, Copy)]
pub struct ViewInfo {
    pub eye: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// 垂直半视角（弧度）。
    pub half_fov_y: f32,
    /// 宽/高。
    pub aspect: f32,
}

impl ViewInfo {
    /// 用当前视角做一次取点。
    pub fn pick(
        &self,
        scene: &crate::engine::scene::Scene,
        ndc_x: f32,
        ndc_y: f32,
        radius: f32,
    ) -> Option<(crate::engine::scene::EntityId, f32)> {
        pick_entity(
            scene,
            self.eye,
            self.forward,
            self.right,
            self.up,
            self.half_fov_y,
            self.aspect,
            ndc_x,
            ndc_y,
            radius,
        )
    }
}

/// 屏幕坐标 → 世界射线（编辑器取点的第一步）。
///
/// `ndc_x/ndc_y` 取 **[-1, 1]**（屏幕中心 = 0,0；y 向上）；`half_fov_y` 是**垂直**半视角（弧度），
/// `aspect = 宽/高`。返回 `(origin, direction)`，方向已归一化。
///
/// 为什么不用投影矩阵求逆：逆矩阵算错时**画面一切正常**，只有取点偏移 —— 而且偏移量随 FOV
/// 变化，很难看出是数学错。用基向量直接拼，每一项都能手算验证（判据就是手算值）。
pub fn screen_ray(
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    half_fov_y: f32,
    aspect: f32,
    ndc_x: f32,
    ndc_y: f32,
) -> (Vec3, Vec3) {
    let t = half_fov_y.tan();
    // 屏幕右上角对应的偏移：垂直分量按 tan(fov/2)，水平再乘宽高比
    let dir = forward + right * (ndc_x * t * aspect) + up * (ndc_y * t);
    (eye, dir.normalize_or_zero())
}

/// 射线与以 `center` 为球心、`radius` 为半径的球求交，返回**最近的正向参数 t**。
///
/// 用球而不是 AABB：场景层目前还没有"每个实体多大"的组件（那是 P4/P3 的活），
/// 球用一个半径就能描述，且**旋转无关** —— 现在实体只有平移，球与 AABB 的差别看不出来，
/// 等实体带上包围盒时再换成盒判据（到时要重写这条判据，不是改参数）。
pub fn ray_sphere(ray_o: Vec3, ray_d: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let d = ray_d.normalize_or_zero();
    if d == Vec3::ZERO {
        return None;
    }
    let m = ray_o - center;
    let b = m.dot(d);
    let c = m.length_squared() - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    // 先取较小的根（近交点）；两个都为负 = 球在身后。
    let t0 = -b - sq;
    let t1 = -b + sq;
    if t0 > 0.0 {
        Some(t0)
    } else if t1 > 0.0 {
        Some(t1)
    } else {
        None
    }
}

/// 世界点 → NDC（`screen_ray` 的逆）。
///
/// 只用于 gizmo 把手的屏幕投影；`forward` 分量 ≤ 0（点在相机背后）时返回 `None` ——
/// 那时候投影会翻到屏幕另一侧，不判掉就会"选到背后的那根轴"。
pub fn project_to_ndc(view: &ViewInfo, p: Vec3) -> Option<(f32, f32)> {
    let v = p - view.eye;
    let z = v.dot(view.forward);
    if z <= 1e-4 {
        return None;
    }
    let t = view.half_fov_y.tan();
    Some((
        (v.dot(view.right) / z) / (t * view.aspect.max(1e-4)),
        (v.dot(view.up) / z) / t,
    ))
}

/// 光标到线段的距离（NDC 空间，直接当"屏幕距离"用）。
fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (vx, vy) = (bx - ax, by - ay);
    let (wx, wy) = (px - ax, py - ay);
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 1e-9 {
        ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (dx, dy) = (wx - vx * t, wy - vy * t);
    (dx * dx + dy * dy).sqrt()
}

/// gizmo 把手长度（米）。屏幕上大约 1m 长；太短点不中，太长会盖住整屏。
pub const GIZMO_LEN: f32 = 1.0;
/// 命中阈值（NDC）。NDC 的 1 个单位 = 半个屏幕宽，0.06 约等于"离把手 3% 屏宽以内"。
pub const GIZMO_PICK_EPS: f32 = 0.06;

/// 光标落在哪根轴上（NDC 空间里离投影线段最近、且够近的那根）。
///
/// 为什么必须在**屏幕空间**判：三根轴在世界里互相垂直，但在屏幕上可能几乎重合
/// （正对着某根轴看的时候，那根轴退化成一点）。在世界里判"离哪条线近"会退化成随机。
///
/// 返回 `None` = 没抓住任何把手（此时应当当作"点空白处"处理，而不是挑一根最近的）。
pub fn pick_axis_at_screen(entity: Vec3, view: &ViewInfo, ndc_x: f32, ndc_y: f32) -> Option<Axis> {
    let origin = project_to_ndc(view, entity)?;
    let mut best: Option<(Axis, f32)> = None;
    for axis in [Axis::X, Axis::Y, Axis::Z] {
        let Some(tip) = project_to_ndc(view, entity + axis.dir() * GIZMO_LEN) else {
            continue;
        };
        let d = dist_to_segment(ndc_x, ndc_y, origin.0, origin.1, tip.0, tip.1);
        if d <= GIZMO_PICK_EPS && best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((axis, d));
        }
    }
    best.map(|(a, _)| a)
}

/// 投影一次取点：屏幕 NDC → 射线 → 场景里最近的实体（按球判据）。
///
/// `radius` 是**取点半径**（米），不是实体的真实尺寸 —— 场景层还没有包围盒组件。
/// 放 0.9m 是为了"点得中"：太小要像素级对准，太大相邻实体会被抢走。
///
/// 返回 `(实体, 沿射线的距离 t)`；距离给编辑器做"由近到远"的调试显示。
pub fn pick_entity(
    scene: &crate::engine::scene::Scene,
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    half_fov_y: f32,
    aspect: f32,
    ndc_x: f32,
    ndc_y: f32,
    radius: f32,
) -> Option<(crate::engine::scene::EntityId, f32)> {
    let (o, d) = screen_ray(eye, forward, right, up, half_fov_y, aspect, ndc_x, ndc_y);
    let mut best: Option<(crate::engine::scene::EntityId, f32)> = None;
    for (id, local) in scene.iter_alive() {
        if let Some(t) = ray_sphere(o, d, local.translation, radius) {
            if best.map(|(_, bt)| t < bt).unwrap_or(true) {
                best = Some((id, t));
            }
        }
    }
    best
}

/// 鼠标式拖拽的完整一步（编辑器左键按下 → 移动 → 抬手 的那条链路）。
///
/// 输入是**两次光标位置**（NDC）与实体当前位置；输出是拖动后的新平移。
/// 这条函数存在的意义：鼠标路径与 AI/脚本路径**共用同一份实现**，
/// 不会出现"鼠标能拖、接口拖出来不一样"的两套行为（铁律 4 的另一面）。
///
/// 返回 `None` 的三种情况都必须当作"没拖动"处理，不许猜：
/// 光标没落在把手上、影子与轴近平行、相机背后的把手。
pub fn mouse_drag_translate(
    view: &ViewInfo,
    entity: Vec3,
    from_ndc: (f32, f32),
    to_ndc: (f32, f32),
) -> Option<(Axis, Vec3)> {
    let axis = pick_axis_at_screen(entity, view, from_ndc.0, from_ndc.1)?;
    let (o0, d0) = screen_ray(
        view.eye,
        view.forward,
        view.right,
        view.up,
        view.half_fov_y,
        view.aspect,
        from_ndc.0,
        from_ndc.1,
    );
    let drag = TranslateDrag::begin(axis, entity, o0, d0)?;
    let (o1, d1) = screen_ray(
        view.eye,
        view.forward,
        view.right,
        view.up,
        view.half_fov_y,
        view.aspect,
        to_ndc.0,
        to_ndc.1,
    );
    drag.drag(o1, d1).map(|p| (axis, p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::scene::{EntityId, Scene, Transform};

    fn approx(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    /// 🔴 轴参数的**手算值**。射线从 (1,0,10) 朝 -Z 打，X 轴过原点：
    /// 最近点是 (1,0,0) ⇒ `t = 1`。
    ///
    /// 反向验证：把 `w0` 写成 `ray_o - point` ⇒ 这条得 −1，立刻红。
    #[test]
    fn gizmo_axis_param_matches_hand_value() {
        let t = axis_param(
            Vec3::new(1.0, 0.0, 10.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::ZERO,
            Axis::X.dir(),
        )
        .expect("不平行");
        assert!((t - 1.0).abs() < 1e-4, "X 轴参数应为 1.0，实际 {t}");
        // 同一条射线对 Y 轴：最近点是原点 ⇒ t = 0
        let ty = axis_param(
            Vec3::new(1.0, 0.0, 10.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::ZERO,
            Axis::Y.dir(),
        )
        .expect("不平行");
        assert!(ty.abs() < 1e-4, "Y 轴参数应为 0，实际 {ty}");
    }

    /// 射线与轴平行时必须**拒绝**（返回 None），不能返回一个乱跳的数。
    #[test]
    fn gizmo_parallel_ray_is_rejected() {
        let r = axis_param(
            Vec3::new(0.0, 0.0, 10.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::ZERO,
            Axis::Z.dir(), // 与射线同向
        );
        assert!(r.is_none(), "平行射线的轴参数无意义，必须拒绝，实际 {r:?}");
    }

    /// 🔴 **模拟拖动 1.0 米 ⇒ 平移增量恰好 1.0**（目标里的验收口径）。
    ///
    /// 按下：射线 (0,0,10)→-Z，实体在原点 ⇒ t0 = 0；
    /// 移动：射线 (1,0,10)→-Z ⇒ t1 = 1；
    /// ⇒ 新平移必须是 (1,0,0)，误差 < 1e-4。
    #[test]
    fn gizmo_drag_moves_exactly_the_axis_delta() {
        let start = TranslateDrag::begin(
            Axis::X,
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 10.0),
            Vec3::new(0.0, 0.0, -1.0),
        )
        .expect("应能开始拖动");
        let moved = start
            .drag(Vec3::new(1.0, 0.0, 10.0), Vec3::new(0.0, 0.0, -1.0))
            .expect("应能拖");
        assert!(
            approx(moved, Vec3::new(1.0, 0.0, 0.0)),
            "拖动 1m 后平移应为 (1,0,0)，实际 {moved:?}"
        );
        // 二次拖动从起点重算：再拖回 t=0 必须回到原处（证明不是累加）
        let back = start
            .drag(Vec3::new(0.0, 0.0, 10.0), Vec3::new(0.0, 0.0, -1.0))
            .expect("应能拖");
        assert!(approx(back, Vec3::ZERO), "拖回原点应回到 (0,0,0)，实际 {back:?}");
    }

    /// 🔴 屏幕中心必须打到正前方的东西；偏出去必须打空。
    ///
    /// 相机在 (0,0,10) 朝 -Z，垂直半视角 45°：NDC (0,0) 的射线就是 (0,0,-1)，
    /// 正对原点处的球 ⇒ 命中；NDC (0.9, 0) 偏出 40° 以上 ⇒ 打空。
    ///
    /// 反向验证：把 `screen_ray` 里 `forward + right*(...)` 的 `right` 与 `up` 互换 ⇒ 红。
    #[test]
    fn screen_ray_hits_dead_center_and_misses_when_offset() {
        let eye = Vec3::new(0.0, 0.0, 10.0);
        let (o, d) = screen_ray(
            eye,
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::X,
            Vec3::Y,
            std::f32::consts::FRAC_PI_4,
            16.0 / 9.0,
            0.0,
            0.0,
        );
        assert!(approx(o, eye));
        assert!(approx(d, Vec3::new(0.0, 0.0, -1.0)), "屏幕中心必须是正前方：{d:?}");
        // 🔴 **必须用非零 NDC**：NDC=(0,0) 时 `right`/`up` 的系数都是 0，
        // 于是"把 right 和 up 写反"这条最典型的错误**测不出来**（本判据第一版就是这样，
        // 注入交换后仍然绿 —— 恒真的断言等于没写，见仓库教训 14）。
        // 半视角 45°（tan=1）、aspect=1 时，NDC=(1,0) 的方向正比于 forward+right。
        let (_, dx) = screen_ray(
            eye,
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::X,
            Vec3::Y,
            std::f32::consts::FRAC_PI_4,
            1.0,
            1.0,
            0.0,
        );
        assert!(dx.x > 0.5, "NDC x=+1 必须偏 +X，实际 {dx:?}");
        assert!(dx.y.abs() < 1e-5, "NDC y=0 时不许有 Y 分量，实际 {dx:?}");
        let (_, dy) = screen_ray(
            eye,
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::X,
            Vec3::Y,
            std::f32::consts::FRAC_PI_4,
            1.0,
            0.0,
            1.0,
        );
        assert!(dy.y > 0.5, "NDC y=+1 必须偏 +Y，实际 {dy:?}");
        assert!(dy.x.abs() < 1e-5, "NDC x=0 时不许有 X 分量，实际 {dy:?}");
        let mut s = Scene::new();
        let id = s.spawn(None, Transform::from_translation(Vec3::ZERO));
        let hit = pick_entity(&s, eye, Vec3::new(0.0, 0.0, -1.0), Vec3::X, Vec3::Y,
            std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.0, 0.0, 0.9);
        assert_eq!(hit.map(|(h, _)| h), Some(id), "中心应命中原点实体");
        let miss = pick_entity(&s, eye, Vec3::new(0.0, 0.0, -1.0), Vec3::X, Vec3::Y,
            std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.95, 0.0, 0.9);
        assert!(miss.is_none(), "偏出屏幕外的射线不该命中，实际 {miss:?}");
    }

    /// 同一视线穿过两个球时，必须选中**近的那个**（远的那栋楼不能抢走点击）。
    #[test]
    fn pick_takes_the_nearest_entity() {
        let mut s = Scene::new();
        let near = s.spawn(None, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)));
        let _far = s.spawn(None, Transform::from_translation(Vec3::new(0.0, 0.0, -5.0)));
        let hit = pick_entity(&s, Vec3::new(0.0, 0.0, 10.0), Vec3::new(0.0, 0.0, -1.0),
            Vec3::X, Vec3::Y, std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.0, 0.0, 0.9);
        assert_eq!(hit.map(|(h, _)| h), Some(near), "应选中近处实体");
    }

    /// 球在相机背后时必须打空（不能因为 `disc >= 0` 就返回一个负 t）。
    #[test]
    fn ray_sphere_ignores_spheres_behind_the_camera() {
        let t = ray_sphere(Vec3::new(0.0, 0.0, 10.0), Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, 20.0), 0.9);
        assert!(t.is_none(), "身后的球不该命中，实际 {t:?}");
    }

    /// 🔴 光标落在 X 把手附近 ⇒ 选中 X；落在空白处 ⇒ `None`（不许挑一根最近的）。
    ///
    /// 相机在原点朝 -Z，实体在 (0,0,-3)。X 把手从 (0,0,-3) 伸到 (1,0,-3)，
    /// 投影后是屏幕上一段向右的线；Y 把手向上、Z 把手朝相机（投影成很短的一段）。
    ///
    /// 反向验证：把 `pick_axis_at_screen` 里的 `d <= GIZMO_PICK_EPS` 去掉（永远取最近）
    /// ⇒ "落在空白处"那条断言红。
    #[test]
    fn gizmo_axis_pick_uses_screen_distance_and_rejects_far_clicks() {
        let view = ViewInfo {
            eye: Vec3::ZERO,
            forward: Vec3::new(0.0, 0.0, -1.0),
            right: Vec3::X,
            up: Vec3::Y,
            half_fov_y: std::f32::consts::FRAC_PI_4,
            aspect: 1.0,
        };
        let entity = Vec3::new(0.0, 0.0, -3.0);
        // X 把手中点投影
        let mid = project_to_ndc(&view, entity + Vec3::X * (GIZMO_LEN * 0.5)).unwrap();
        assert_eq!(
            pick_axis_at_screen(entity, &view, mid.0, mid.1),
            Some(Axis::X),
            "把手中点必须选中 X（投影 {mid:?}）"
        );
        // 远离所有把手
        assert_eq!(
            pick_axis_at_screen(entity, &view, 0.85, 0.85),
            None,
            "空白处不许挑一根最近的"
        );
        // 相机背后的把手不许参与：**视线方向不变**，把实体放到 +Z（身后）
        assert_eq!(
            pick_axis_at_screen(Vec3::new(0.0, 0.0, 3.0), &view, 0.0, 0.0),
            None,
            "相机背后的把手必须被拒绝（投影会翻到屏幕另一侧）"
        );
    }

    /// 🔴 **鼠标拖拽整条链路**：抓住 Y 把手往屏幕上方拖 ⇒ 平移沿 +Y 增加。
    ///
    /// 这是目标里"拖拽 3D 物体"的验收：从光标位置出发，经选轴 → 起拖 → 落到新平移。
    /// 反向验证：`mouse_drag_translate` 里把 `pick_axis_at_screen` 换成固定 `Axis::X`
    /// ⇒ 断言"轴必须是 Y"红。
    #[test]
    fn gizmo_mouse_drag_moves_along_the_picked_axis() {
        let view = ViewInfo {
            eye: Vec3::ZERO,
            forward: Vec3::new(0.0, 0.0, -1.0),
            right: Vec3::X,
            up: Vec3::Y,
            half_fov_y: std::f32::consts::FRAC_PI_4,
            aspect: 1.0,
        };
        let entity = Vec3::new(0.0, 0.0, -3.0);
        let from = project_to_ndc(&view, entity + Vec3::Y * (GIZMO_LEN * 0.5)).unwrap();
        // 往上拖 0.2 NDC
        let to = (from.0, from.1 + 0.2);
        let (axis, moved) =
            mouse_drag_translate(&view, entity, from, to).expect("应抓住 Y 把手并拖动");
        assert_eq!(axis, Axis::Y, "抓到的必须是 Y 把手");
        assert!(moved.y > entity.y + 0.05, "往屏幕上方拖必须抬高物体：{moved:?}");
        assert!(
            (moved.x - entity.x).abs() < 1e-4 && (moved.z - entity.z).abs() < 1e-4,
            "其它两轴不许动：{moved:?}"
        );
    }

    /// 斜视角下也必须是 1.0：射线从 (3,4,10) 朝 -Z 附近偏一点，X 轴过原点。
    /// 用两条只差 X=1 的平行射线，差值恒为 1（与相机位置无关）——这就是"投影到轴"的意义。
    #[test]
    fn gizmo_drag_is_view_independent() {
        let d = Vec3::new(-0.1, -0.2, -1.0);
        let a = axis_param(Vec3::new(3.0, 4.0, 10.0), d, Vec3::ZERO, Axis::X.dir()).unwrap();
        let b = axis_param(Vec3::new(4.0, 4.0, 10.0), d, Vec3::ZERO, Axis::X.dir()).unwrap();
        assert!(
            (b - a - 1.0).abs() < 1e-4,
            "两条只差 1m 的平行射线在 X 轴上应差 1.0，实际 {}",
            b - a
        );
    }
}
