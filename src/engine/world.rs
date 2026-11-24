//! 静态世界几何：**引擎与玩法共用的数据类型**（引擎化 P1a，2026-11-25）。
//!
//! 为什么单独成模块：`MapObstacle` / `ObstacleKind` 原来定义在 `engine::game` 里，
//! 而渲染器也要读它们 ⇒ **引擎目录只能反过来依赖玩法目录**（`renderer.rs` 曾有一行
//! `use crate::engine::game::{...}`）。把类型提到这一层，依赖方向就正过来了：
//! 玩法可以依赖世界几何，渲染器也可以依赖世界几何，两者互不相识。
//!
//! ⚠️ 这里**只放类型**。`MapObstacle` 的构造方法（`new` / `shaped` / `geom`）仍留在
//! `engine/game/types.rs` —— 它们要用玩法的 `obstacle_max_hp` 与 `MAP_BLOCK_HEIGHT`
//! （同 crate 任意模块都可以写 inherent impl）。将来把耐久/主题也提上来时再一起搬。

use crate::engine::geom::Shape;

/// 障碍种类：决定摆放形态与尺寸（渲染侧 marker 颜色由 main.rs 按 kind 映射）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObstacleKind {
    /// 墙体：1..=3 个中等盒子沿切线并排成墙（第一关基准形态）
    Wall,
    /// 大块：1..=2 个大尺寸独立方块（密度高、体型大）
    Block,
    /// 路障：2..=4 个长条薄墙（单簇更长、更稀疏）
    Barrier,
    /// 树木：场景装饰（安全环外），细长圆柱+树冠方块，可碰撞可击穿
    Tree,
    /// 建筑物：场景装饰（安全环外），大尺寸方块，可碰撞高耐久
    Building,
    /// 残骸：场景装饰（安全环外），矮小散落方块群
    Ruin,
}


/// 地图静态障碍盒（AABB：世界坐标中心 (x, y, z) + 半尺寸 half_w/half_h/half_d）。
/// 贴地障碍 y = half_h；城市建筑/树冠可抬升 y（如树冠 3.4m）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapObstacle {
    pub x: f32,
    pub z: f32,
    pub half_w: f32,
    pub half_d: f32,
    /// 盒中心高度（米；贴地障碍 = half_h）
    pub y: f32,
    /// 半高（米；贴地障碍默认 1.2 = 原 MAP_BLOCK_HEIGHT/2）
    pub half_h: f32,
    /// 障碍种类（第一关全部为 Wall；主题轮换见 theme_for_level）
    pub kind: ObstacleKind,
    /// 材质 tint 覆盖（城市手绘地图用：消防栓红/树冠绿/玻璃蓝等；None = 按种类默认色）
    pub tint: Option<[f32; 3]>,
    /// 血量上限（按种类，见 obstacle_max_hp）
    pub max_hp: f32,
    /// 当前血量（归 0 → 摧毁：从物理刚体/AI 网格/渲染 marker 中移除）
    pub hp: f32,
    /// 几何形状（**只管渲染模板**）。默认 [`Shape::Legacy`] = 旧行为（立方体，
    /// 绿色 tint 兜底成二十面体），所以既有构造点零改动即保持原画面。
    ///
    /// ⚠ 不参与碰撞：碰撞体始终是这条障碍的 AABB，与 shape 无关。
    /// 详见 `Shape::inscribed_radius_factor` 上那段"尚未接线"的说明。
    pub shape: Shape,
}



