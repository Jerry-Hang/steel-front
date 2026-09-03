//! 实例形状标签（渲染几何与碰撞共享的"这个物体是什么形状"）。
//!
//! ## 为什么需要它
//! 2026-09-01 之前，marker 的形状不是数据，而是**从颜色猜出来的**：`build.rs` 的
//! `is_foliage(tint)`（g 显著大于 r/b）把绿色障碍渲染成二十面体，其余一律立方体。
//! 后果是"想要圆的就不能有颜色"——树干、灯杆、桶、穹顶、沙袋堆全都只能拿盒子凑，
//! 整座 5×5 街区城市因此只有 43 个盒子可画，观感是模型沙盘而不是城市。
//!
//! ## 怎么生效
//! 标签写在 `InstanceData.tint.w` 里（该分量此前对 marker 恒为 1.0，且片元只用
//! `tint.rgb`，所以是现成的空闲位）。选它而不是扩 stride 的原因：实例 buffer 的
//! stride/容量/`MARKER_SLOT_BASE..GUN_INSTANCE_INDEX` 一整套槽位算术都由 80 字节
//! 推导，改 stride 会波及 CPU 剔除、PT 材质表与传统管线；用 tint.w 则一处都不用动。
//!
//! ## 兼容性
//! [`Shape::Legacy`]（=1.0）是**未迁移构造点**的取值：行为完全等于旧规则
//! （立方体 + 绿色 tint 走二十面体兜底）。`main.rs` 里手写的掩体/自发光 marker 仍走
//! 这条路，所以本模块上线不会改变它们的画面。新代码一律显式 [`Shape`]，别再依赖颜色。
//!
//! 注意：只有 **marker 槽位带** 读这个标签。NPC 四肢/头与自发光体仍由槽位带决定形状
//! （槽位带同时决定 `flat_flag` 材质模式，两者不冲突）。

/// marker 实例的几何模板选择。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// 立方体（24 顶点 / 12 三角，逐面 0..1 UV）。
    Box,
    /// 竖直单位圆柱（r=1、y∈[-0.5,0.5]、24 段含上下盖；50 顶点 / 96 三角）。
    /// 实例矩阵的 xz 缩放 = 半径，y 缩放 = 半高。
    Cylinder,
    /// 二十面体（12 顶点 / 20 三角，归一化到半径 1）。树冠、沙袋堆、圆顶。
    Ico,
    /// 一级细分二十面体（42 顶点 / 80 三角，半径 1）。近处需要圆润的球体。
    Sphere,
    /// **只碰撞、不绘制**。用途只有一个：GLB 道具的结构碰撞核——它必须留在障碍表里
    /// （物理刚体、AI 视线、伤害结算都按下标对应），但画出来会和 GLB 表面共面打 z-fighting。
    /// 由 `main.rs` 组装 marker 时过滤掉，所以 GPU 侧永远看不到这个标签，两条渲染管线都不用改。
    None,
    /// **外部建模的网格**（Blender → GLB）。片元据此跳过全部"给纯 tint 盒子补细节"的
    /// 程序化表面效果：窗带、玻璃分格+菲涅耳、树冠值噪声、以及按"每面 0..1 UV"采样的
    /// marker 混凝土皮肤。
    ///
    /// 为什么必须有它：那四条效果只按 `flat_flag` 与顶点色的**通道比例**分类，不看几何来源。
    /// 本项目混凝土灰立面 `channelSpread < 0.14` 会被判成"中性"→ 片元按 `FLOOR_H = 3.15`
    /// 再画一层窗带；而 GLB 楼层是 3.4 m，于是每层错位 0.25 m、越往上漂得越多，
    /// 直接画在建模好的窗台与壁柱上——等于换个形式重演缺陷 D11。
    /// 皮肤纹理那条更致命：它假设 UV 是逐面 0..1，而 GLB 是世界投影 UV，会把墙纹任意铺开。
    Authored,
    /// 未迁移的旧构造点：等价于 [`Shape::Box`]，但保留 tint 颜色嗅探兜底。
    Legacy,
}

impl Shape {
    /// 立方体。0.0 而非 1.0：1.0 要留给 [`Shape::Legacy`]，那是历史数据里
    /// 已经写死的值（`WorldMarker` 字面量普遍写 `tint: [r, g, b, 1.0]`）。
    pub const TAG_BOX: f32 = 0.0;
    pub const TAG_CYLINDER: f32 = 2.0;
    pub const TAG_ICO: f32 = 3.0;
    pub const TAG_SPHERE: f32 = 4.0;
    /// 只碰撞不绘制。5.0 是 tint.w 历史上从未出现过的取值（旧数据只有 1.0 与显式 0/2/3/4）。
    pub const TAG_NONE: f32 = 5.0;
    /// 外部建模网格。片元看到它就不做任何程序化立面加工。
    pub const TAG_AUTHORED: f32 = 6.0;
    /// 历史默认值。GPU 侧按立方体处理，但额外允许旧的绿色→二十面体兜底。
    pub const TAG_LEGACY: f32 = 1.0;

