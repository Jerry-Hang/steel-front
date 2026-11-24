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

#[cfg(test)]
mod tests {
    use super::*;

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
