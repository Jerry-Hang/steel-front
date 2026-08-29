//! 全景路径追踪基准（2026-08-29 立项）：以 RT core 的真实光照作为「记录数据」与
//! 后续光照烘焙的参照；特定位置/室内场景启用完整路径追踪。
//!
//! 设计（阶段化）：
//! 阶段1（基础）：启用 RT 扩展（renderer.rs 已加：AS/ray_query/RT-pipeline）——完成；
//! 阶段2（AS 构建）：从场景实例数据构建 BLAS（地面/障碍/建筑盒体 + 枪模网格）+ TLAS；
//! 阶段3（计算通道）：compute + rayQueryEXT（无需 RT pipeline，RT core 直接加速 BVH 遍历）：
//!   主射线 → 太阳直射 + 环境半球 + 1-2 次漫反射反弹 + 阴影射线（AO 真实获取）；
//! 阶段4（采集）：每帧/每 N 帧输出 PT 图像 → screenshots/pt_ref.png（记录数据）；
//! 阶段5（应用）：以 PT 结果为参照烘焙光照贴图/AO/间接光；室内场景按需完整 PT 每帧。
//!
//! 渲染模型（与现状关键差异）：
//! - 现状：栅格 Blinn-Phong + PCF 阴影 + radiance 被 clamp(≤1.0) ——「不亮不暗」根因；
//! - PT：物理正确路径积分（Kajiya）——亮暗随真实能量，无 clamp；
//! - 参照物：以 PT 帧作为「真理帧」，用于日后校准栅格光照/AO 烘焙。

/// 阶段2 的输入：场景盒体集合（AABB 中心/半宽；地面特殊大盒）
#[derive(Debug, Clone, Copy)]
pub struct PtBox {
    pub center: [f32; 3],
    pub half: [f32; 3],
    /// 材质：0=地面 1=混凝土 2=金属 3=树冠
    pub material: u32,
}

/// 阶段3 的基准着色参数（与游戏光照同语义，便于对比）
pub const PT_SUN_DIR: [f32; 3] = [-0.4, 0.9, -0.3];
pub const PT_SUN_COLOR: [f32; 3] = [1.0, 0.95, 0.85];
pub const PT_SUN_INTENSITY: f32 = 1.5;
pub const PT_AMBIENT_COLOR: [f32; 3] = [0.5, 0.55, 0.6];
pub const PT_AMBIENT_INTENSITY: f32 = 0.5;
