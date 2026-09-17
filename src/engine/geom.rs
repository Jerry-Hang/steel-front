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
///
/// ⚠ 这是**与 GPU 共享的线格式**，不是普通的内部枚举：取值写在 `InstanceData.tint.w`
/// 里，`build.rs` 的两条顶点路径按**区间**（1.5~2.5 圆柱、3.5~4.5 球…）解码它。
/// 因此删变体可以，**改剩下变体的数值不行**。
///
/// 2026-09-08 清理：删掉 `Box`（tag 0.0）与 `Ico`（tag 3.0）两个变体——CPU 侧从来没有
/// 任何构造点会产出它们（盒子走 [`Shape::Legacy`]，圆树冠走 [`Shape::Sphere`]，
/// 二十面体被一级细分的 Sphere 取代）。这两个 tag 值就此作废，着色器里对应的分支变成
/// 永不命中的兜底，保留不动（顶点管线已冻结，只为兼容性存在）。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// 竖直单位圆柱（r=1、y∈[-0.5,0.5]、24 段含上下盖；50 顶点 / 96 三角）。
    /// 实例矩阵的 xz 缩放 = 半径，y 缩放 = 半高。
    Cylinder,
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
    /// 未迁移的旧构造点：画成立方体，但保留 tint 颜色嗅探兜底（绿色→树冠）。
    Legacy,
}

impl Shape {
    pub const TAG_CYLINDER: f32 = 2.0;
    pub const TAG_SPHERE: f32 = 4.0;
    /// 只碰撞不绘制。5.0 是 tint.w 历史上从未出现过的取值（旧数据只有 1.0 与显式 2/4）。
    pub const TAG_NONE: f32 = 5.0;
    /// 外部建模网格。片元看到它就不做任何程序化立面加工。
    pub const TAG_AUTHORED: f32 = 6.0;
    /// 历史默认值。GPU 侧按立方体处理，但额外允许旧的绿色→二十面体兜底。
    /// 立方体的 tag 取 1.0 而不是 0.0：0.0 在 tint.w 上曾是"未初始化"的观感，
    /// 而 1.0 是 `WorldMarker` 字面量里已经写死的那批（`tint: [r, g, b, 1.0]`）。
    pub const TAG_LEGACY: f32 = 1.0;

    /// 写入 `InstanceData.tint.w` 的标签值。
    pub const fn tag(self) -> f32 {
        match self {
            Shape::Cylinder => Shape::TAG_CYLINDER,
            Shape::Sphere => Shape::TAG_SPHERE,
            Shape::None => Shape::TAG_NONE,
            Shape::Authored => Shape::TAG_AUTHORED,
            Shape::Legacy => Shape::TAG_LEGACY,
        }
    }