    /// 写入 `InstanceData.tint.w` 的标签值。
    pub const fn tag(self) -> f32 {
        match self {
            Shape::Box => Shape::TAG_BOX,
            Shape::Cylinder => Shape::TAG_CYLINDER,
            Shape::Ico => Shape::TAG_ICO,
            Shape::Sphere => Shape::TAG_SPHERE,
            Shape::None => Shape::TAG_NONE,
            Shape::Authored => Shape::TAG_AUTHORED,
            Shape::Legacy => Shape::TAG_LEGACY,
        }
    }

    /// 标签值 → 形状。未知/越界值一律退回 [`Shape::Legacy`]，让 GPU 侧的兜底分支
    /// 去处理，而不是在这里发明新语义。
    pub const fn from_tag(v: f32) -> Shape {
        // f32 精确比较：标签只由 tag() 写入，取值是 0/1/2/3/4/5 这些可精确表示的小整数。
        if v == Shape::TAG_BOX {
            Shape::Box
        } else if v == Shape::TAG_CYLINDER {
            Shape::Cylinder
        } else if v == Shape::TAG_ICO {
            Shape::Ico
        } else if v == Shape::TAG_SPHERE {
            Shape::Sphere
        } else if v == Shape::TAG_NONE {
            Shape::None
        } else if v == Shape::TAG_AUTHORED {
            Shape::Authored
        } else {
            Shape::Legacy
        }
    }

    /// 该形状的**水平足迹**是否内切于它的 AABB。
    ///
    /// 用途是碰撞：圆柱/球在 AABB 的四个角上是"看得见但不该挡住"的空隙，把
    /// 胶囊半径按 √2 折算成内切半径，玩家才能贴到柱子边上而不被隐形方块弹开。
    pub const fn inscribed_radius_factor(self) -> f32 {
        match self {
            Shape::Cylinder | Shape::Sphere | Shape::Ico => core::f32::consts::FRAC_1_SQRT_2,
            // None / Authored 的足迹就是它的碰撞盒本身（GLB 的旋转 AABB），不内切
            Shape::Box | Shape::None | Shape::Authored | Shape::Legacy => 1.0,
        }
    }
}

impl Default for Shape {
    /// 默认 [`Shape::Legacy`]：新增字段时，所有既有的 `MapObstacle` 构造点
    /// 不需要逐个改就能保持原画面。
    fn default() -> Self {
        Shape::Legacy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_tags_round_trip() {
        for s in [
            Shape::Box,
            Shape::Cylinder,
            Shape::Ico,
            Shape::Sphere,
            Shape::None,
            Shape::Authored,
            Shape::Legacy,
        ] {
            assert_eq!(Shape::from_tag(s.tag()), s, "tag round-trip broke for {:?}", s);
        }
    }

    #[test]
    fn unknown_shape_tag_falls_back_to_legacy() {
        // 越界/浮点垃圾值不得变成"隐形的新形状"，必须退回旧行为。
        // 5.0 / 6.0 不在此列：2026-09-03 起它们分别是 None（只碰撞不绘制）与
        // Authored（外部建模，跳过程序化立面）的合法标签。
        for v in [-1.0, 7.0, 1.5, 2.71828, 4.5] {
            assert_eq!(Shape::from_tag(v), Shape::Legacy, "tag {} must be Legacy", v);
        }
    }

    #[test]
    fn gpu_side_thresholds_leave_room_for_none_and_authored() {
        // 片元/顶点着色器用 flat_flag 的**区间**分路径（>2.5 枪、>1.5 NPC、>0.5 皮肤、
        // <1.5 marker/立面加工）。Authored 走 flat_flag = 1.25，必须落在
        // "(0.5, 1.5) 之内、且不等于任何既有阈值"，否则会被 NPC 轮廓光或枪模直出抢走。
        let authored_flat = 1.25f32;
        assert!(authored_flat > 0.5 && authored_flat < 1.5);
        assert!((authored_flat - 1.0).abs() > 0.05, "不能与 marker 的 1.0 重合");
        assert_eq!(Shape::from_tag(Shape::TAG_AUTHORED), Shape::Authored);
        assert_eq!(Shape::from_tag(Shape::TAG_NONE), Shape::None);
    }

    #[test]
    fn none_shape_tag_does_not_collide_with_legacy_data() {
        // 旧构造点普遍写 tint[3] = 1.0；None 必须是**新**值，绝不能被历史数据误触发。
        assert_ne!(Shape::TAG_NONE, Shape::TAG_LEGACY);
        assert_eq!(Shape::from_tag(1.0), Shape::Legacy);
        assert_eq!(Shape::from_tag(Shape::TAG_NONE), Shape::None);
    }

    #[test]
    fn legacy_tag_is_what_old_constructors_write() {
        // main.rs / renderer.rs 里手写的 WorldMarker 一律 tint[3] = 1.0。
        // 若这个断言失败，说明有构造点开始自己写 tint.w，需要逐个复核而不是改常量。
        assert_eq!(Shape::TAG_LEGACY, 1.0);
        assert_eq!(Shape::default().tag(), 1.0);
    }

    #[test]
    fn round_shapes_inscribe_their_aabb() {
        assert_eq!(Shape::Box.inscribed_radius_factor(), 1.0);
        assert!(Shape::Cylinder.inscribed_radius_factor() < 1.0);
        assert!(Shape::Sphere.inscribed_radius_factor() > 0.7);
    }
}