    /// 标签值 → 形状。未知/越界值一律退回 [`Shape::Legacy`]，让 GPU 侧的兜底分支
    /// 去处理，而不是在这里发明新语义。
    ///
    /// `#[cfg(test)]`：CPU 侧从不反读 tint.w（`renderer.rs` 只写不读），所以这个解码器
    /// 唯一的作用是配合 [`Shape::tag`] 钉住线格式，防止有人改数值把 GPU 分支错位。
    #[cfg(test)]
    pub const fn from_tag(v: f32) -> Shape {
        // f32 精确比较：标签只由 tag() 写入，取值是 1/2/4/5/6 这些可精确表示的小整数。
        if v == Shape::TAG_CYLINDER {
            Shape::Cylinder
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

    /// 几何模板在该轴上的**半幅**（按轴：0=x / 1=y / 2=z）。实例缩放 = 想要的半尺寸 ÷ 它，
    /// 于是**画出来的尺寸恒等于碰撞 AABB**。
    ///
    /// ## 为什么必须有它（2026-09-17）
    /// 模板不是单位盒：立方体/球是 **±1**（半幅 1.0），圆柱是 **r=1、y∈[−0.5, 0.5]**
    /// （xz 半幅 1.0、y 半幅 0.5）。`WorldMarker::for_obstacle` 此前对三个轴一律写
    /// `2*half` ⇒ **所有程序化构件画出来是设计尺寸的 2 倍**，而圆柱的高度恰好是对的、
    /// 直径错 2 倍 —— 这种"部分正确"的约定比全错更难被发现，它存活了三周。
    ///
    /// 实测后果（出生点机位 `RV3D_DUMP_NEAR` 逐件对表）：
    /// · 路缘石设计 0.55 宽 × 0.21 高 ⇒ 画成 1.1 m 宽、0.42 m 高的矮墙；
    /// · 柱廊柱设计 φ0.62 / 柱头 φ0.92 ⇒ 画成 φ1.24 / φ1.84（柱头读作悬空圆盘）；
    /// · 灌木球设计 φ2.86 ⇒ 画成 φ5.7 的巨石；
    /// · 中央隔离带三段式的**宽度**全部翻倍 ⇒ 读作一条 U 形槽而不是花坛；
    /// · 玩家能站进"看得见的那半个盒子"里（碰撞 AABB 只有可见尺寸的一半）= 穿模；
    /// · 子弹打不中看得见的外挑部分、却打不中看不见的部分；PT 场景按 AABB 建盒
    ///   ⇒ 光追与光栅对同一件东西的尺寸认知不一致。
    ///
    /// 🔴 **本常量唯一的消费者是 `for_obstacle`**：改这里必须同时看那里，
    /// 二者不同步就是"全城构件集体变大/变小一倍"这种一眼可见的事故。
    pub const fn template_half_extent(self, axis: usize) -> f32 {
        match self {
            // 只碰撞不绘制的 GLB 碰撞核不走 marker 模板；取 1.0 让缩放退化成半尺寸本身
            Shape::None => 1.0,
            // 单位圆柱：r=1，但 y 已经烘成 ±0.5
            Shape::Cylinder => {
                if axis == 1 {
                    0.5
                } else {
                    1.0
                }
            }
            Shape::Sphere | Shape::Authored | Shape::Legacy => 1.0,
        }
    }

    /// 该形状的**水平足迹**是否内切于它的 AABB。
    ///
    /// ⚠ **目前没有任何生产代码调用它** —— 也就是说这条几何学结论还没有接到碰撞系统上：
    /// 圆柱/球形障碍的碰撞体仍是它的 AABB，四个角上"看得见但不该挡住"的空隙仍然会把玩家
    /// 弹开。`game.rs` 里 `geom()` 的注释以前声称"碰撞足迹随形状收缩"，那是**不成立的**，
    /// 已按现状改正。要真的实现收缩，接缝在 `MapObstacle` → 物理刚体半径那一步，
    /// 属于会改变手感的改动，需要实机验证后再做。
    #[cfg(test)]
    pub const fn inscribed_radius_factor(self) -> f32 {
        match self {
            Shape::Cylinder | Shape::Sphere => core::f32::consts::FRAC_1_SQRT_2,
            // None / Authored 的足迹就是它的碰撞盒本身（GLB 的旋转 AABB），不内切
            Shape::None | Shape::Authored | Shape::Legacy => 1.0,
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
            Shape::Cylinder,
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
        //
        // 0.0 与 3.0 是 2026-09-08 作废的 Box / Ico 标签。这里把它们当垃圾值钉住：
        // 若有人重新分配这两个 tag，本测试会失败，逼他去看 build.rs 里仍然存在的
        // m_cyl/m_ico 区间分支，而不是神不知鬼不觉地复用出第三种语义。
        for v in [-1.0, 7.0, 1.5, core::f32::consts::PI, 4.5, 0.0, 3.0] {
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

    /// 模板半幅：立方体/球 ±1，圆柱 r=1 但 y 只有 ±0.5，GLB 碰撞核退化成 1。
    ///
    /// 这条锁的是 `for_obstacle` 的缩放推导的输入。数值错了不会崩、不会报 VUID，
    /// 只会让全城构件集体变大或变小一倍（`renderer.rs::marker_visible_size_matches_aabb`
    /// 是端到端的那一条，两条要一起看）。
    #[test]
    fn template_half_extent_matches_the_rendered_templates() {
        for axis in 0..3 {
            // CUBE_POS / SPH_POS 都是 ±1
            assert_eq!(Shape::Legacy.template_half_extent(axis), 1.0);
            assert_eq!(Shape::Sphere.template_half_extent(axis), 1.0);
            // GLB 碰撞核不绘制：取 1.0 使缩放退化成半尺寸本身
            assert_eq!(Shape::None.template_half_extent(axis), 1.0);
        }
        // 单位圆柱：xz 半径 1.0，y 已烘成 ±0.5
        assert_eq!(Shape::Cylinder.template_half_extent(0), 1.0);
        assert_eq!(Shape::Cylinder.template_half_extent(1), 0.5);
        assert_eq!(Shape::Cylinder.template_half_extent(2), 1.0);
    }

    #[test]
    fn round_shapes_inscribe_their_aabb() {
        assert_eq!(Shape::Legacy.inscribed_radius_factor(), 1.0);
        assert!(Shape::Cylinder.inscribed_radius_factor() < 1.0);
        assert!(Shape::Sphere.inscribed_radius_factor() > 0.7);
        // GLB 的碰撞核与外部建模件不得被内切收缩——它们的盒子就是它本身的足迹。
        assert_eq!(Shape::None.inscribed_radius_factor(), 1.0);
        assert_eq!(Shape::Authored.inscribed_radius_factor(), 1.0);
    }
}
