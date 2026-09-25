//! Vulkan 渲染器模块
//!
//! 使用 ash 0.38 初始化 Vulkan，渲染一个旋转的带纹理立方体。
//! 包含完整的 Vulkan 管线生命周期管理。
//! 已接入 MVP Uniform Buffer（model/view/proj）与深度缓冲。

use std::ffi::CStr;
use std::time::Instant;
use std::fs::File;
use ash::{
    ext::{debug_utils::Instance as DebugUtils, mesh_shader::Device as MeshShaderDevice},
    khr::{surface::Instance as Surface, swapchain::Device as Swapchain},
    util, vk, Device, Entry, Instance,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;
use super::lighting::{LightUniform, LIGHT_UBO_BINDING};
// 障碍 marker 材质/尺寸构建依赖游戏侧障碍定义（仅类型引用，无运行时依赖）
use crate::engine::game::{MapObstacle, ObstacleKind};

/// ash 的字符串指针类型跟随平台 c_char：x86_64/Linux 为 `*const i8`，
/// AArch64（Apple Silicon/Android/高通 X Elite）为 `*const u8`。
/// 实例/设备扩展名与层名统一走该别名，跨平台无需逐个转型。
#[cfg(target_arch = "x86_64")]
type RawCString = *const i8;
#[cfg(not(target_arch = "x86_64"))]
type RawCString = *const u8;

// ============================================================
// 数据类型
// ============================================================

/// Camera Uniform 数据（view/proj 两个 4x4 矩阵 + lod_params + 网格着色器扩展字段，256 字节）
#[repr(C)]
#[derive(Copy, Clone)]
struct CameraUniform {
    view: glam::Mat4,
    proj: glam::Mat4,
    /// (terrain_lod_high_end, fade_start, fade_end, terrain_lod_med_end)
    /// x/w：地形网格 LOD 切换距离（shader 不读这两个分量，仅 CPU 侧语义扩展）
    /// y/z：实例远档十字 quad 地面淡出区间（shader 读取，语义保持不变）
    lod_params: [f32; 4],
    /// 视锥 6 平面（Gribb–Hartmann，法线朝内、归一化）。仅网格着色器读取；
    /// 传统顶点着色器声明的 ViewProj 只读前 144 字节，本扩展字段对其透明。
    planes: [[f32; 4]; 6],
    /// xyz = 相机世界位置，w = 近档距离²（几何 LOD 切换阈值）。仅网格着色器读取。
    cam_pos: [f32; 4],
}

// 光照 Uniform 类型与布局由 lighting 模块统一维护（`lighting::LightUniform`，352 字节）。
// 默认全零 = 光照关闭：片元着色器走原「纹理+顶点颜色 50% 混合」路径，向后兼容。

/// 立方体顶点数据
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
    uv: [f32; 2],
}
// 步长契约：主管线把它当 `stride` 用（`size_of::<Vertex>()`），而**着色器把
// `location 0/1/2 = pos/color/uv` 与 32B 步长当成既成事实**（`build.rs` 的 mesh 路径
// 与 `铁律 B` 的 "stride=32" 都这么写）。加字段/加填充会让属性整体错位，
// 而 Vulkan 与驱动**都不报错**（只是画面默默变错）⇒ 在这里把它钉死。
const _: () = assert!(
    std::mem::size_of::<Vertex>() == 32,
    "Vertex 必须是 32B（pos@0/color@12/uv@24）：管线步长与着色器取属性都按它写死"
);

/// 性能快照（供性能日志系统，2026-08-16）：帧耗时与各渲染阶段耗时（微秒）
#[derive(Clone, Copy)]
pub struct PerfSnapshot {
    pub frame_us: u64,
    pub cull_us: u64,
    pub terrain_us: u64,
    pub wait_fence_us: u64,
    pub acquire_us: u64,
    pub record_us: u64,
    pub submit_us: u64,
    pub present_us: u64,
}

/// HUD 覆盖层顶点：屏幕空间 NDC 位置（Y 已翻转）+ RGBA 颜色
#[repr(C)]
#[derive(Clone, Copy)]
struct HudVertex {
    pos: [f32; 2],
    color: [f32; 4],
}
// HUD 覆盖层自己的步长契约（与主管线的 `Vertex` 无关）：`hud.vert.spv` 按
// `pos vec2 + color vec4` 取属性，步长由这里推导 ⇒ 改动同样必须在这里被挡住。
const _: () = assert!(
    std::mem::size_of::<HudVertex>() == 24,
    "HudVertex 必须是 24B（pos vec2 + color vec4）：HUD 着色器按此布局取属性"
);

/// 立方体 24 顶点（每面 4 个，CCW 外侧绕序；每面 UV 铺满 0..1）
const VERTICES: [Vertex; 24] = [
    // 前 (+Z)
    Vertex { pos: [-1.0, -1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [ 1.0, -1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [ 1.0,  1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [-1.0,  1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
    // 后 (-Z)
    Vertex { pos: [ 1.0, -1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [-1.0, -1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [-1.0,  1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [ 1.0,  1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
    // 右 (+X)
    Vertex { pos: [ 1.0, -1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [ 1.0, -1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [ 1.0,  1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [ 1.0,  1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
    // 左 (-X)
    Vertex { pos: [-1.0, -1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [-1.0, -1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [-1.0,  1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [-1.0,  1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
    // 上 (+Y)
    Vertex { pos: [-1.0,  1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [ 1.0,  1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [ 1.0,  1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [-1.0,  1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
    // 下 (-Y)
    Vertex { pos: [-1.0, -1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [ 1.0, -1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [ 1.0, -1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [-1.0, -1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
];

/// 立方体 36 索引（6 面 × 2 三角形）
const INDICES: [u32; 36] = [
     0,  1,  2,  0,  2,  3, // 前
     4,  5,  6,  4,  6,  7, // 后
     8,  9, 10,  8, 10, 11, // 右
    12, 13, 14, 12, 14, 15, // 左
    16, 18, 17, 16, 19, 18, // 上（绕序同地面 quad：从上方看是正面）
    20, 22, 21, 20, 23, 22, // 下（反向：从下方看才是正面）
];

/// 远档 LOD：十字交叉双 quad（8 顶点 / 12 索引），边长与立方体一致（±1.0）。
/// quad1 位于 XY 平面（面向 ±Z），quad2 位于 ZY 平面（面向 ±X），绕序 CCW 与立方体一致。
/// 顶点色用白色：远档实例 tint 不变，纹理/颜色混合结果与近档一致。
const FAR_VERTS: [Vertex; 8] = [
    Vertex { pos: [-1.0, -1.0,  0.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [ 1.0, -1.0,  0.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [ 1.0,  1.0,  0.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [-1.0,  1.0,  0.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
    Vertex { pos: [ 0.0, -1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [ 0.0, -1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [ 0.0,  1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [ 0.0,  1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
];
const FAR_INDICES: [u32; 12] = [
     0,  1,  2,  0,  2,  3, // XY 平面 quad
     4,  5,  6,  4,  6,  7, // ZY 平面 quad
];

/// 地面平铺 quad（4 顶点 / 6 索引）：XZ 平面 y=0、边长 ±1（2×2m，与实例网格间距一致）。
/// 地面实例专用（近档+远档共用）：几何本身无侧壁，实例矩阵纯平移，彻底消除旧版
/// 立方体/压扁薄片侧壁带来的"掀盖纸箱铺地"格子感。顶点色白，纹理/颜色混合结果
/// 与旧立方体顶面一致。
/// 绕序注意：本管线为 FrontFace::CLOCKWISE + shader Y 翻转，水平面从上方看必须是
/// 逆时针（索引 [0,2,1, 0,3,2]）才是正面。立方体顶/底面与 mesh 圆柱盖曾按相反约定
/// 绕序 —— 顶面从上方恒被背面剔除，每个 marker 盒子实际是"顶面开口的盒子"（喷泉池/
/// 花坛"坑"、柱头"管口"的根因），2026-09-19 已统一，回归判据见 `horizontal_winding_tests`。
/// marker/NPC 的垂直面不受影响。
const GROUND_VERTS: [Vertex; 4] = [
    Vertex { pos: [-1.0, 0.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [ 1.0, 0.0,  1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [ 1.0, 0.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [-1.0, 0.0, -1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
];
const GROUND_INDICES: [u32; 6] = [0, 2, 1, 0, 3, 2];

/// 距离 LOD 阈值：相机到实例中心距离 < 120 用近档几何，否则远档几何。
/// 地面实例近/远档均为平铺 quad（GROUND_VERTS）；marker/NPC/自发光近档用立方体、
/// 远档用十字双 quad（FAR_VERTS），共用不变。
const LOD_DISTANCE: f32 = 120.0;
/// 远档十字 quad 地面距离淡出区间（地平线处自然消失）
/// FADE_END=900 保证任何可达机位（|x|,|z|<=600）最近场点距离 <=486 < 900，
/// 场外不再“实例全灭”；远角 1210 > 900 仍自然淡出（地平线无硬边）。
const FADE_START: f32 = 400.0;
const FADE_END: f32 = 900.0;

/// 地面微细节层在主 descriptor set 里的绑定号。
/// **必须与 build.rs `FRAGMENT_SHADER_WGSL` 的 `@group(0) @binding(9) ground_detail_tex`
/// 同步**（0..8 已被 camera UBO / 地面烘焙图 / 实例 storage / 采样器 / 光照 UBO /
/// 阴影图 / 阴影采样器 / marker 皮肤 / NPC 皮肤占用）。
/// 该绑定是**硬依赖**：片元无条件采样它，缺绑定不会报错，只会让采样恒 0 把
/// 相机周边的地面乘成纯黑（见 `Renderer::ground_detail_image` 注释）。
const GROUND_DETAIL_BINDING: u32 = 9;

// ============================================================
// 地形常量（世界 512×512，与实例场同域）
// ============================================================
const TERRAIN_VERTS: usize = 257;
const TERRAIN_CELLS: usize = 256;
const TERRAIN_HALF: f32 = 255.0;
const TERRAIN_UV_SCALE: f32 = 32.0; // uv 铺 0..16 重复采样
/// 地形网格渲染下沉量：地面平铺 quad 抬到 +0.05、地形网格整体下沉 0.35，
/// 两层地面在深度上拉开 0.4m，杜绝远距离深度精度不足导致的 z-fighting 闪烁
/// （旧版实例场与地形几乎共面，远档顶面被深度测试剔除、只剩侧壁可见）。
const TERRAIN_RENDER_SINK: f32 = 0.35;
/// 程序化地形平坦半径（米）：覆盖中央 60×60 安全区、障碍环带 58–130m 与两军接火区
const TERRAIN_FLAT_RADIUS: f32 = 230.0; // 城市占地 ±215 需平地（2026-08-21 城市地图）
/// 平坦区外丘陵最大抬升（米，平滑抬升 × 噪声幅值，恒 ≤ 本常量）
const TERRAIN_HILL_AMPLITUDE: f32 = 15.0;
/// 丘陵抬升过渡带宽（米）：半径 140 → 320 内 smoothstep 从 0 升到满幅（起点斜率 0）
const TERRAIN_HILL_RAMP: f32 = 130.0;
/// 值噪声格距（米）：格距越大丘陵越平缓（低频滚动丘陵，LOD morph 无突兀）
const TERRAIN_HILL_CELL: f32 = 128.0;

// ---- 地形网格 LOD（3 级密度：高 257² / 中 129² / 低 65² 顶点）----
/// 各级每边格数（256 / 128 / 64），顶点数 = 格数 + 1，格间距 = 512 / 格数。
/// 粗网格顶点恰为细网格顶点子集（间距 2.0 / 4.0 / 8.0，起点同为 -255）。
const TERRAIN_LOD_CELLS: [usize; 3] = [TERRAIN_CELLS, TERRAIN_CELLS / 2, TERRAIN_CELLS / 4];
const _: () = assert!(TERRAIN_LOD_CELLS[0] + 1 == TERRAIN_VERTS);

/// 相机到地形中心地面距离的 LOD 阈值：
/// dist < TERRAIN_LOD_HIGH_END → 高级（高密度）；dist < TERRAIN_LOD_MED_END → 中级；其余低级。
const TERRAIN_LOD_HIGH_END: f32 = 110.0;
const TERRAIN_LOD_MED_END: f32 = 260.0;
/// 各级进入高度 morph 过渡带的距离起点（起点→END 之间做 smoothstep 渐变）。
const TERRAIN_LOD_HIGH_MORPH_START: f32 = 70.0;
const TERRAIN_LOD_MED_MORPH_START: f32 = 200.0;

/// 地形网格 LOD 级别（索引 0/1/2 = 高级/中级/低级，对应 TERRAIN_LOD_CELLS）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerrainLod {
    High = 0,
    Medium = 1,
    Low = 2,
}

impl TerrainLod {
    fn from_idx(idx: usize) -> TerrainLod {
        match idx {
            0 => TerrainLod::High,
            1 => TerrainLod::Medium,
            _ => TerrainLod::Low,
        }
    }
    fn cells(self) -> usize {
        TERRAIN_LOD_CELLS[self as usize]
    }
    fn verts(self) -> usize {
        TERRAIN_LOD_CELLS[self as usize] + 1
    }
    fn cell_size(self) -> f32 {
        512.0 / TERRAIN_LOD_CELLS[self as usize] as f32
    }
    fn index_count(self) -> u32 {
        (self.cells() * self.cells() * 6) as u32
    }
    fn name(self) -> &'static str {
        match self {
            TerrainLod::High => "high",
            TerrainLod::Medium => "medium",
            TerrainLod::Low => "low",
        }
    }
}

/// 纯函数：相机到地形中心地面距离 → 基础 LOD 级别
/// （默认 Medium 画质；非测试构建下仅被 #[cfg(test)] 单元测试调用）
#[allow(dead_code)]
fn terrain_lod_for_distance(dist: f32) -> TerrainLod {
    terrain_lod_for_distance_with_params(dist, quality_params(QualityPreset::DEFAULT))
}

/// 纯函数：按画质参数计算相机到地形中心地面距离 → 基础 LOD 级别
fn terrain_lod_for_distance_with_params(dist: f32, params: QualityParams) -> TerrainLod {
    if dist < params.terrain_lod_high_end {
        TerrainLod::High
    } else if dist < params.terrain_lod_med_end {
        TerrainLod::Medium
    } else {
        TerrainLod::Low
    }
}

/// 纯函数（默认 Medium 画质）：距离 → (要绘制的网格级别, morph 进度 t∈[0,1])。
/// （非测试构建下仅被 #[cfg(test)] 单元测试调用）
#[allow(dead_code)]
fn terrain_lod_blend(dist: f32) -> (TerrainLod, f32) {
    terrain_lod_blend_with_params(dist, quality_params(QualityPreset::DEFAULT))
}

/// 纯函数：按画质参数计算距离 → (要绘制的网格级别, morph 进度 t∈[0,1])。
/// t 为该级网格顶点高度向下一级（更粗）曲面三角形插值的进度：
/// t=0 完全细曲面，t=1 完全等于下一级曲面（几何重合，切换无 popping）。
fn terrain_lod_blend_with_params(dist: f32, params: QualityParams) -> (TerrainLod, f32) {
    if dist < params.terrain_lod_high_end {
        let t = ((dist - params.terrain_lod_high_morph_start)
            / (params.terrain_lod_high_end - params.terrain_lod_high_morph_start))
        .clamp(0.0, 1.0);
        (TerrainLod::High, smooth_t(t))
    } else if dist < params.terrain_lod_med_end {
        let t = ((dist - params.terrain_lod_med_morph_start)
            / (params.terrain_lod_med_end - params.terrain_lod_med_morph_start))
        .clamp(0.0, 1.0);
        (TerrainLod::Medium, smooth_t(t))
    } else {
        (TerrainLod::Low, 1.0)
    }
}

/// 画质预设（纯 CPU 侧参数：地形 LOD 切换距离 + 实例近/远档分界距离等；
/// 不触碰 pipeline/shader/swapchain 创建路径，零 VUID 风险）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityPreset {
    Low,
    Medium,
    High,
}

impl QualityPreset {
    /// 默认画质：Medium（与现有渲染行为完全一致）
    pub const DEFAULT: QualityPreset = QualityPreset::Medium;

    /// 画质显示标签（供 HUD / 日志使用）
    pub fn label(&self) -> &'static str {
        match self {
            QualityPreset::Low => "低画质",
            QualityPreset::Medium => "中画质",
            QualityPreset::High => "高画质",
        }
    }
}

/// 画质参数（纯 CPU 侧）：地形 LOD 两级切换距离与 morph 过渡带起点、实例近/远档分界距离
#[derive(Debug, Clone, Copy, PartialEq)]
struct QualityParams {
    /// 相机到地形中心距离 < 该值 → 高级（高密度）网格
    terrain_lod_high_end: f32,
    /// 距离 < 该值 → 中级网格；其余低级
    terrain_lod_med_end: f32,
    /// 高级→中级 morph 过渡带起点（起点→high_end 之间做 smoothstep 渐变）
    terrain_lod_high_morph_start: f32,
    /// 中级→低级 morph 过渡带起点
    terrain_lod_med_morph_start: f32,
    /// 实例近档/远档分界距离（近档立方体 / 远档十字 quad 的切换半径）
    instance_lod_distance: f32,
}

/// 画质参数表：Medium 与现有常量完全一致（TERRAIN_LOD_HIGH_END / TERRAIN_LOD_MED_END /
/// TERRAIN_LOD_HIGH_MORPH_START / TERRAIN_LOD_MED_MORPH_START / LOD_DISTANCE）；
/// Low 各阈值减小（更早降级、更小近档半径），High 各阈值增大（更晚降级、更大近档半径）。
const QUALITY_PARAMS: [QualityParams; 3] = [
    // Low
    QualityParams {
        terrain_lod_high_end: 80.0,
        terrain_lod_med_end: 180.0,
        terrain_lod_high_morph_start: 50.0,
        terrain_lod_med_morph_start: 140.0,
        instance_lod_distance: 90.0,
    },
    // Medium（沿用现有常量，行为不变）
    QualityParams {
        terrain_lod_high_end: TERRAIN_LOD_HIGH_END,
        terrain_lod_med_end: TERRAIN_LOD_MED_END,
        terrain_lod_high_morph_start: TERRAIN_LOD_HIGH_MORPH_START,
        terrain_lod_med_morph_start: TERRAIN_LOD_MED_MORPH_START,
        instance_lod_distance: LOD_DISTANCE,
    },
    // High
    QualityParams {
        terrain_lod_high_end: 145.0,
        terrain_lod_med_end: 340.0,
        terrain_lod_high_morph_start: 95.0,
        terrain_lod_med_morph_start: 260.0,
        instance_lod_distance: 160.0,
    },
];

/// 纯函数：画质预设 → 参数表
fn quality_params(preset: QualityPreset) -> QualityParams {
    match preset {
        QualityPreset::Low => QUALITY_PARAMS[0],
        QualityPreset::Medium => QUALITY_PARAMS[1],
        QualityPreset::High => QUALITY_PARAMS[2],
    }
}

// ============================================================
// PNG 截图（swapchain 图像读回，纯逻辑部分）
// ============================================================

/// 截图读回时主机侧等待 render_finished 信号量的超时（纳秒，2 秒足够完成一帧渲染）
const SCREENSHOT_WAIT_TIMEOUT_NS: u64 = 2_000_000_000;

/// 交换链图像获取的超时（纳秒）。**绝不可以用 `u64::MAX`**：呈现引擎不给图像时
/// （隐藏/被遮挡窗口 + IMMEDIATE 是已知诱因）主循环会静默卡死，外面只看到"日志停住"。
const ACQUIRE_TIMEOUT_NS: u64 = 1_000_000_000;

/// 检视围栏等待超时（纳秒）。5 秒足够任何一帧；超时说明 GPU 侧出了问题，要留下日志。
const FENCE_WAIT_TIMEOUT_NS: u64 = 5_000_000_000;

/// 连续 acquire 超时到第几次就**降级到 mailbox 自动恢复**（≈3 秒没图像）
const ACQUIRE_STALL_FALLBACK: u32 = 3;

/// 连续 acquire 超时到第几次就放弃这一帧并报错（≈30 秒没图像，日志里要能被看见）
const ACQUIRE_STALL_MAX: u32 = 30;

/// 连续**围栏**超时到第几次就判定"GPU 侧卡死"（3 × 5s = 15s 没有任何一帧完成）
const FENCE_STALL_MAX: u32 = 3;

/// 是否已经该判定 GPU 侧卡死（纯函数，可单测）
fn fence_stall_due(timeouts: u32) -> bool {
    timeouts >= FENCE_STALL_MAX
}

/// 像素字节序策略（由 swapchain 像素格式决定）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PixelOrder {
    /// 源为 B,G,R,A 字节序（转 RGBA 需交换 R/B）
    Bgra,
    /// 源为 R,G,B,A 字节序（直接拷贝）
    Rgba,
}

/// 纯函数：swapchain 像素格式 → 字节序策略。
/// 支持 B8G8R8A8 / R8G8B8A8 的 UNORM/SRGB 四种格式；SRGB 仅影响解码语义，
/// 存储字节序与 UNORM 相同，写 PNG 时保持原始编码；未知格式返回 Err。
fn pixel_order_for_format(format: vk::Format) -> Result<PixelOrder, String> {
    match format {
        vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB => Ok(PixelOrder::Bgra),
        vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB => Ok(PixelOrder::Rgba),
        _ => Err(format!("不支持的交换链像素格式: {:?}", format)),
    }
}

/// 纯函数：把 staging buffer 中的像素字节流（swapchain 格式字节序）转换为 RGBA8。
/// src/dst 长度必须相等且为 4 的倍数（每像素 4 字节）；未知格式返回 Err。
fn convert_pixels_to_rgba(format: vk::Format, src: &[u8], dst: &mut [u8]) -> Result<(), String> {
    if src.len() != dst.len() || src.len() % 4 != 0 {
        return Err("像素缓冲区长度非法".to_string());
    }
    match pixel_order_for_format(format)? {
        PixelOrder::Bgra => {
            for (chunk, out) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                out[0] = chunk[2];
                out[1] = chunk[1];
                out[2] = chunk[0];
                out[3] = chunk[3];
            }
        }
        PixelOrder::Rgba => dst.copy_from_slice(src),
    }
    Ok(())
}

/// 物理设备选择偏好（来自 `RV3D_GPU`）
#[derive(Debug, Clone, PartialEq, Eq)]
enum GpuPreference {
    /// 不设 `RV3D_GPU`：有窗口表面的设备里**优先独显**（本仓历史行为）
    Auto,
    Discrete,
    Integrated,
    /// 设备名包含该子串（已转小写）
    Name(String),
}

/// 解析 `RV3D_GPU`：`igpu`/`integrated` → 集显；`dgpu`/`discrete` → 独显；
/// 其它非空值 → **按设备名子串匹配**（大小写不敏感，例如 `RV3D_GPU=radeon`）；空/未设 → `Auto`。
fn parse_gpu_preference(raw: Option<&str>) -> GpuPreference {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return GpuPreference::Auto;
    };
    match s.to_ascii_lowercase().as_str() {
        "igpu" | "integrated" => GpuPreference::Integrated,
        "dgpu" | "discrete" => GpuPreference::Discrete,
        other => GpuPreference::Name(other.to_string()),
    }
}

/// 设备类型排序权重（`Auto` 用：独显 2 > 集显 1 > 其它 0）
fn gpu_type_rank(t: vk::PhysicalDeviceType) -> u8 {
    match t {
        vk::PhysicalDeviceType::DISCRETE_GPU => 2,
        vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
        _ => 0,
    }
}

/// 从候选里挑一个设备（**纯函数，可单测**）：返回下标；匹配不到返回 `None`
/// （调用方据此**报错退出**，不静默回退 —— 见调用点注释）。
fn pick_physical_device(
    candidates: &[(vk::PhysicalDeviceType, String)],
    pref: &GpuPreference,
) -> Option<usize> {
    match pref {
        GpuPreference::Discrete => candidates
            .iter()
            .position(|(t, _)| *t == vk::PhysicalDeviceType::DISCRETE_GPU),
        GpuPreference::Integrated => candidates
            .iter()
            .position(|(t, _)| *t == vk::PhysicalDeviceType::INTEGRATED_GPU),
        GpuPreference::Name(want) => candidates
            .iter()
            .position(|(_, n)| n.to_ascii_lowercase().contains(want.as_str())),
        // 与历史行为一致：`max_by_key` 取"最大的那个"，并列时取**最后一个**
        GpuPreference::Auto => candidates
            .iter()
            .enumerate()
            .max_by_key(|(_, (t, _))| gpu_type_rank(*t))
            .map(|(i, _)| i),
    }
}

/// `queue_present` 的结果分类（纯函数，可单测）。
///
/// 🔴 2026-09-22 复查补：此前只处理 `Err(ERROR_OUT_OF_DATE_KHR)` 与 `Ok(true)`（SUBOPTIMAL），
/// **其余 `Err` 一律被静默忽略** —— `ERROR_SURFACE_LOST_KHR` / `ERROR_DEVICE_LOST` 会被
/// 当成"这一帧呈现成功"，主循环继续跑（画面已经死了，帧计数与 fps 照走）。
/// 现在只认 `Ok(false)` 为成功；除"重建交换链"两种之外的 Err 一律升级为错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresentOutcome {
    /// `Ok(false)`：真·呈现成功
    Presented,
    /// OUT_OF_DATE / SUBOPTIMAL：交换链需要重建（这是**成功的一类**，不是失败）
    RecreateSwapchain,
    /// 其它 Err（SURFACE_LOST / DEVICE_LOST / …）：不能当成功
    Failed,
}

/// 纯函数：`queue_present` 的返回值 → 处置方式。判据见 `PresentOutcome`。
fn classify_present(result: Result<bool, vk::Result>) -> PresentOutcome {
    match result {
        Ok(false) => PresentOutcome::Presented,
        Ok(true) => PresentOutcome::RecreateSwapchain,
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => PresentOutcome::RecreateSwapchain,
        Err(_) => PresentOutcome::Failed,
    }
}

/// `acquire_next_image` 的**错误**结果分类（纯函数，可单测）。
///
/// 🔴 2026-09-25 复查补：此前 acquire 用的是 `timeout = u64::MAX`，于是"呈现引擎一直不给图像"
/// 会让主循环**静默卡死**在 acquire 里 —— 日志停住、无 panic、无 VUID、无 `has been lost`，
/// 从外面看就是"游戏死了"（当晚独显 + `defense_line` + IMMEDIATE 的 TDR 就是这个形态）。
/// 现在超时有限（`ACQUIRE_TIMEOUT_NS`），超时会计数、记日志，并最终降级到 mailbox 自动恢复。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcquireOutcome {
    /// 这一轮没有图像：`TIMEOUT` / `NOT_READY`，可以重试
    Retry,
    /// 交换链要重建（OUT_OF_DATE / SURFACE_LOST）
    RecreateSwapchain,
    /// 不可恢复（DEVICE_LOST 等）
    Failed,
}

fn classify_acquire_err(e: vk::Result) -> AcquireOutcome {
    match e {
        vk::Result::TIMEOUT | vk::Result::NOT_READY => AcquireOutcome::Retry,
        vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::ERROR_SURFACE_LOST_KHR => {
            AcquireOutcome::RecreateSwapchain
        }
        _ => AcquireOutcome::Failed,
    }
}

/// 某顶点 (x,z) 在下一级（更粗）网格曲面上的高度：
/// 先定位所在粗网格 cell，再用与地形索引一致的三角形剖分做重心插值。
/// 粗网格点与细网格点重合处返回值与该点粗网格高度完全一致。
fn terrain_coarse_height(x: f32, z: f32, coarse: &[f32], coarse_cells: usize) -> f32 {
    let cell = 512.0 / coarse_cells as f32;
    let uf = (x + TERRAIN_HALF) / cell;
    let vf = (z + TERRAIN_HALF) / cell;
    let cw = coarse_cells + 1;
    let cx = (uf.floor().max(0.0) as usize).min(coarse_cells - 1);
    let cz = (vf.floor().max(0.0) as usize).min(coarse_cells - 1);
    let u = uf - cx as f32;
    let v = vf - cz as f32;
    let h00 = coarse[cz * cw + cx];
    let h10 = coarse[cz * cw + cx + 1];
    let h01 = coarse[(cz + 1) * cw + cx];
    let h11 = coarse[(cz + 1) * cw + cx + 1];
    if u >= v {
        // 三角形 (v0,v1,v2)：右下三角
        (1.0 - u) * h00 + (u - v) * h10 + v * h11
    } else {
        // 三角形 (v0,v3,v2)：左上三角
        (1.0 - v) * h00 + (v - u) * h01 + u * h11
    }
}

/// 实例网格（256×256 = 65536）
const GRID_SIZE: u32 = 256;
const INSTANCE_COUNT: u32 = GRID_SIZE * GRID_SIZE;
/// 世界障碍 marker 上限（程序化地图每关障碍盒数远小于此；同时决定实例 buffer 的额外容量）
const MAX_MARKER_INSTANCES: u32 = 8192;
/// marker 在实例 buffer 中的起始 slot：跳过 0..=INSTANCE_COUNT。
///
/// slot 65536 是地形 identity（shader 硬编码 TERRAIN_INSTANCE_INDEX=65536 读取，
/// cull_and_upload 只写 0..visible-1，永不触碰），marker 从 65537 起，互不干扰。
const MARKER_SLOT_BASE: u32 = INSTANCE_COUNT + 1;
/// NPC 士兵可视化段数上限（每几何区；人形 15 段/人 × 8 NPC = 120 段，余量充足；
/// 同时决定实例 buffer 的额外容量。三几何区：盒（躯干/脚/枪）、圆柱（四肢）、球（头））
const MAX_NPC_INSTANCES: u32 = 3072; // 2026-08-23：128v128 + 尸体/存活混合实测峰值 2220
/// NPC 盒体段区起始 slot：紧接 marker 区之后（见 MARKER_SLOT_BASE）
const NPC_SLOT_BASE: u32 = MARKER_SLOT_BASE + MAX_MARKER_INSTANCES;
/// NPC 圆柱段区（四肢）起始 slot
const NPC_CYL_SLOT_BASE: u32 = NPC_SLOT_BASE + MAX_NPC_INSTANCES;
/// NPC 球体段区（头）起始 slot
const NPC_SPH_SLOT_BASE: u32 = NPC_CYL_SLOT_BASE + MAX_NPC_INSTANCES;
/// 自发光实体上限（爆炸闪光等瞬时特效，并发数远小于此）
const MAX_EMISSIVE_INSTANCES: u32 = 64;
/// 自发光实体在实例 buffer 中的起始 slot：紧接 NPC 区之后（见 NPC_SPH_SLOT_BASE）。
/// 必须与 build.rs 的 EMISSIVE_INSTANCE_BASE（NPC_INSTANCE_BASE + 3072）同步。
const EMISSIVE_SLOT_BASE: u32 = NPC_SPH_SLOT_BASE + MAX_NPC_INSTANCES;
/// 枪模专用 identity 槽（走 flat=1 纯色路径，与 build.rs 顶点 shader 同步）
const GUN_INSTANCE_INDEX: u32 =
    INSTANCE_COUNT + 1 + MAX_MARKER_INSTANCES + MAX_NPC_INSTANCES * 3 + MAX_EMISSIVE_INSTANCES;
/// GLB 道具合并网格专用 identity 槽。
///
/// 道具的位姿在 CPU 上就烘进顶点（`props::merge`），所以 GPU 侧只需要一个 identity 矩阵，
/// 和地形/枪模同一个套路——**不必为道具新增一整段实例区**，只多占一个槽位。
/// 该槽的 `tint.w = Shape::Authored.tag()`(=6.0) 是给 shader 看的，`tint.rgb = 1` 让
/// 片元直出顶点色（`input.color = vertexColor × tint.rgb`）。
const PROP_INSTANCE_INDEX: u32 = GUN_INSTANCE_INDEX + 1;
/// 🪖 **士兵 GLB 的实例区起点**（2026-09-13）。
///
/// 与道具那个"单个 identity 槽"不同：士兵的位姿**每帧都在变**（人要走动），
/// 不能把变换烘进顶点，所以**必须真的开一段实例区**，每个 NPC 占一个槽。
///
/// **为什么走实例化而不是照抄 NPC 的 18 段**：mesh 着色器每个 workgroup（=一个实例）
/// 最多输出 50 顶点 / 96 图元（`build.rs::MeshOutput`），而 `soldier.glb` 是
/// 1082 顶点 / 540 三角形 —— **结构上装不下**。而 `cmd_draw_indexed` 的**实例数是自由
/// 参数**，可以在传统顶点管线上一次画 N 个：网格上传一次，实例矩阵每帧写 N 个。
/// 这正是枪模已经在走的路（`self.gun_pipeline` + `cmd_draw_indexed`），
/// 而 `self.pipeline`（传统 VERTEX、`depth_test` 开）**在 mesh 可用时同样被无条件创建**，
/// 道具也早就在用它 —— 所以这里不需要新增任何管线。
const SOLDIER_INSTANCE_BASE: u32 = PROP_INSTANCE_INDEX + 1;
/// 士兵实例槽容量。取 `MAX_AI`(768) 的上限：压力模式红蓝各 128，加上普通波次也够。
/// **超出的 NPC 直接不画真网格**（退回 18 段箱体），不是静默越界：
/// `set_npc_visuals` 里 `len() < MAX_SOLDIER_INSTANCES` 就不再 push，`upload_soldiers`
/// 上传时再按容量取一次 min。（2026-09-22 复查：这里原先引用的 `write_soldier_instances`
/// 已不存在，是过期名字，已改正。）
const MAX_SOLDIER_INSTANCES: u32 = 768;
/// 士兵网格的顶点/索引预留容量（`soldier.glb` 实测 1082 顶点 / 540 索引，留 4 倍余量）。
/// ⚠️ 换更细的士兵模型要同步放大，否则 `set_soldier_mesh` 会拒绝上传并打 error。
const SOLDIER_MESH_VERTS: u32 = 4096;
const SOLDIER_MESH_INDICES: u32 = 8192;
/// 实例 storage buffer 的总元素数。**唯一权威定义**——历史上它是三份互相抄写的副本
/// （`buffer_elems` + 主管线 descriptor `.range()` + 阴影 pass descriptor `.range()`），
/// 加一个槽位只要漏改任一份，shader 就会对那一槽越界读 storage buffer：驱动不会报错，
/// 只会返回全零，于是 `inst.model` 变成零矩阵、所有顶点塌到一点、几何**完全不显示**且
/// 没有任何日志或 VUID 提示（2026-09-04 加道具槽时正好踩中，靠"红屏探针 + 换槽对照"才定位）。
/// 现在由最高槽位反推，结构上不可能再漏。
const INSTANCE_BUFFER_ELEMS: u64 =
    SOLDIER_INSTANCE_BASE as u64 + MAX_SOLDIER_INSTANCES as u64;
/// 并行剔除的**段数上限** = `cull_and_upload` 里两张栈上前缀和表 `[u32; N]` 的长度。
///
/// 🔴 2026-09-22 复查补：段数原来是裸的 `pool.workers() + 1`，只靠一句
/// `debug_assert!(nw <= 64)` 兜着 —— 而 **release 里 `debug_assert` 根本不存在**
/// （本仓 `[profile.release]` 用默认：`debug-assertions = false`、`overflow-checks = false`）。
/// 64 个 worker 以上的机器（64C/128T 起）会**每帧**在 `near_prefix[w]` 上
/// `index out of bounds: the len is 64 but the index is 64` —— 高核数机器一启动就崩。
/// 现在由 [`cull_segment_count`] 硬夹上限；本机的 `workers + 1` 远小于 64 ⇒ **行为零变化**。
const CULL_MAX_SEGMENTS: usize = 64;

/// 纯函数：worker 数 → 并行剔除段数（调用线程参与首段，故 +1），并以
/// [`CULL_MAX_SEGMENTS`] 为上限 —— 段数只影响并行度，不影响结果（前缀和按段相加，
/// 段边界怎么切都不改变可见集合与近/远分档）。**不碰线程调度策略**：池的拓扑、
/// 亲和、降频都在 `cpu.rs`，这里只决定"切几段"。
fn cull_segment_count(workers: usize) -> usize {
    (workers + 1).min(CULL_MAX_SEGMENTS)
}
/// 道具分桶边长（米）。全城约 ±175m ⇒ 约 9×9 格。
/// 🔴 2026-09-12 第 44 轮：由 40m 改为 20m。**原注释的理由已被实测推翻** ——
/// 「桶再小则 draw call 数上升（每桶一次 cmd_draw_indexed 与其绑定开销）」
/// 这条经 `RV3D_ONE_PROP_DRAW` 对照否定：把 28 个可见桶合并成 **1 个** draw
/// 反而**更慢**（126.7 → 82.6 fps，wait_fence 4164 → 9006µs），
/// 因为它没剔除、多画了 2.7 倍顶点。**draw call 数不是瓶颈。**
/// 而第 43 轮同时证明成本与**绘制的顶点数近似成正比**（2.69× 顶点 → 2.14× 时间）。
/// ⇒ 该优化的是"每帧画多少顶点"，切细桶正是为此；draw call 上升是安全的代价。
// 🔴 2026-09-12 第 104 轮：20.0 → **10.0**。
//
// 第 103 轮的同场 A/B 定案：**AI 侧 CPU 优化不再涨帧**（省 196µs `ai_us`，fps 纹丝不动）——
// 因为同期 `wait_fence_us` 3000~4000 = **CPU 在等 GPU**。⇒ 第②条只剩 GPU 侧，而**道具是最大分项
// （3.20ms / 34%）**，成本随**画的顶点数**走（第 44 轮实测：2.69x 顶点 → 2.14x 时间）。
//
// 分桶更细 ⇒ 每帧可见桶覆盖的顶点更少 ⇒ 直接减 GPU 顶点吞吐。**绘制调用数不是瓶颈**
// （第 44 轮实测：单桶全画反而更慢，82.6 fps）⇒ 细分安全。
// 第 44 轮 40→20 的已验证收益：三角形 −14%、顶点跨度 −15%、`wait_fence_us` 4164→2、fps 126.7→131.7。
// 🔴 2026-09-12：20.0 → **10.0**（第 104 轮，**实测 +1.35% fps**：中位 133.1→134.9、最差帧 125.6→131.4）。
//
// ⚠️ **5.0 试过并已按预先声明的判据退回**（第 105 轮）：中位 fps 一模一样（134.9），
// **而最差帧从 131.4 退到 125.5**。同场取证：桶 164/546 可见、提交三角形 199190、
// 顶点区间 474508（占顶点总数 1563020 的 **30%**）。
// ⇒ **10.0 是这条杠杆的拐点**：再细分只会增加绘制调用与剔除开销，不再减少顶点吞吐。
// **不要再往下调，除非先证明瓶颈已从"顶点吞吐"变成别的。**
const PROP_BIN_CELL_M: f32 = 10.0;
// 🔴 槽位布局断言改成**写死具体数字**。
// 原来这里写的是 `INSTANCE_BUFFER_ELEMS > GUN_INSTANCE_INDEX`，而它永远成立
// （本常量就是由 `SOLDIER_INSTANCE_BASE + MAX_SOLDIER_INSTANCES` 定义出来的，而
//  `SOLDIER_INSTANCE_BASE` 又是 `GUN_INSTANCE_INDEX + 2`）⇒ 按教训 14「永远成立的断言等于没写」。
// 现在任何槽位/容量改动都会**编译失败**，改动者必须回来同步 `build.rs` 的槽位常量
// 与三处 `.range()`（建 buffer / 主管线 / 阴影 pass）—— 那正是 2026-09-04 静默越界
// （几何整体消失、无 VUID）的入口。
const _: () = assert!(
    GUN_INSTANCE_INDEX == 83_009,
    "枪模槽位变了：必须与 build.rs 的枪槽字面量同步"
);
const _: () = assert!(
    EMISSIVE_SLOT_BASE == 82_945,
    "自发光区起点变了：必须与 build.rs 的 EMISSIVE_INSTANCE_BASE 同步"
);
const _: () = assert!(
    PROP_INSTANCE_INDEX == 83_010,
    "道具槽位变了：必须与 build.rs 同步"
);
const _: () = assert!(
    SOLDIER_INSTANCE_BASE == 83_011,
    "士兵实例区起点变了：必须与 build.rs 同步"
);
const _: () = assert!(
    INSTANCE_BUFFER_ELEMS == 83_779,
    "实例 buffer 容量变了：必须同步三处 .range() 与 build.rs"
);


/// 实例数据（model 4x4 + tint vec4，std430 步长 80 字节）
#[repr(C)]
#[derive(Copy, Clone)]
struct InstanceData {
    model: [f32; 16],
    tint: [f32; 4],
}
const _: () = assert!(std::mem::size_of::<InstanceData>() == 80);

/// 世界障碍 marker 输入（模型矩阵 = 平移+缩放，tint = 颜色）。
/// 由 main.rs 从游戏关卡地图转换而来，经 `set_world_markers` 缓存为实例数据。
pub struct WorldMarker {
    pub model: glam::Mat4,
    pub tint: [f32; 4],
}

impl WorldMarker {
    /// 障碍 marker 的模型矩阵 = 平移到盒心 × 逐轴归一的缩放。
    ///
    /// 缩放取 `half / template_half_extent(axis)`，因此**画出来的尺寸恒等于碰撞 AABB**：
    /// 立方体/球模板是 ±1（半幅 1）⇒ 缩放 = half；单位圆柱 y 只烘到 ±0.5 ⇒ 该轴缩放 = 2·half。
    /// 判据与"为什么以前一律写 `2*half` 是错的"见 `geom::Shape::template_half_extent`。
    fn obstacle_model(ob: &MapObstacle) -> glam::Mat4 {
        let half = glam::Vec3::new(ob.half_w, ob.half_h, ob.half_d);
        let tmpl = glam::Vec3::new(
            ob.shape.template_half_extent(0),
            ob.shape.template_half_extent(1),
            ob.shape.template_half_extent(2),
        );
        glam::Mat4::from_translation(glam::Vec3::new(ob.x, ob.y, ob.z))
            * glam::Mat4::from_scale(half / tmpl)
    }

    /// 从物理障碍盒构建世界 marker：模型见 [`Self::obstacle_model`]，
    /// 即渲染盒与碰撞 AABB **逐轴同尺寸**（2026-09-17 起；此前可见尺寸是 AABB 的 2 倍，
    /// 玩家能站进看得见的那半个盒子里，子弹也会打空）。
    ///
    /// 材质：按 ObstacleKind 调色板 + 确定性逐障碍微变（terrain_hash 量化格点），
    /// 墙（砖红）/块（金属灰蓝）/栅栏（木板）/树（树干棕）/建筑（混凝土）/残骸（土棕）
    /// 各有可辨识材质色；同一种类的相邻盒子明度/色相 ±6% 抖动，形成砖缝/板纹颗粒感。
    pub fn for_obstacle(ob: &MapObstacle) -> Self {
        WorldMarker {
            model: Self::obstacle_model(ob),
            tint: {
                // 🔴 `RV3D_DEBUG_KIND=1`：按 `ObstacleKind` 给六种纯色，**让几何自报家门**。
                //
                // 存在理由（2026-09-12）：为画面正中一组薄板连猜三个假设 ——
                // 柱廊檐梁 / 退化几何 / 广场长椅 —— **全部落空**，而二分只查出"它是 marker"。
                // **读代码猜类别已被证明无效（本会话第 4 次）**，所以改用这一招：
                // 关掉这个开关就是正常画面，打开则一眼看出那片薄板属于哪一类，
                // 再回 `city.rs` 找**那一个**调用点，不必再猜。
                // ⚠ 环境变量**只解析一次**。本函数每帧被调用 1700+ 次（`marker=1709`）——
                //    原先每次都 `std::env::var(...)`（带锁 + 扫环境表），**开关关着也照调**，
                //    130fps 下约 22 万次/秒，是纯浪费（2026-09-12 第 93 轮修）。
                static DEBUG_KIND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                if *DEBUG_KIND.get_or_init(|| std::env::var("RV3D_DEBUG_KIND").is_ok()) {
                    let c = match ob.kind {
                        ObstacleKind::Wall => [1.0, 0.0, 0.0],      // 红
                        ObstacleKind::Block => [0.0, 1.0, 0.0],     // 绿
                        ObstacleKind::Barrier => [0.0, 0.0, 1.0],   // 蓝
                        ObstacleKind::Tree => [1.0, 1.0, 0.0],      // 黄
                        ObstacleKind::Building => [1.0, 0.0, 1.0],  // 品红
                        ObstacleKind::Ruin => [0.0, 1.0, 1.0],      // 青
                    };
                    return WorldMarker {
                        model: Self::obstacle_model(ob),
                        tint: [c[0], c[1], c[2], ob.shape.tag()],
                    };
                }
                let mut t = match ob.tint {
                    Some(c) => [c[0], c[1], c[2], 1.0],
                    None => obstacle_material_tint(ob.kind, ob.x, ob.z),
                };
                // tint.w 是几何形状标签（见 engine::geom）。Legacy 标签 = 1.0，
                // 与旧代码写死的 1.0 逐位相同，所以未迁移的障碍画面不变。
                t[3] = ob.shape.tag();
                t
            },
        }
    }
}

/// 障碍种类 → 基础材质色（片元 marker 路径直出 tint × fade，无贴图混合）
fn obstacle_base_color(kind: ObstacleKind) -> [f32; 3] {
    match kind {
        ObstacleKind::Wall => [0.66, 0.38, 0.30], // 砖墙红
        ObstacleKind::Block => [0.46, 0.50, 0.56], // 金属掩体（钢灰蓝）
        ObstacleKind::Barrier => [0.60, 0.45, 0.28], // 木板路障
        ObstacleKind::Tree => [0.48, 0.36, 0.22], // 树干棕
        ObstacleKind::Building => [0.58, 0.58, 0.61], // 混凝土
        ObstacleKind::Ruin => [0.44, 0.39, 0.33], // 残骸土棕
    }
}

/// 障碍 marker 材质 tint：种类基础色 × 确定性逐障碍微变（1/8m 量化格点哈希）。
/// 同一障碍每帧/每关颜色恒定；三通道各自独立抖动形成材质颗粒感。
fn obstacle_material_tint(kind: ObstacleKind, x: f32, z: f32) -> [f32; 4] {
    let base = obstacle_base_color(kind);
    let qx = (x * 8.0).round() as i32;
    let qz = (z * 8.0).round() as i32;
    let unit = |ix: i32, iz: i32| (terrain_hash(ix, iz) & 0xFFFF) as f32 / 65535.0;
    let jr = 0.94 + 0.12 * unit(qx + 1, qz);
    let jg = 0.94 + 0.12 * unit(qx, qz + 1);
    let jb = 0.94 + 0.12 * unit(qx, qz);
    [
        (base[0] * jr).clamp(0.0, 1.0),
        (base[1] * jg).clamp(0.0, 1.0),
        (base[2] * jb).clamp(0.0, 1.0),
        1.0,
    ]
}

/// NPC 士兵可视化输入（位置/朝向 yaw/阵营配色）。
/// 由 main.rs 从游戏 AI 状态转换而来，经 `set_npc_visuals` 展开为 7 段积木人实例数据。
pub struct NpcVisual {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub tint: [f32; 4],
    /// 动画相位（秒，由 main.rs 累积时钟驱动；行走摆动/开火后坐共用）
    pub phase: f32,
    /// 移动中（腿/臂摆动动画）
    pub moving: bool,
    /// 攻击开火（枪身/手臂后坐脉冲）
    pub firing: bool,
}

/// 单个地形 LOD 网格：静态几何（顶点/索引）+ CPU 侧顶点（供高度 morph 逐帧更新）
struct TerrainLodMesh {
    vertex_buffer: vk::Buffer,
    vertex_memory: vk::DeviceMemory,
    /// 持久映射的顶点内存指针（每帧 morph 后整块重传）
    vertex_mapped: *mut std::ffi::c_void,
    index_buffer: vk::Buffer,
    index_memory: vk::DeviceMemory,
    index_count: u32,
    /// CPU 侧顶点（pos.y 每帧按 morph 更新后整块上传）
    verts: Vec<Vertex>,
    /// 顶点原始细高度（terrain_height，永不改变）
    base_heights: Vec<f32>,
    /// 顶点向下一级曲面插值的高度目标（Low 级为空 Vec）
    coarse_heights: Vec<f32>,
}

// ============================================================
// 地形高度（程序化丘陵 + 中央平坦作战区）
// ============================================================

/// Hermite 平滑插值（LOD morph 过渡系数 / 地形抬升 / 值噪声插值共用）
fn smooth_t(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// 确定性整数哈希（纯 u32 算术，跨平台逐位一致；值噪声格点采样用）
fn terrain_hash(ix: i32, iz: i32) -> u32 {
    let mut h = (ix as u32).wrapping_mul(0x1B873593) ^ (iz as u32).wrapping_mul(0xCC9E2D51);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846CA68B);
    h ^= h >> 16;
    h
}

/// 格点伪随机高度：[-1, 1)
fn terrain_lattice_height(ix: i32, iz: i32) -> f32 {
    (terrain_hash(ix, iz) & 0xFFFF) as f32 / 32768.0 - 1.0
}

/// 双线性 smoothstep 值噪声（确定性、低频平缓、C1 连续）
fn terrain_value_noise(x: f32, z: f32, cell: f32) -> f32 {
    let fx = x / cell;
    let fz = z / cell;
    let ix = fx.floor() as i32;
    let iz = fz.floor() as i32;
    let tx = smooth_t(fx - ix as f32);
    let tz = smooth_t(fz - iz as f32);
    let h00 = terrain_lattice_height(ix, iz);
    let h10 = terrain_lattice_height(ix + 1, iz);
    let h01 = terrain_lattice_height(ix, iz + 1);
    let h11 = terrain_lattice_height(ix + 1, iz + 1);
    let a = h00 + (h10 - h00) * tx;
    let b = h01 + (h11 - h01) * tx;
    a + (b - a) * tz
}

/// 地形高度：半径 ≤ TERRAIN_FLAT_RADIUS（中央 60×60 安全区、障碍环带 58–130m、
/// 两军接火区都落在此圆内）恒 y=0；之外按距离 smoothstep 抬升的确定性值噪声丘陵，
/// 幅值 ≤ TERRAIN_HILL_AMPLITUDE、坡度平缓（LOD morph 无突兀）。
pub fn terrain_height(x: f32, z: f32) -> f32 {
    let flat_r2 = TERRAIN_FLAT_RADIUS * TERRAIN_FLAT_RADIUS;
    let r2 = x * x + z * z;
    if r2 <= flat_r2 {
        return 0.0;
    }
    let t = ((r2.sqrt() - TERRAIN_FLAT_RADIUS) / TERRAIN_HILL_RAMP).clamp(0.0, 1.0);
    smooth_t(t) * TERRAIN_HILL_AMPLITUDE * terrain_value_noise(x, z, TERRAIN_HILL_CELL)
}

/// 供 CPU 侧（NPC/实例）查询地形高度，与 GPU 地形完全同源
pub fn terrain_height_at(x: f32, z: f32) -> f32 {
    terrain_height(x, z)
}

// ============================================================
// 渲染器
// ============================================================

pub struct Renderer {
    _entry: Entry,
    instance: Instance,
    #[allow(dead_code)]
    debug_utils: Option<DebugUtils>,
    #[allow(dead_code)]
    debug_messenger: Option<vk::DebugUtilsMessengerEXT>,
    surface_loader: Surface,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    #[allow(dead_code)]
    physical_device_properties: vk::PhysicalDeviceProperties,
    graphics_queue_family_index: u32,
    present_queue_family_index: u32,
    device: Device,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    swapchain_loader: Swapchain,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    swapchain_format: vk::Format,
    swapchain_extent: vk::Extent2D,
    swapchain_image_views: Vec<vk::ImageView>,
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    /// 第一人称枪模专用管线：与 `pipeline` 完全同源（同 shader 模块、同 layout、同
    /// render pass），**只有 depth 状态不同**——本管线 `depth_test=OFF` 且不写深度。
    ///
    /// 为什么要单独一条：主管线历史上把 `depth_test_enable` 设成 false，整个世界几何
    /// 因此没有遮挡（楼穿楼）。主管线改成 true 之后，枪模会被它前面的墙裁掉——而
    /// 第一人称武器必须恒可见。所以把"不测深度"这个需求**收束到只服务枪模的一条管线上**，
    /// 而不是让整个世界陪着它放弃遮挡。
    gun_pipeline: vk::Pipeline,
    /// 可选网格着色器路径（VK_EXT_mesh_shader）：mesh 管线 + 独立 pipeline layout
    /// （同一 descriptor set layout + MESH_EXT push constant）。mesh_enabled=false 时
    /// 保持 null 且完全不参与记录阶段，传统顶点管线行为逐字节不变。
    mesh_enabled: bool,
    mesh_shader: Option<MeshShaderDevice>,
    mesh_pipeline: vk::Pipeline,
    mesh_pipeline_layout: vk::PipelineLayout,
    /// 设备 maxMeshWorkGroupCount[0]（VK_EXT_mesh_shader 最低保证 65535）；
    /// 地面场 65536 个 workgroup 单次下发会超限，绘制按此值分块。
    mesh_max_wg_x: u32,
    /// 虚空检视模式（枪械检视）：只画枪模——不画地形/NPC/marker/阴影，背景纯色虚空
    pub void_mode: bool,
    framebuffers: Vec<vk::Framebuffer>,
    /// MSAA 采样数（RV3D_MSAA=1/2/4/8，默认 4；0 或 1 = 关）。主 pass 颜色/深度附件
    /// 用该采样数渲染，经 resolve 附件输出到交换链图像（几何边缘抗锯齿）。
    msaa_samples: vk::SampleCountFlags,
    /// MSAA 颜色附件（每交换链图像一个，samples=msaa_samples；渲染目标，不 STORE）
    msaa_images: Vec<vk::Image>,
    msaa_image_memory: Vec<vk::DeviceMemory>,
    msaa_image_views: Vec<vk::ImageView>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    image_available_semaphores: Vec<vk::Semaphore>,
    render_finished_semaphores: Vec<vk::Semaphore>,
    in_flight_fences: Vec<vk::Fence>,
    /// 连续 acquire 超时计数（>0 说明呈现引擎没给图像；判据见 `classify_acquire_err`）
    acquire_timeouts: u32,
    /// 连续围栏超时计数 + "GPU 侧卡死"标志：卡死后 render() 直接返回 Ok(())，
    /// 主循环保持响应（输入/日志照常），而不是每帧卡满超时。
    fence_timeouts: u32,
    gpu_stalled: bool,
    /// 呈现模式覆盖（`None` = 按 `RV3D_PRESENT_MODE` 选）；acquire 持续超时会降级写 mailbox
    present_mode_override: Option<vk::PresentModeKHR>,
    current_frame: usize,
    max_frames_in_flight: usize,
    /// 上一帧 render() 总耗时（微秒，性能日志用）
    last_frame_us: u64,
    last_cull_us: u64,
    /// 物理设备名称（性能日志头部用）
    device_name: String,
    vertex_buffer: vk::Buffer,
    vertex_buffer_memory: vk::DeviceMemory,
    index_buffer: vk::Buffer,
    index_buffer_memory: vk::DeviceMemory,
    /// 远档 LOD 十字 quad 几何（独立 vertex/index buffer）
    far_vertex_buffer: vk::Buffer,
    far_vertex_buffer_memory: vk::DeviceMemory,
    far_index_buffer: vk::Buffer,
    far_index_buffer_memory: vk::DeviceMemory,
    /// 地面平铺 quad 几何（近档+远档地面 draw 共用，见 GROUND_VERTS）
    ground_vertex_buffer: vk::Buffer,
    ground_vertex_buffer_memory: vk::DeviceMemory,
    ground_index_buffer: vk::Buffer,
    ground_index_buffer_memory: vk::DeviceMemory,
    /// UV 球体几何（爆炸球形扩散用；CPU 生成 24×12 段，见 create_sphere_geometry）
    sphere_vertex_buffer: vk::Buffer,
    sphere_vertex_buffer_memory: vk::DeviceMemory,
    sphere_index_buffer: vk::Buffer,
    sphere_index_buffer_memory: vk::DeviceMemory,
    sphere_index_count: u32,
    /// NPC 人体圆柱几何（四肢用；CPU 生成 24 段含上下盖，见 create_cylinder_geometry）
    cylinder_vertex_buffer: vk::Buffer,
    cylinder_vertex_buffer_memory: vk::DeviceMemory,
    cylinder_index_buffer: vk::Buffer,
    cylinder_index_buffer_memory: vk::DeviceMemory,
    cylinder_index_count: u32,
    /// 地形 LOD 网格（索引 0/1/2 = 高/中/低密度；顶点缓冲 HOST_VISIBLE 供 morph 每帧更新）
    terrain_lods: Vec<TerrainLodMesh>,
    /// 每帧一份 instance buffer（双缓冲，避免 CPU 写与上一帧 GPU 读竞态）
    instance_buffers: Vec<vk::Buffer>,
    instance_buffers_memory: Vec<vk::DeviceMemory>,
    /// 每帧对应的持久映射指针
    instance_mapped: Vec<*mut std::ffi::c_void>,
    /// 全量实例（CPU 侧保留，每帧剔除后压缩上传）
    instances: Vec<InstanceData>,
    /// 剔除结果暂存：并行剔除阶段 A 每段写入可见实例索引（容量 = INSTANCE_COUNT，
    /// 创建时一次分配，避免每帧堆分配；阶段 B 按前缀和从暂存拷贝上传）
    culled_scratch: Vec<u32>,
    /// 并行剔除各段近档计数（阶段 A 统计，join 后做前缀和，供阶段 B 定位写入偏移）
    seg_near_counts: Vec<std::sync::atomic::AtomicU32>,
    /// 并行剔除各段远档计数
    seg_far_counts: Vec<std::sync::atomic::AtomicU32>,
    /// 每实例包围球半径（创建时预算，剔除循环查表免每帧 sqrt）
    instance_radii: Vec<f32>,
    /// 实例球心 SoA（SIMD 剔除用，创建时一次填充，连续内存便于向量化加载）
    instance_center_x: Vec<f32>,
    instance_center_y: Vec<f32>,
    instance_center_z: Vec<f32>,
    /// 世界障碍 marker（关卡切换时由 main.rs 设置；独立于实例场，见 MARKER_SLOT_BASE）
    markers: Vec<InstanceData>,
    /// 本帧 marker 近/远档计数（record_command_buffer 读取，render 时更新）
    last_marker_near: u32,
    last_marker_far: u32,
    /// NPC 士兵段实例（由 set_npc_visuals 构建，三几何分区：盒/圆柱/球）
    npc_box_parts: Vec<InstanceData>,
    npc_cyl_parts: Vec<InstanceData>,
    npc_sph_parts: Vec<InstanceData>,
    /// 本帧 NPC 近/远档段计数（record_command_buffer 读取，render 时更新）
    last_npc_box_near: u32,
    last_npc_box_far: u32,
    last_npc_cyl_near: u32,
    last_npc_cyl_far: u32,
    last_npc_sph_near: u32,
    last_npc_sph_far: u32,
    /// 自发光实体实例（爆炸闪光等，由 set_emissive_markers 构建，见 EMISSIVE_SLOT_BASE）
    emissive_markers: Vec<InstanceData>,
    /// 本帧自发光近/远档计数（record_command_buffer 读取，render 时更新）
    last_emissive_near: u32,
    last_emissive_far: u32,
    /// 性能日志节流（1 次/秒）
    last_perf_log: Instant,
    /// 时间窗内帧计数（fps 统计）
    frame_count: u32,
    /// fps 统计时间窗起点
    perf_window_start: Instant,
    /// 性能探针：本帧各阶段耗时（µs，1Hz 日志输出，定位 CPU/GPU/交换链瓶颈）
    stage_wait_fence_us: u64,
    stage_acquire_us: u64,
    stage_terrain_us: u64,
    stage_record_us: u64,
    stage_submit_us: u64,
    stage_present_us: u64,
    depth_images: Vec<vk::Image>,
    depth_images_memory: Vec<vk::DeviceMemory>,
    depth_image_views: Vec<vk::ImageView>,
    // ---- 阴影贴图（2026-08-11：depth-only pass 渲光空间深度，主 pass 3x3 PCF）----
    shadow_image: vk::Image,
    shadow_image_memory: vk::DeviceMemory,
    shadow_image_view: vk::ImageView,
    shadow_sampler: vk::Sampler,
    shadow_render_pass: vk::RenderPass,
    shadow_framebuffer: vk::Framebuffer,
    shadow_pipeline_layout: vk::PipelineLayout,
    shadow_pipeline: vk::Pipeline,
    /// 阴影 UBO（每帧 slot 一份 64B mat4，避免 in-flight 竞态）
    shadow_ubo_buffers: Vec<vk::Buffer>,
    shadow_ubo_memory: Vec<vk::DeviceMemory>,
    shadow_ubo_mapped: Vec<*mut std::ffi::c_void>,
    shadow_descriptor_set_layout: vk::DescriptorSetLayout,
    shadow_descriptor_sets: Vec<vk::DescriptorSet>,
    // ---- 新增：Uniform / Descriptor 相关 ----
    descriptor_set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    descriptor_sets: Vec<vk::DescriptorSet>,
    uniform_buffers: Vec<vk::Buffer>,
    uniform_buffers_memory: Vec<vk::DeviceMemory>,
    uniform_mapped: Vec<*mut std::ffi::c_void>,
    /// 光照 Uniform（每帧一份，默认全零 = 光照关闭）
    light_uniform_buffers: Vec<vk::Buffer>,
    light_uniform_buffers_memory: Vec<vk::DeviceMemory>,
    light_uniform_mapped: Vec<*mut std::ffi::c_void>,
    texture_image: vk::Image,
    texture_image_memory: vk::DeviceMemory,
    texture_image_view: vk::ImageView,
    texture_sampler: vk::Sampler,
    // ---- marker/NPC 程序化皮肤纹理（RV3D_SKIN_TEX=1 时片元采样；缺省 0 纯色回退）----
    skin_marker_image: vk::Image,
    skin_marker_memory: vk::DeviceMemory,
    skin_marker_image_view: vk::ImageView,
    skin_npc_image: vk::Image,
    skin_npc_memory: vk::DeviceMemory,
    skin_npc_image_view: vk::ImageView,
    /// 地面微细节 tile 纹理（build.rs 片元 `@group(0) @binding(9) ground_detail_tex`）。
    ///
    /// ⚠ **这张图必须存在并被绑定**，否则就是 2026-09-03「大面积黑地」的确证根因：
    /// 片元着色器无条件静态引用 binding 9，而 descriptor set layout 历史上只到 binding 8，
    /// 于是该描述符槽从未创建 → 驱动给空描述符 → 采样恒返回 0 →
    /// `mixed *= mix(1.0, g * GROUND_DETAIL_GAIN, gdetail)` 变成 `mixed *= (1 - gdetail)`。
    /// `gdetail = 1 - smoothstep(0.06, 0.25, 米每像素)` 在相机周边（地面俯角 > ~5°，
    /// 实测半径 ~20-30m 一圈）恒等于 1 → **地面被乘成纯黑 (0,0,0)**；越远 gdetail→0
    /// 才逐渐恢复正常，所以黑区边界是一条恒定俯角的水平线并带一段平滑灰阶过渡。
    /// 这条路径与光照/阴影无关（`light_data.flags.x < 0.5` 的早退分支同样乘 `mixed`），
    /// 也与实例覆盖无关；marker/NPC/枪模在到达这段代码前就 return，所以楼和树照常。
    /// 注意 swapchain clear color 是 (0.24,0.36,0.60) 浅蓝，**黑色绝不可能是"露出清屏色"**。
    ///
    /// 必须以 UNORM（线性）view 创建：纹素存的是「亮度调制 / 2」（见 procedural.rs
    /// `generate_ground_detail_texture`），走 SRGB view 会把 128 解成 0.214 → 全场暗一半。
    ground_detail_image: vk::Image,
    ground_detail_memory: vk::DeviceMemory,
    ground_detail_image_view: vk::ImageView,
    /// RV3D_SKIN_TEX=1 启用 marker/NPC 皮肤纹理（缺省 0 = 保持纯色路径，冒烟基线不变）
    skin_tex_enabled: bool,
    /// 各向异性过滤是否可用（物理设备支持 samplerAnisotropy 时为 true）
    texture_anisotropy_enabled: bool,
    // ---- HUD 覆盖层（自包含：独立 pipeline / 独立顶点缓冲，不侵入主 pass）----
    hud_pipeline: vk::Pipeline,
    /// HUD **overlay pass 专用**管线（1 采样、无深度、`render_pass = hud_render_pass`）。
    /// 与 `hud_pipeline` 的区别只有渲染状态 —— 主 pass 那份是 MSAA + 带深度的，绑错就是 UB。
    hud_overlay_pipeline: vk::Pipeline,
    hud_pipeline_layout: vk::PipelineLayout,
    hud_vertex_buffer: vk::Buffer,
    hud_vertex_buffer_memory: vk::DeviceMemory,
    hud_mapped: *mut std::ffi::c_void,
    hud_vertex_count: u32,
    hud_render_pass: vk::RenderPass,
    hud_framebuffers: Vec<vk::Framebuffer>,
    hud_capacity_quads: u32,
    // ---- 第一人称枪模专用网格（程序化高模，主管线绘制 = 深度测试关 = 恒可见不穿模）----
    gun_vertex_buffer: vk::Buffer,
    gun_vertex_buffer_memory: vk::DeviceMemory,
    gun_index_buffer: vk::Buffer,
    gun_index_buffer_memory: vk::DeviceMemory,
    gun_mapped: *mut std::ffi::c_void,
    gun_vertex_count: u32,
    gun_index_count: u32,
    gun_buffer_capacity_verts: u32,
    gun_buffer_capacity_idx: u32,
    // ---- 🪖 士兵 GLB 网格（2026-09-13）：走 `self.pipeline`（传统 VERTEX、depth 开）+ 实例化 ----
    /// 与枪模不同，**按常量容量一次分配、永不重建**：士兵网格只在启动时上传一次，
    /// 不存在切枪那种"容量忽大忽小"的场景，而重建会 destroy 在飞 buffer → device lost。
    soldier_vertex_buffer: vk::Buffer,
    soldier_vertex_buffer_memory: vk::DeviceMemory,
    soldier_index_buffer: vk::Buffer,
    soldier_index_buffer_memory: vk::DeviceMemory,
    soldier_vertex_count: u32,
    soldier_index_count: u32,
    /// 本帧要画的士兵实例（每 NPC 一个槽）。`set_npc_visuals` 填，上传后清零。
    soldier_parts: Vec<InstanceData>,
    /// 本帧实际写入的实例数（= `soldier_parts.len().min(MAX_SOLDIER_INSTANCES)`）
    soldier_drawn: u32,
    /// NPC 段数触顶告警的**一次性闩**（2026-09-14）。
    ///
    /// 为什么需要它：`set_npc_visuals` / `set_dead_bodies` 里三处
    /// `if len < MAX_NPC_INSTANCES { push }` 在超容时**静默丢弃** —— 不崩、不报 VUID、
    /// 画面上只是"少了几个兵"，与未结案 #12 描述的是同一类失败模式。
    /// 容量本身够用（3072 vs 实测峰值 2220），所以这个闩**正常情况下永不置位**；
    /// 一旦置位，排查的人就能立刻知道"少人"是因为触顶，
    /// 而不必去怀疑剔除矩阵 / 模型 / 取景（那三样我今晚都白查过）。
    /// 用闩而不是每次都记：这段每帧都跑，刷屏会把真正有用的行淹掉。
    npc_cap_warned: bool,
    /// PT 盒数触顶告警的一次性闩（2026-09-14）。
    ///
    /// 理由同 `npc_cap_warned`：`ray_tracer::PT_MAX_BOXES` 超限时那一行 `.min()`
    /// **静默丢弃**，而 PT 是低 spp 的噪点图 —— **少几个盒子肉眼根本看不出来**
    /// （未结案 #10 实测 `marker=547 > 512`，即每次丢 35 个）。
    /// PT 本来就被当作"调试/烘焙参照视图"，所以更没人会去数盒子。
    /// 用闩是因为它由 `signature()` 量化（~0.5m）触发场景重建，移动相机时一秒能重建好几次。
    pt_box_cap_warned: bool,
    /// GLB 道具合并网格（`engine::props::merge` 在 CPU 上烘好位姿的静态几何）。
    /// 全部道具共用一次 draw call：位姿已进顶点，所以只需要 `PROP_INSTANCE_INDEX`
    /// 这一个 identity 实例，不必为道具新开一整段实例区。
    prop_vertex_buffer: vk::Buffer,
    prop_vertex_memory: vk::DeviceMemory,
    prop_index_buffer: vk::Buffer,
    prop_index_memory: vk::DeviceMemory,
    prop_mapped: *mut std::ffi::c_void,
    prop_vertex_count: u32,
    prop_index_count: u32,
    prop_capacity_verts: u32,
    prop_capacity_idx: u32,
    /// 道具的空间分桶（每桶一段连续索引 + 包围球）。见 `engine::props::merge_binned`。
    /// 桶共用同一个 VBO/IBO，剔除只决定发不发某一段索引，不需要重传顶点。
    prop_bins: Vec<crate::engine::props::PropBin>,
    /// 阴影专用几何（2026-09-19 建筑 LOD 专项，PROGRESS §14）：名单建筑烘成
    /// AABB 盒壳（`merge_shadow_binned`），只被 shadow pass 绑定；主 pass 不读它。
    /// 缓冲随地图重载整体重建，不做持久映射。
    prop_sh_vertex_buffer: vk::Buffer,
    prop_sh_vertex_memory: vk::DeviceMemory,
    prop_sh_index_buffer: vk::Buffer,
    prop_sh_index_memory: vk::DeviceMemory,
    prop_sh_index_count: u32,
    prop_sh_bins: Vec<crate::engine::props::PropBin>,
    /// RV3D_SHADOW_LOD=0 → 阴影退回全量道具几何（A/B 诊断门，同 RV3D_NO_SHADOW 惯例）
    shadow_lod: bool,
    /// 本帧视锥 6 平面（法线朝外）。由 `render()` 在写 CameraUniform 的同一处填，
    /// 那时 `record_command_buffer()` 还没被调用，所以道具分桶剔除拿到的一定是本帧的。
    frame_frustum: [[f32; 4]; 6],
    /// 上一帧渲染统计（供 HUD / 日志）
    last_near_count: u32,
    last_far_count: u32,
    last_terrain_lod_name: &'static str,
    /// 当前光照 uniform（每帧由 set_lights 更新，render 时写入帧 slot）
    light_data: LightUniform,
    /// 当前画质预设（默认 Medium = 现有行为；纯 CPU 侧参数）
    quality: QualityPreset,
    /// 常驻 PT 资源（首帧构建一次复用；2026-08-29 修复每帧重建+泄漏！）
    pt_resident: Option<Box<crate::engine::ray_tracer::PtAssets>>,
    pt_img: vk::Image,
    pt_img_mem: vk::DeviceMemory,
    pt_view: vk::ImageView,
    pt_pipeline: vk::Pipeline,
    pt_layout: vk::PipelineLayout,
    pt_setl: vk::DescriptorSetLayout,
    pt_pool: vk::DescriptorPool,
    pt_dset: vk::DescriptorSet,
    pt_module: vk::ShaderModule,
    /// 时域累积图像（RGBA32F：rgb=Σ线性样本，a=已累积 spp）
    pt_acc: vk::Image,
    pt_acc_mem: vk::DeviceMemory,
    pt_acc_view: vk::ImageView,
    /// 已累积帧数 / 目标 spp / 下一帧是否清空重开 / 上次取景指纹
    /// （Cell：累积状态在 `record_command_buffer(&self)` 里推进，改签名会波及整条渲染链）
    pt_frame: std::cell::Cell<u32>,
    pt_spp_target: u32,
    pt_reset: std::cell::Cell<bool>,
    pt_view_sig: std::cell::Cell<u64>,
    /// 实时 PT 渲染分辨率（init_pt_resident 决定，上屏块必须用同一个值，
    /// 否则 dispatch 与图像尺寸不一致 = 越界写/半屏黑）
    pub pt_size: (u32, u32),
    pub pt_move_base_cam: std::cell::Cell<[f32; 3]>,
    pub pt_move_base_fwd: std::cell::Cell<[f32; 3]>,

    /// 路径追踪实时渲染开关（config.pt_enable；present 前 PT 帧上屏）
    pub pt_live_enabled: bool,
    /// PT 取景参数（每帧 set_pt_params 注入：相机 + 太阳 + 曝光）
    pt_params: crate::engine::ray_tracer::PtParams,
    /// 当前 BLAS 内容对应的盒数（= pt_fill_geom 写入的盒数量）
    pt_box_count: usize,
    /// 场景指纹（WorldMarker 集合变化时重建 BLAS，避免逐帧重建）
    pt_scene_sig: u64,
    /// 当前 PT BLAS 创建时的道具几何键：(道具 VB 句柄, 道具属性表句柄, 道具索引数)。
    /// 任一变化 ⇒ BLAS 尺寸/引用都要变 ⇒ 必须整体重建（就地 rebuild 只重写内容）。
    /// 见 2026-09-19「道具喂进 BLAS」专项（用户决策）。
    pt_prop_key: (u64, u64, u32),
    /// 🏢 道具逐三角属性表（2×u32/三角：量化面法线 + 平均顶点色），**device-local**。
    /// 着色器绝不允许直接随机读道具主 VB——那是 HOST_VISIBLE 内存，每次命中都走 PCIe，
    /// pt3 实测把 PT 从 126fps 打到 1.5fps。表在 set_props 里随道具一起重建。
    prop_attr_buf: vk::Buffer,
    prop_attr_mem: vk::DeviceMemory,
    prop_attr_tris: u32,
    /// 截图请求路径（Some 表示本帧渲染完成后读回 swapchain 图像并写 PNG）
    screenshot_request: Option<std::path::PathBuf>,
    /// 截图读回 staging buffer（按 max_frames_in_flight 双缓冲，惰性创建）
    screenshot_buffers: Vec<vk::Buffer>,
    screenshot_buffers_memory: Vec<vk::DeviceMemory>,
    /// 截图读回 fence（每帧 slot 一个，提交拷贝后等待）
    screenshot_fences: Vec<vk::Fence>,
}

fn load_spirv(path: &str) -> Result<Vec<u32>, String> {
    let mut file = File::open(path).map_err(|e| format!("打开着色器文件失败 '{}': {}", path, e))?;
    util::read_spv(&mut file).map_err(|e| format!("读取 SPIR-V 文件失败 '{}': {}", path, e))
}

/// POD → &[u8]（push constants 上传，零外部依赖）
#[inline]
fn bytemuck_bytes<T: Sized>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v as *const T as *const u8, std::mem::size_of::<T>()) }
}

/// PtBox.material → 反照率（与 WorldMarker 障碍调色板同源，PT 才可当烘焙参照）
fn pt_albedo_of(b: &crate::engine::ray_tracer::PtBox) -> [f32; 3] {
    let k = match b.material {
        1 => ObstacleKind::Building,
        2 => ObstacleKind::Block,
        3 => ObstacleKind::Tree,
        _ => return [0.34, 0.32, 0.29],
    };
    obstacle_base_color(k)
}

/// PT 道具逐三角属性表烘焙（纯函数，判据见 `pt_prop_attrs_tests`）：
/// 每三角 2×u32 = 量化面法线（(v·127+127) 每轴 u8）+ 平均顶点色（u8×3）。
/// 法线由世界坐标顶点直接算（道具是闭合壳，着色器再翻到迎向来射侧，绕序无关）；
/// 退化三角形回退 [0,1,0]（NaN 钳黑教训同源）。不足一整三角的尾索引被丢弃，
/// 由 build_pt_as 的 `prop_attr_tris*3 == prop_index_count` 等式把关兜底。
fn pt_bake_prop_attrs(verts: &[[f32; 11]], indices: &[u32]) -> Vec<u32> {
    let ntri = indices.len() / 3;
    let mut attrs: Vec<u32> = Vec::with_capacity(ntri * 2);
    let qn = |v: f32| -> u32 { (v.clamp(-1.0, 1.0) * 127.0 + 127.0).round() as u32 & 0xFF };
    let qc = |v: f32| -> u32 { (v.clamp(0.0, 1.0) * 255.0).round() as u32 & 0xFF };
    for tri in indices.chunks_exact(3) {
        let vp = |k: usize, c: usize| verts[tri[k] as usize][c];
        let e1 = [vp(1, 0) - vp(0, 0), vp(1, 1) - vp(0, 1), vp(1, 2) - vp(0, 2)];
        let e2 = [vp(2, 0) - vp(0, 0), vp(2, 1) - vp(0, 1), vp(2, 2) - vp(0, 2)];
        let mut n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let ln = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if ln > 1e-7 {
            n = [n[0] / ln, n[1] / ln, n[2] / ln];
        } else {
            n = [0.0, 1.0, 0.0];
        }
        let w0 = qn(n[0]) | (qn(n[1]) << 8) | (qn(n[2]) << 16);
        let cr = (vp(0, 8) + vp(1, 8) + vp(2, 8)) / 3.0;
        let cg = (vp(0, 9) + vp(1, 9) + vp(2, 9)) / 3.0;
        let cb = (vp(0, 10) + vp(1, 10) + vp(2, 10)) / 3.0;
        let w1 = qc(cr) | (qc(cg) << 8) | (qc(cb) << 16);
        attrs.push(w0);
        attrs.push(w1);
    }
    attrs
}

/// 场景指纹（坐标量化到 1m）：只有盒集合真的变了才重建 BLAS，避免逐帧重建
fn pt_scene_sig(boxes: &[crate::engine::ray_tracer::PtBox]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    {
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        mix(boxes.len() as u64);
        for b in boxes {
            for c in b.center.iter().chain(b.half.iter()) {
                mix(*c as i32 as u32 as u64);
            }
            mix(b.material as u64);
        }
    }
    h
}

impl Renderer {
    pub fn new(window: &Window) -> Result<Self, String> {
        let mut renderer = Self::init_instance(window)?;
        renderer.init_swapchain()?;
        renderer.init_render_pass()?;
        renderer.init_command_pool()?;
        renderer.create_instance_buffer()?;
        renderer.init_msaa_resources()?;
        renderer.init_depth_resources()?;
        renderer.init_descriptors()?;       // ← 新增
        renderer.init_pipeline()?;
        renderer.init_mesh_pipeline()?;
        renderer.init_hud()?;
        renderer.init_framebuffers()?;
        renderer.init_hud_overlay()?;
        renderer.init_texture()?;
        renderer.init_shadow_resources()?;
        renderer.init_shadow_pipeline()?;
        renderer.update_texture_descriptor_sets()?;
        renderer.init_command_buffers()?;
        renderer.init_sync_objects()?;
        Ok(renderer)
    }

    // ============================================================
    // 初始化步骤
    // ============================================================

    fn init_instance(window: &Window) -> Result<Self, String> {
        let entry =
            unsafe { Entry::load().map_err(|e| format!("无法加载 Vulkan 库: {}", e))? };

        let app_info = vk::ApplicationInfo::default()
            .application_name(c"Steel Front")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"Steel Front Engine")
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_3);

        let window_extensions = {
            let display_handle = window
                .display_handle()
                .map_err(|e| format!("获取显示句柄失败: {:?}", e))?
                .as_raw();
            ash_window::enumerate_required_extensions(display_handle)
                .map_err(|e| format!("无法获取窗口所需扩展: {:?}", e))?
        };
        let mut required_extensions: Vec<RawCString> = window_extensions
            .iter()
            .map(|&p| p as RawCString)
            .collect();
        required_extensions.push(c"VK_EXT_debug_utils".as_ptr() as RawCString);
        let ext_names = required_extensions.as_slice();

        let layer_names = [c"VK_LAYER_KHRONOS_validation"];
        let layers: Vec<RawCString> = layer_names
            .iter()
            .map(|l| l.as_ptr() as RawCString)
            .collect();

        let layer_properties = unsafe {
            entry
                .enumerate_instance_layer_properties()
                .map_err(|e| format!("无法枚举实例层属性: {}", e))?
        };
        let has_validation = std::env::var("RV3D_VALIDATION").map(|v| v == "1").unwrap_or(false)
            && layer_properties.iter().any(|prop| {
                let name = unsafe { CStr::from_ptr(prop.layer_name.as_ptr()) };
                name.to_bytes_with_nul() == b"VK_LAYER_KHRONOS_validation\0"
            });
        if has_validation {
            log::info!("RV3D_VALIDATION=1 且验证层可用，已启用");
        } else {
            log::warn!("RV3D_VALIDATION 未设置或验证层不可用，将不使用验证层（驱动宽松行为）");
        }

        let instance_create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(ext_names)
            .enabled_layer_names(if has_validation { &layers } else { &[] });

        let instance = unsafe {
            entry
                .create_instance(&instance_create_info, None)
                .map_err(|e| format!("创建 Vulkan 实例失败: {}", e))?
        };

        let debug_utils = if has_validation {
            let debug_utils_loader = DebugUtils::new(&entry, &instance);
            let debug_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::ERROR

                        | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                        | vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL

                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(vulkan_debug_callback));
            let messenger = unsafe {
                debug_utils_loader
                    .create_debug_utils_messenger(&debug_create_info, None)
                    .expect("创建调试报告器失败")
            };
            Some((debug_utils_loader, messenger))
        } else {
            None
        };

        let surface = {
            let raw_handle = window
                .window_handle()
                .map_err(|e| format!("获取窗口句柄失败: {:?}", e))?
                .as_raw();
            let display_handle = window
                .display_handle()
                .map_err(|e| format!("获取显示句柄失败: {:?}", e))?
                .as_raw();
            unsafe {
                ash_window::create_surface(&entry, &instance, display_handle, raw_handle, None)
                    .map_err(|e| format!("创建 Vulkan 表面失败: {:?}", e))?
            }
        };
        let surface_loader = Surface::new(&entry, &instance);

        let physical_devices = unsafe {
            instance
                .enumerate_physical_devices()
                .map_err(|e| format!("枚举物理设备失败: {}", e))?
        };
        if physical_devices.is_empty() {
            return Err("没有找到支持 Vulkan 的 GPU".to_string());
        }

        // 🔴 2026-09-23：设备选择从"写死优先独显"改为**可指定**（`RV3D_GPU`）。
        // 起因：验证时 dGPU 可能被别的任务占着（用户在跑 AI），而本机是**双 GPU 笔记本**
        // （RTX 5060 Laptop + AMD Radeon 集显）—— 没有这个开关就只能跑在独显上。
        // 默认仍是"有窗口表面的设备里优先独显"⇒ 不设 `RV3D_GPU` 时行为与从前逐字一致。
        let candidates: Vec<(vk::PhysicalDeviceType, String, vk::PhysicalDevice)> = physical_devices
            .iter()
            .filter_map(|&device| {
                let properties = unsafe { instance.get_physical_device_properties(device) };
                let surface_support = unsafe {
                    surface_loader
                        .get_physical_device_surface_support(device, 0, surface)
                        .unwrap_or(false)
                };
                if surface_support {
                    let name = unsafe {
                        CStr::from_ptr(properties.device_name.as_ptr())
                            .to_string_lossy()
                            .to_string()
                    };
                    Some((properties.device_type, name, device))
                } else {
                    None
                }
            })
            .collect();
        if candidates.is_empty() {
            return Err("没有找到支持本窗口表面的物理设备".to_string());
        }
        let gpu_pref = parse_gpu_preference(std::env::var("RV3D_GPU").ok().as_deref());
        let pick_list: Vec<(vk::PhysicalDeviceType, String)> =
            candidates.iter().map(|(t, n, _)| (*t, n.clone())).collect();
        let picked = pick_physical_device(&pick_list, &gpu_pref).ok_or_else(|| {
            // ⚠️ 匹配不到时**报错退出**，不静默回退到独显 —— 否则"以为在核显上验的"会是假的
            let available: Vec<&str> = candidates.iter().map(|(_, n, _)| n.as_str()).collect();
            format!(
                "RV3D_GPU={:?} 没有匹配到任何设备（可选：{}；也可用 igpu/dgpu）",
                gpu_pref,
                available.join(" / ")
            )
        })?;
        let (physical_device_type, picked_name, physical_device) = candidates[picked].clone();
        let physical_device_properties =
            unsafe { instance.get_physical_device_properties(physical_device) };
        let device_name = picked_name;
        log::info!(
            "选择物理设备: {}（{:?}；RV3D_GPU={:?}）",
            device_name,
            physical_device_type,
            gpu_pref
        );
        // GPU 硬件能力探测：光追/Tensor Core/DLSS 可用性判定（仅日志，不影响初始化）
        crate::engine::gpu_caps::log_gpu_hardware_caps(&instance, physical_device, &device_name);

        let queue_families =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };

        let graphics_queue_family_index = queue_families
            .iter()
            .position(|qf| qf.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or_else(|| "没有找到图形队列族".to_string())?
            as u32;

        let present_queue_family_index = queue_families
            .iter()
            .enumerate()
            .find(|(i, _)| unsafe {
                surface_loader
                    .get_physical_device_surface_support(physical_device, *i as u32, surface)
                    .unwrap_or(false)
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到呈现队列族".to_string())?;

        let queue_priorities = [1.0_f32];
        let mut queue_indices = vec![graphics_queue_family_index];
        if present_queue_family_index != graphics_queue_family_index {
            queue_indices.push(present_queue_family_index);
        }
        queue_indices.sort();
        queue_indices.dedup();

        let queue_create_infos: Vec<vk::DeviceQueueCreateInfo> = queue_indices
            .iter()
            .map(|&index| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(index)
                    .queue_priorities(&queue_priorities)
            })
            .collect();

        let swapchain_ext_name = c"VK_KHR_swapchain";
        let mesh_shader_ext_name = c"VK_EXT_mesh_shader";
        // ---- 可选网格着色器路径：检测 VK_EXT_mesh_shader（仿 gpu_caps.rs 枚举模式）。
        //      本机 WSLg/dzn 实测扩展缺失 → mesh_enabled=false，设备创建与今天逐字节一致。
        //      支持时：扩展加入 enabled_extension_names，并把
        //      PhysicalDeviceMeshShaderFeaturesEXT(mesh_shader=true) 挂到 pNext 链
        //      （task_shader 不启用：本设计为纯 mesh 阶段，无 task 阶段）。
        let mesh_shader_available = {
            let ext_names: Vec<String> = unsafe {
                instance
                    .enumerate_device_extension_properties(physical_device)
                    .unwrap_or_default()
                    .iter()
                    .map(|e| {
                        CStr::from_ptr(e.extension_name.as_ptr())
                            .to_string_lossy()
                            .into_owned()
                    })
                    .collect()
            };
            if ext_names.iter().any(|n| n == "VK_EXT_mesh_shader") {
                let mut mesh_features = vk::PhysicalDeviceMeshShaderFeaturesEXT::default();
                let mut f2 = vk::PhysicalDeviceFeatures2::default();
                f2.p_next = &mut mesh_features as *mut _ as *mut std::ffi::c_void;
                unsafe {
                    instance.get_physical_device_features2(physical_device, &mut f2);
                }
                if mesh_features.mesh_shader == vk::TRUE {
                    log::info!("VK_EXT_mesh_shader 可用：启用可选网格着色器渲染路径");
                    true
                } else {
                    log::warn!(
                        "VK_EXT_mesh_shader 扩展存在但 meshShader 特性不可用，回退传统顶点管线"
                    );
                    false
                }
            } else {
                log::info!("VK_EXT_mesh_shader 不可用：使用传统顶点渲染路径");
                false
            }
        };

        // 设备创建：mesh 可用时仅追加扩展名与特性结构（其余字段不变）；
        // 不可用时与旧代码完全一致（enabled_extension_names=[swapchain]，pNext=null）。
        let mut device_extensions: Vec<RawCString> = vec![swapchain_ext_name.as_ptr()];
        if mesh_shader_available {
            device_extensions.push(mesh_shader_ext_name.as_ptr());
            // 2026-08-29 路径追踪基准：启用光线追踪核心扩展（ray_query 计算侧；AS 构建）
            device_extensions.push(c"VK_KHR_buffer_device_address".as_ptr());
            device_extensions.push(c"VK_KHR_deferred_host_operations".as_ptr());
            device_extensions.push(c"VK_KHR_acceleration_structure".as_ptr());
            device_extensions.push(c"VK_KHR_ray_query".as_ptr());
            device_extensions.push(c"VK_KHR_ray_tracing_pipeline".as_ptr());
        }
        let supported_features =
            unsafe { instance.get_physical_device_features(physical_device) };
        let mut physical_device_features = vk::PhysicalDeviceFeatures::default();
        physical_device_features.sampler_anisotropy = supported_features.sampler_anisotropy;

        let mut mesh_features = vk::PhysicalDeviceMeshShaderFeaturesEXT::default().mesh_shader(true);
        // 2026-08-29：RT 特性链（rayQuery + accelerationStructure features——扩展启用 ≠ 特性启用！）
        let mut rq_features = vk::PhysicalDeviceRayQueryFeaturesKHR::default();
        rq_features.ray_query = vk::TRUE;
        let mut as_features = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
        as_features.acceleration_structure = vk::TRUE;
        let mut bda_features = vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR::default();
        bda_features.buffer_device_address = vk::TRUE;
        // 链到 mesh 特性（若无 mesh 则直接挂在 device_create_info.pNext）
        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extensions)
            .enabled_features(&physical_device_features);
        let device_create_info = if mesh_shader_available {
            device_create_info.push_next(&mut mesh_features)
        } else {
            device_create_info
        };
        // RT 特性链（Ext 启用 ≠ Feature 启用；rayQuery/accelStructure 必须显式 true）
        let device_create_info = device_create_info
            .push_next(&mut as_features)
            .push_next(&mut bda_features)
            .push_next(&mut rq_features);

        {
            let mut names = Vec::new();
            let exts = unsafe { instance.enumerate_device_extension_properties(physical_device).unwrap_or_default() };
            for e in &exts {
                let n = unsafe { std::ffi::CStr::from_ptr(e.extension_name.as_ptr()) }.to_string_lossy().into_owned();
                names.push(n);
            }
            let want: Vec<String> = unsafe {
                use std::ffi::CStr;
                device_extensions.iter().map(|p| CStr::from_ptr(*p).to_string_lossy().into_owned()).collect()
            };
            log::warn!("device-create: 请求={:?} 缺失={:?}", want, want.iter().filter(|w| !names.contains(*w)).collect::<Vec<_>>());
        }
        let device = unsafe {
            instance
                .create_device(physical_device, &device_create_info, None)
                .map_err(|e| format!("创建逻辑设备失败: {}", e))?
        };

        let graphics_queue = unsafe { device.get_device_queue(graphics_queue_family_index, 0) };
        let present_queue = unsafe { device.get_device_queue(present_queue_family_index, 0) };

        let (debug_utils_loader, debug_messenger) = match debug_utils {
            Some((loader, messenger)) => (Some(loader), Some(messenger)),
            None => (None, None),
        };

        let swapchain_loader = Swapchain::new(&instance, &device);
        let mesh_shader_loader = if mesh_shader_available {
            Some(MeshShaderDevice::new(&instance, &device))
        } else {
            None
        };

        Ok(Self {
            _entry: entry,
            instance,
            debug_utils: debug_utils_loader,
            debug_messenger,
            surface_loader,
            surface,
            physical_device,
            physical_device_properties,
            graphics_queue_family_index,
            present_queue_family_index,
            device,
            graphics_queue,
            present_queue,
            swapchain_loader,
            swapchain: vk::SwapchainKHR::null(),
            swapchain_images: Vec::new(),
            swapchain_format: vk::Format::UNDEFINED,
            swapchain_extent: vk::Extent2D::default(),
            swapchain_image_views: Vec::new(),
            render_pass: vk::RenderPass::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            gun_pipeline: vk::Pipeline::null(),
            // 2026-09-05：**恢复网格着色器为唯一主渲染路径**（AGENTS.md 渲染技术路线铁律）。
            // 此前它是硬编码 false，起因是 2026-09-02 的「mesh 路径地面全黑」A/B 结论；但根因
            // 已查明是 `binding 9` 未绑定导致的乘零，而两条管线**共用同一个片元着色器**
            // （mesh 管线在 init_mesh_pipeline 里从磁盘读 assets/triangle.frag.spv），所以那
            // 从来不是 mesh 着色器的 bug，而是被误记到它头上的一个 FS 侧缺陷。binding 9 已修，
            // 止血补丁在此摘除。
            // ⚠ 顶点管线（init_pipeline / VERTEX_SHADER_WGSL）自此**冻结**：只作为缺
            //   VK_EXT_mesh_shader 时（WSLg / dzn）的兼容回退存在，不再接受功能开发，也不与
            //   mesh 路径做双份维护——新特性一律只写 mesh 路径。
            mesh_enabled: mesh_shader_available,
            device_name,
            mesh_shader: mesh_shader_loader,
            mesh_pipeline: vk::Pipeline::null(),
            mesh_pipeline_layout: vk::PipelineLayout::null(),
            mesh_max_wg_x: 1,
            void_mode: false,
            framebuffers: Vec::new(),
            // MSAA：RV3D_MSAA=1/2/4/8（默认 4x；0/1 = 关闭）
            msaa_samples: match std::env::var("RV3D_MSAA") {
                Ok(v) => match v.trim().parse::<u32>() {
                    Ok(2) => vk::SampleCountFlags::TYPE_2,
                    Ok(4) => vk::SampleCountFlags::TYPE_4,
                    Ok(8) => vk::SampleCountFlags::TYPE_8,
                    _ => vk::SampleCountFlags::TYPE_1,
                },
                Err(_) => vk::SampleCountFlags::TYPE_4,
            },
            msaa_images: Vec::new(),
            msaa_image_memory: Vec::new(),
            msaa_image_views: Vec::new(),
            command_pool: vk::CommandPool::null(),
            command_buffers: Vec::new(),
            image_available_semaphores: Vec::new(),
            render_finished_semaphores: Vec::new(),
            in_flight_fences: Vec::new(),
            acquire_timeouts: 0,
            fence_timeouts: 0,
            gpu_stalled: false,
            present_mode_override: None,
            current_frame: 0,
            max_frames_in_flight: 2,
            last_frame_us: 0,
            last_cull_us: 0,
            vertex_buffer: vk::Buffer::null(),
            vertex_buffer_memory: vk::DeviceMemory::null(),
            index_buffer: vk::Buffer::null(),
            index_buffer_memory: vk::DeviceMemory::null(),
            far_vertex_buffer: vk::Buffer::null(),
            far_vertex_buffer_memory: vk::DeviceMemory::null(),
            far_index_buffer: vk::Buffer::null(),
            far_index_buffer_memory: vk::DeviceMemory::null(),
            ground_vertex_buffer: vk::Buffer::null(),
            ground_vertex_buffer_memory: vk::DeviceMemory::null(),
            ground_index_buffer: vk::Buffer::null(),
            ground_index_buffer_memory: vk::DeviceMemory::null(),
            sphere_vertex_buffer: vk::Buffer::null(),
            sphere_vertex_buffer_memory: vk::DeviceMemory::null(),
            sphere_index_buffer: vk::Buffer::null(),
            sphere_index_buffer_memory: vk::DeviceMemory::null(),
            sphere_index_count: 0,
            cylinder_vertex_buffer: vk::Buffer::null(),
            cylinder_vertex_buffer_memory: vk::DeviceMemory::null(),
            cylinder_index_buffer: vk::Buffer::null(),
            cylinder_index_buffer_memory: vk::DeviceMemory::null(),
            cylinder_index_count: 0,
            terrain_lods: Vec::new(),
            instance_buffers: Vec::new(),
            instance_buffers_memory: Vec::new(),
            instance_mapped: Vec::new(),
            instances: Vec::new(),
            culled_scratch: Vec::new(),
            seg_near_counts: Vec::new(),
            seg_far_counts: Vec::new(),
            instance_radii: Vec::with_capacity(INSTANCE_COUNT as usize),
            instance_center_x: Vec::with_capacity(INSTANCE_COUNT as usize),
            instance_center_y: Vec::with_capacity(INSTANCE_COUNT as usize),
            instance_center_z: Vec::with_capacity(INSTANCE_COUNT as usize),
            markers: Vec::new(),
            last_marker_near: 0,
            last_marker_far: 0,
            npc_box_parts: Vec::new(),
        npc_cyl_parts: Vec::new(),
        npc_sph_parts: Vec::new(),
            last_npc_box_near: 0,
            last_npc_box_far: 0,
            last_npc_cyl_near: 0,
            last_npc_cyl_far: 0,
            last_npc_sph_near: 0,
            last_npc_sph_far: 0,
            emissive_markers: Vec::new(),
            last_emissive_near: 0,
            last_emissive_far: 0,
            last_perf_log: Instant::now(),
            frame_count: 0,
            perf_window_start: Instant::now(),
            stage_wait_fence_us: 0,
            stage_acquire_us: 0,
            stage_terrain_us: 0,
            stage_record_us: 0,
            stage_submit_us: 0,
            stage_present_us: 0,
            depth_images: Vec::new(),
            depth_images_memory: Vec::new(),
            depth_image_views: Vec::new(),
            shadow_image: vk::Image::null(),
            shadow_image_memory: vk::DeviceMemory::null(),
            shadow_image_view: vk::ImageView::null(),
            shadow_sampler: vk::Sampler::null(),
            shadow_render_pass: vk::RenderPass::null(),
            shadow_framebuffer: vk::Framebuffer::null(),
            shadow_pipeline_layout: vk::PipelineLayout::null(),
            shadow_pipeline: vk::Pipeline::null(),
            shadow_ubo_buffers: Vec::new(),
            shadow_ubo_memory: Vec::new(),
            shadow_ubo_mapped: Vec::new(),
            shadow_descriptor_set_layout: vk::DescriptorSetLayout::null(),
            shadow_descriptor_sets: Vec::new(),
            // ---- 新增字段初始值 ----
            descriptor_set_layout: vk::DescriptorSetLayout::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            descriptor_sets: Vec::new(),
            uniform_buffers: Vec::new(),
            uniform_buffers_memory: Vec::new(),
            uniform_mapped: Vec::new(),
            light_uniform_buffers: Vec::new(),
            light_uniform_buffers_memory: Vec::new(),
            light_uniform_mapped: Vec::new(),
            texture_image: vk::Image::null(),
            texture_image_memory: vk::DeviceMemory::null(),
            texture_image_view: vk::ImageView::null(),
            texture_sampler: vk::Sampler::null(),
            skin_marker_image: vk::Image::null(),
            skin_marker_memory: vk::DeviceMemory::null(),
            skin_marker_image_view: vk::ImageView::null(),
            skin_npc_image: vk::Image::null(),
            skin_npc_memory: vk::DeviceMemory::null(),
            skin_npc_image_view: vk::ImageView::null(),
            ground_detail_image: vk::Image::null(),
            ground_detail_memory: vk::DeviceMemory::null(),
            ground_detail_image_view: vk::ImageView::null(),
            // 2026-08-22：默认启用（RV3D_SKIN_TEX=0 关闭纯色回退）——障碍需要表面细节
            skin_tex_enabled: std::env::var("RV3D_SKIN_TEX").as_deref() != Ok("0"),
            texture_anisotropy_enabled: physical_device_features.sampler_anisotropy != 0,
            hud_pipeline: vk::Pipeline::null(),
            hud_overlay_pipeline: vk::Pipeline::null(),
            hud_pipeline_layout: vk::PipelineLayout::null(),
            hud_vertex_buffer: vk::Buffer::null(),
            hud_vertex_buffer_memory: vk::DeviceMemory::null(),
            hud_mapped: std::ptr::null_mut(),
            hud_vertex_count: 0,
            hud_render_pass: vk::RenderPass::null(),
            hud_framebuffers: Vec::new(),
            hud_capacity_quads: 4096,
            gun_vertex_buffer: vk::Buffer::null(),
            gun_vertex_buffer_memory: vk::DeviceMemory::null(),
            gun_index_buffer: vk::Buffer::null(),
            gun_index_buffer_memory: vk::DeviceMemory::null(),
            gun_mapped: std::ptr::null_mut(),
            gun_vertex_count: 0,
            gun_index_count: 0,
            gun_buffer_capacity_verts: 0,
            gun_buffer_capacity_idx: 0,
            soldier_vertex_buffer: vk::Buffer::null(),
            soldier_vertex_buffer_memory: vk::DeviceMemory::null(),
            soldier_index_buffer: vk::Buffer::null(),
            soldier_index_buffer_memory: vk::DeviceMemory::null(),
            soldier_vertex_count: 0,
            soldier_index_count: 0,
            soldier_parts: Vec::new(),
            npc_cap_warned: false,
            pt_box_cap_warned: false,
            soldier_drawn: 0,
            prop_vertex_buffer: vk::Buffer::null(),
            prop_vertex_memory: vk::DeviceMemory::null(),
            prop_index_buffer: vk::Buffer::null(),
            prop_index_memory: vk::DeviceMemory::null(),
            prop_mapped: std::ptr::null_mut(),
            prop_vertex_count: 0,
            prop_index_count: 0,
            prop_capacity_verts: 0,
            prop_capacity_idx: 0,
            prop_bins: Vec::new(),
            prop_sh_vertex_buffer: vk::Buffer::null(),
            prop_sh_vertex_memory: vk::DeviceMemory::null(),
            prop_sh_index_buffer: vk::Buffer::null(),
            prop_sh_index_memory: vk::DeviceMemory::null(),
            prop_sh_index_count: 0,
            prop_sh_bins: Vec::new(),
            shadow_lod: std::env::var("RV3D_SHADOW_LOD").as_deref() != Ok("0"),
            frame_frustum: [[0.0f32; 4]; 6],
            last_near_count: 0,
            last_far_count: 0,
            last_terrain_lod_name: "high",
            light_data: LightUniform::default(),
            quality: QualityPreset::DEFAULT,
            pt_live_enabled: false,
            pt_resident: None,
            pt_params: crate::engine::ray_tracer::PtParams::default(),
            pt_box_count: 0,
            pt_scene_sig: 0,
            pt_prop_key: (0, 0, 0),
            prop_attr_buf: vk::Buffer::null(),
            prop_attr_mem: vk::DeviceMemory::null(),
            prop_attr_tris: 0,
            pt_img: vk::Image::null(),
            pt_img_mem: vk::DeviceMemory::null(),
            pt_view: vk::ImageView::null(),
            pt_pipeline: vk::Pipeline::null(),
            pt_layout: vk::PipelineLayout::null(),
            pt_setl: vk::DescriptorSetLayout::null(),
            pt_pool: vk::DescriptorPool::null(),
            pt_dset: vk::DescriptorSet::null(),
            pt_module: vk::ShaderModule::null(),
            pt_acc: vk::Image::null(),
            pt_acc_mem: vk::DeviceMemory::null(),
            pt_acc_view: vk::ImageView::null(),
            pt_frame: std::cell::Cell::new(0),
            pt_spp_target: 256,
            pt_reset: std::cell::Cell::new(true),
            pt_view_sig: std::cell::Cell::new(0),
            pt_size: (64, 64),
            pt_move_base_cam: std::cell::Cell::new([0.0; 3]),
            pt_move_base_fwd: std::cell::Cell::new([0.0; 3]),

            screenshot_request: None,
            screenshot_buffers: Vec::new(),
            screenshot_buffers_memory: Vec::new(),
            screenshot_fences: Vec::new(),
        })
    }

    fn init_swapchain(&mut self) -> Result<(), String> {
        let surface_capabilities = unsafe {
            self.surface_loader
                .get_physical_device_surface_capabilities(self.physical_device, self.surface)
                .map_err(|e| format!("获取表面能力失败: {}", e))?
        };

        let surface_formats = unsafe {
            self.surface_loader
                .get_physical_device_surface_formats(self.physical_device, self.surface)
                .map_err(|e| format!("获取表面格式失败: {}", e))?
        };
        let format = surface_formats
            .iter()
            .find(|f| {
                f.format == vk::Format::B8G8R8A8_SRGB
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .unwrap_or(&surface_formats[0]);

        let present_modes = unsafe {
            self.surface_loader
                .get_physical_device_surface_present_modes(self.physical_device, self.surface)
                .map_err(|e| format!("获取呈现模式失败: {}", e))?
        };
        // 呈现模式可被 RV3D_PRESENT_MODE 覆盖（immediate/mailbox/fifo），性能探针对比用。
        //
        // 🔴 2026-09-13：**补上 `mailbox`**。此前只有 immediate/fifo，而默认落在
        // IMMEDIATE —— 屏幕上是**持续撕裂**，在快速转视角时正好读成"残影/鬼影"
        // （用户 2026-09-13 报告"晃画面和跑动时枪有非常明显的残影"）。
        // 抓帧抓不到它：`PrintWindow` 拿的是已合成的完整帧，撕裂只发生在显示器上。
        //
        // 三种模式的取舍（原注释只记了前两条）：
        //   * FIFO   —— 独显直连下等不到 vblank 中断 ⇒ 主循环冻结（2026-08-23）
        //   * MAILBOX —— 不撕裂且不阻塞；原注释记的"笔记本混合切换时 device lost"
        //                是 MUX 切换场景，独显直连/手动模式下不触发
        //   * IMMEDIATE —— 最稳但撕裂
        // ⇒ 默认仍保持 IMMEDIATE（基准/压力测试要的是最稳 + 全速），
        //   **玩家路径由 `SteelFront.bat` 显式设成 mailbox**（见该文件）。
        let preferred = match self.present_mode_override {
            Some(m) => m,
            None => match std::env::var("RV3D_PRESENT_MODE").as_deref() {
                Ok("immediate") => vk::PresentModeKHR::IMMEDIATE,
                Ok("fifo") => vk::PresentModeKHR::FIFO,
                Ok("mailbox") => vk::PresentModeKHR::MAILBOX,
                _ => vk::PresentModeKHR::IMMEDIATE,
            },
        };
        let present_mode = present_modes
            .iter()
            .find(|&&m| m == preferred)
            .copied()
            .unwrap_or(vk::PresentModeKHR::FIFO);

        let extent = if surface_capabilities.current_extent.width != u32::MAX {
            surface_capabilities.current_extent
        } else {
            vk::Extent2D { width: 1280, height: 720 }
        };

        let image_count = {
            let mut count = surface_capabilities.min_image_count + 1;
            if surface_capabilities.max_image_count != 0 {
                count = count.min(surface_capabilities.max_image_count);
            }
            count
        };

        let mut queue_family_indices = vec![self.graphics_queue_family_index];
        if self.present_queue_family_index != self.graphics_queue_family_index {
            queue_family_indices.push(self.present_queue_family_index);
        }
        let sharing_mode = if queue_family_indices.len() > 1 {
            vk::SharingMode::CONCURRENT
        } else {
            vk::SharingMode::EXCLUSIVE
        };

        // COLOR_ATTACHMENT | TRANSFER_SRC：截图读回需要把 swapchain 图像作为
        // TRANSFER 源拷贝到 staging buffer（vkCmdCopyImageToBuffer 的 VUID 要求）。
        //
        // 🔴 **TRANSFER_DST（2026-09-15 补，未结案 #2 的直接证据）**：PT 实时通路把
        // `pt_img` blit 到 swapchain 图像（见本文件 PT present 段：先 barrier 到
        // `TRANSFER_DST_OPTIMAL`，再 `cmd_blit_image`），**那一步要求目标图像带 TRANSFER_DST**。
        // 缺它时开启验证层（`RV3D_VALIDATION=1`）当场报两条：
        //   * `VUID-vkCmdBlitImage-dstImage-00224`（dstImage 缺 TRANSFER_DST）
        //   * `VUID-VkImageMemoryBarrier-oldLayout-01213`（barrier 到 TRANSFER_DST_OPTIMAL）
        // 这正是"设 `pt_enable=true` 一启动就 `0xC0000005`"的来源：**非法用法驱动不报错，
        // 崩在别处**。PT 之前一直开不起来，所以这条从来没被验证层看见过。
        let mut swapchain_usage =
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC;
        if surface_capabilities
            .supported_usage_flags
            .contains(vk::ImageUsageFlags::TRANSFER_DST)
        {
            swapchain_usage |= vk::ImageUsageFlags::TRANSFER_DST;
        } else {
            log::warn!(
                "surface 不支持 TRANSFER_DST：PT 实时通路无法 blit 到交换链（PT 打开时画面会异常）"
            );
        }
        let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(swapchain_usage)
            .image_sharing_mode(sharing_mode)
            .queue_family_indices(&queue_family_indices)
            .pre_transform(surface_capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);

        self.swapchain = unsafe {
            self.swapchain_loader
                .create_swapchain(&swapchain_create_info, None)
                .map_err(|e| format!("创建交换链失败: {}", e))?
        };
        self.swapchain_images = unsafe {
            self.swapchain_loader
                .get_swapchain_images(self.swapchain)
                .map_err(|e| format!("获取交换链图像失败: {}", e))?
        };
        self.swapchain_format = format.format;
        self.swapchain_extent = extent;
        // 诊断（2026-08-15）：surface current_extent vs 最终 swapchain extent ——
        // 若 current_extent 是窗口逻辑尺寸而实际物理尺寸不同，画面会 1:1 错位
        log::info!(
            "swapchain diag: current_extent={}x{} final={}x{} flags={:?} min_images={} usage={:?}",
            surface_capabilities.current_extent.width,
            surface_capabilities.current_extent.height,
            extent.width,
            extent.height,
            swapchain_create_info.flags,
            swapchain_create_info.min_image_count,
            swapchain_create_info.image_usage
        );

        self.swapchain_image_views = self
            .swapchain_images
            .iter()
            .map(|&image| {
                let subresource_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1);
                let view_create_info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(self.swapchain_format)
                    .subresource_range(subresource_range);
                unsafe {
                    self.device
                        .create_image_view(&view_create_info, None)
                        .map_err(|e| format!("创建图像视图失败: {e}"))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        log::info!(
            "交换链初始化完成: {}x{}, 格式: {:?}, 图像数: {}, present_mode: {:?}",
            extent.width,
            extent.height,
            format.format,
            image_count,
            present_mode
        );
        Ok(())
    }

    fn init_render_pass(&mut self) -> Result<(), String> {
        let msaa = self.msaa_samples;
        let color_attachment = vk::AttachmentDescription::default()
            .format(self.swapchain_format)
            .samples(msaa)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE) // 经 resolve 输出，自身不保留
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

        let color_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let color_attachment_refs = [color_attachment_ref];

        // 解析附件：MSAA 颜色 → 交换链图像（TYPE_1，最终呈现）
        let resolve_attachment = vk::AttachmentDescription::default()
            .format(self.swapchain_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
        let resolve_attachment_ref = vk::AttachmentReference::default()
            .attachment(1)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

        // 深度附件（D32_SFLOAT，与颜色同采样数）
        let depth_attachment = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(msaa)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let depth_attachment_ref = vk::AttachmentReference::default()
            .attachment(2)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

        let resolve_attachment_refs = [resolve_attachment_ref];
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment_refs)
            .resolve_attachments(&resolve_attachment_refs)
            .depth_stencil_attachment(&depth_attachment_ref);
        let subpasses = [subpass];
        let attachments = [color_attachment, resolve_attachment, depth_attachment];

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            );
        let dependencies = [dependency];

        let render_pass_create_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);

        self.render_pass = unsafe {
            self.device
                .create_render_pass(&render_pass_create_info, None)
                .map_err(|e| format!("创建渲染流程失败: {}", e))?
        };
        Ok(())
    }

    /// MSAA 颜色附件（每交换链图像一个，samples=msaa_samples）。
    /// 主 pass 渲染到该附件，subpass resolve 输出到交换链图像；MSAA 关闭时跳过。
    fn init_msaa_resources(&mut self) -> Result<(), String> {
        self.msaa_images.clear();
        self.msaa_image_memory.clear();
        self.msaa_image_views.clear();
        if self.msaa_samples == vk::SampleCountFlags::TYPE_1 {
            return Ok(());
        }
        for _ in 0..self.swapchain_images.len() {
            let image_info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(self.swapchain_format)
                .extent(vk::Extent3D {
                    width: self.swapchain_extent.width,
                    height: self.swapchain_extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(self.msaa_samples)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            let image = unsafe {
                self.device
                    .create_image(&image_info, None)
                    .map_err(|e| format!("创建 MSAA 颜色 Image 失败: {}", e))?
            };
            let mem_reqs = unsafe { self.device.get_image_memory_requirements(image) };
            let mem_type = self.pick_memory_type(mem_reqs, true)?;
            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_reqs.size)
                .memory_type_index(mem_type);
            let memory = unsafe {
                self.device
                    .allocate_memory(&alloc_info, None)
                    .map_err(|e| format!("分配 MSAA 颜色内存失败: {}", e))?
            };
            unsafe { self.device.bind_image_memory(image, memory, 0) }
                .map_err(|e| format!("绑定 MSAA 颜色内存失败: {}", e))?;
            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(self.swapchain_format)
                .components(vk::ComponentMapping {
                    r: vk::ComponentSwizzle::IDENTITY,
                    g: vk::ComponentSwizzle::IDENTITY,
                    b: vk::ComponentSwizzle::IDENTITY,
                    a: vk::ComponentSwizzle::IDENTITY,
                })
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            let view = unsafe {
                self.device
                    .create_image_view(&view_info, None)
                    .map_err(|e| format!("创建 MSAA 颜色 ImageView 失败: {}", e))?
            };
            self.msaa_images.push(image);
            self.msaa_image_memory.push(memory);
            self.msaa_image_views.push(view);
        }
        Ok(())
    }

    /// 为每个交换链图像创建深度缓冲（D32_SFLOAT Image + depth aspect ImageView）
    fn init_depth_resources(&mut self) -> Result<(), String> {
        let depth_format = vk::Format::D32_SFLOAT;
        self.depth_images.clear();
        self.depth_images_memory.clear();
        self.depth_image_views.clear();

        for _ in 0..self.swapchain_images.len() {
            let image_info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(depth_format)
                .extent(vk::Extent3D {
                    width: self.swapchain_extent.width,
                    height: self.swapchain_extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(self.msaa_samples)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            let image = unsafe {
                self.device
                    .create_image(&image_info, None)
                    .map_err(|e| format!("创建深度 Image 失败: {}", e))?
            };

            let mem_reqs = unsafe { self.device.get_image_memory_requirements(image) };
            let memory_type = self.pick_memory_type(mem_reqs, true)?;
            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_reqs.size)
                .memory_type_index(memory_type);
            let memory = unsafe {
                self.device
                    .allocate_memory(&alloc_info, None)
                    .map_err(|e| format!("分配深度 Image 内存失败: {}", e))?
            };
            unsafe {
                self.device
                    .bind_image_memory(image, memory, 0)
                    .map_err(|e| format!("绑定深度 Image 内存失败: {}", e))?;
            }

            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(depth_format)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::DEPTH)
                        .base_mip_level(0)
                        .level_count(1)
                        .base_array_layer(0)
                        .layer_count(1),
                );
            let view = unsafe {
                self.device
                    .create_image_view(&view_info, None)
                    .map_err(|e| format!("创建深度 Image View 失败: {}", e))?
            };

            self.depth_images.push(image);
            self.depth_images_memory.push(memory);
            self.depth_image_views.push(view);
        }
        log::info!(
            "深度缓冲创建完成: {} 张 {}x{} D32_SFLOAT",
            self.depth_images.len(),
            self.swapchain_extent.width,
            self.swapchain_extent.height
        );
        Ok(())
    }

    // ============================================================
    // 新增：初始化 Descriptor（Uniform Buffer + 布局 + 池 + 分配）
    // ============================================================
    /// 创建并持久映射一个 HOST_VISIBLE | HOST_COHERENT 的 Uniform Buffer
    fn create_uniform_buffer(
        &self,
        size: u64,
    ) -> Result<(vk::Buffer, vk::DeviceMemory, *mut std::ffi::c_void), String> {
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buffer = unsafe {
            self.device
                .create_buffer(&buffer_info, None)
                .map_err(|e| format!("创建 Uniform Buffer 失败: {}", e))?
        };

        let mem_requirements = unsafe {
            self.device.get_buffer_memory_requirements(buffer)
        };

        let mem_properties = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };

        let memory_type = mem_properties
            .memory_types
            .iter()
            .enumerate()
            .find(|(i, mem_type)| {
                let type_mask = 1 << i;
                (mem_requirements.memory_type_bits & type_mask) != 0
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT)
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到合适的内存类型（Uniform Buffer）".to_string())?;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_requirements.size)
            .memory_type_index(memory_type);

        let buffer_memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配 Uniform Buffer 内存失败: {}", e))?
        };

        unsafe {
            self.device
                .bind_buffer_memory(buffer, buffer_memory, 0)
                .map_err(|e| format!("绑定 Uniform Buffer 内存失败: {}", e))?;
        }

        let mapped = unsafe {
            self.device
                .map_memory(buffer_memory, 0, size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射 Uniform Buffer 内存失败: {}", e))?
        };

        Ok((buffer, buffer_memory, mapped))
    }

    fn init_descriptors(&mut self) -> Result<(), String> {
        let max_frames = self.max_frames_in_flight;

        // ---- 1. 创建 Descriptor Set Layout ----
        // 描述：binding=0, 类型=UNIFORM_BUFFER, 阶段=VERTEX（mesh 路径额外 +MESH_EXT）
        let ubo_stage_flags = if self.mesh_enabled {
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT
        } else {
            vk::ShaderStageFlags::VERTEX
        };
        let ubo_layout_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(ubo_stage_flags);
        // 纹理采样（贴图 binding=1，采样器 binding=3，均只在 Fragment 阶段使用）
        let sampled_image_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 实例 storage buffer（binding=2，Vertex 阶段读取；mesh 路径额外 +MESH_EXT）
        let storage_stage_flags = if self.mesh_enabled {
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT
        } else {
            vk::ShaderStageFlags::VERTEX
        };
        let storage_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(storage_stage_flags);
        let sampler_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(3)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 光照 Uniform（binding=4，Fragment 阶段读取；默认全零 = 关闭）
        let light_ubo_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(LIGHT_UBO_BINDING)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 阴影贴图（binding=5 SAMPLED_IMAGE、binding=6 SAMPLER，均 Fragment 阶段采样）
        let shadow_map_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(5)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let shadow_sampler_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(6)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // marker/NPC 程序化皮肤纹理（binding=7/8 SAMPLED_IMAGE，Fragment 采样；
        // RV3D_SKIN_TEX=1 启用，缺省 0 纯色回退。绑定号必须与 build.rs WGSL 同步）
        let marker_skin_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(7)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let npc_skin_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(8)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 地面微细节层（binding=9；build.rs 片元 `ground_detail_tex`，**无条件采样**，
        // 不受 RV3D_SKIN_TEX 门控）。漏掉这个绑定 = 采样恒 0 = 相机周边地面纯黑，
        // 详见字段 `ground_detail_image` 的注释。
        let ground_detail_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(GROUND_DETAIL_BINDING)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let bindings = [
            ubo_layout_binding,
            sampled_image_binding,
            storage_binding,
            sampler_binding,
            light_ubo_binding,
            shadow_map_binding,
            shadow_sampler_binding,
            marker_skin_binding,
            npc_skin_binding,
            ground_detail_binding,
        ];

        let layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&bindings);

        self.descriptor_set_layout = unsafe {
            self.device
                .create_descriptor_set_layout(&layout_info, None)
                .map_err(|e| format!("创建 Descriptor Set Layout 失败: {}", e))?
        };
        // binding 0 = view/proj UBO；binding 1 = 贴图；binding 2 = 实例 storage buffer；
        // 原采样器 binding 2 顺延到 binding 3（与 WGSL 一致）。
        log::info!(
            "Descriptor Set Layout: binding 0 = UBO(view/proj), binding 1 = 贴图, binding 2 = 实例 STORAGE_BUFFER, binding 3 = 采样器"
        );

        // ---- 2. 创建 Uniform Buffer（每帧一个）----
        let buffer_size = std::mem::size_of::<CameraUniform>() as u64;

        for _ in 0..max_frames {
            let buffer_info = vk::BufferCreateInfo::default()
                .size(buffer_size)
                .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);

            let buffer = unsafe {
                self.device
                    .create_buffer(&buffer_info, None)
                    .map_err(|e| format!("创建 Uniform Buffer 失败: {}", e))?
            };

            let mem_requirements = unsafe {
                self.device.get_buffer_memory_requirements(buffer)
            };

            let mem_properties = unsafe {
                self.instance
                    .get_physical_device_memory_properties(self.physical_device)
            };

            let memory_type = mem_properties
                .memory_types
                .iter()
                .enumerate()
                .find(|(i, mem_type)| {
                    let type_mask = 1 << i;
                    (mem_requirements.memory_type_bits & type_mask) != 0
                        && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                        && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT)
                })
                .map(|(i, _)| i as u32)
                .ok_or_else(|| "没有找到合适的内存类型（Uniform Buffer）".to_string())?;

            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_requirements.size)
                .memory_type_index(memory_type);

            let buffer_memory = unsafe {
                self.device
                    .allocate_memory(&alloc_info, None)
                    .map_err(|e| format!("分配 Uniform Buffer 内存失败: {}", e))?
            };

            unsafe {
                self.device
                    .bind_buffer_memory(buffer, buffer_memory, 0)
                    .map_err(|e| format!("绑定 Uniform Buffer 内存失败: {}", e))?;
            }

            // 持久映射（map 一次，之后每帧直接写）
            let mapped = unsafe {
                self.device
                    .map_memory(buffer_memory, 0, buffer_size, vk::MemoryMapFlags::empty())
                    .map_err(|e| format!("映射 Uniform Buffer 内存失败: {}", e))?
            };

            self.uniform_buffers.push(buffer);
            self.uniform_buffers_memory.push(buffer_memory);
            self.uniform_mapped.push(mapped);
        }

        // ---- 2b. 创建光照 Uniform Buffer（每帧一份，默认全零 = 光照关闭）----
        let light_ubo_size = std::mem::size_of::<LightUniform>() as u64;
        for _ in 0..max_frames {
            let (buffer, buffer_memory, mapped) = self.create_uniform_buffer(light_ubo_size)?;
            self.light_uniform_buffers.push(buffer);
            self.light_uniform_buffers_memory.push(buffer_memory);
            self.light_uniform_mapped.push(mapped);
        }

        // ---- 3. 创建 Descriptor Pool ----
        let pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count((max_frames * 3) as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                // binding 1 地面贴图 + binding 5 阴影图 + binding 7/8 marker/NPC 皮肤纹理
                // + binding 9 地面微细节层（缺一个 = 该 set 分配失败 → 启动即报错）
                .descriptor_count((max_frames * 5) as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count((max_frames * 2) as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count((max_frames * 2) as u32),
        ];

        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets((max_frames * 2) as u32);

        self.descriptor_pool = unsafe {
            self.device
                .create_descriptor_pool(&pool_info, None)
                .map_err(|e| format!("创建 Descriptor Pool 失败: {}", e))?
        };

        // ---- 4. 分配 Descriptor Sets ----
        let layouts: Vec<vk::DescriptorSetLayout> = (0..max_frames)
            .map(|_| self.descriptor_set_layout)
            .collect();

        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);

        self.descriptor_sets = unsafe {
            self.device
                .allocate_descriptor_sets(&alloc_info)
                .map_err(|e| format!("分配 Descriptor Sets 失败: {}", e))?
        };

        // ---- 5. 更新 Descriptor Sets（把 buffer 绑到 set 上）----
        for i in 0..max_frames {
            let buffer_info = vk::DescriptorBufferInfo::default()
                .buffer(self.uniform_buffers[i])
                .offset(0)
                .range(buffer_size);
            let buffer_infos = [buffer_info];

            let descriptor_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buffer_infos);
            let descriptor_writes = [descriptor_write];

            unsafe {
                self.device.update_descriptor_sets(&descriptor_writes, &[]);
            }

            // 实例 storage buffer（每帧 set 指向该帧自己的 buffer，消除读写竞态）
            let instance_info = vk::DescriptorBufferInfo::default()
                .buffer(self.instance_buffers[i])
                .offset(0)
                // 范围必须覆盖到最高槽位（道具 identity 槽），否则 shader 读该 slot 会
                // 越界——驱动不报错，只返回全零，几何会静默消失。见 INSTANCE_BUFFER_ELEMS。
                .range(std::mem::size_of::<InstanceData>() as u64 * INSTANCE_BUFFER_ELEMS);
            let instance_infos = [instance_info];
            let instance_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(2)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&instance_infos);
            let instance_writes = [instance_write];
            unsafe {
                self.device.update_descriptor_sets(&instance_writes, &[]);
            }

            // 光照 Uniform（默认全零 = 关闭）
            let light_info = vk::DescriptorBufferInfo::default()
                .buffer(self.light_uniform_buffers[i])
                .offset(0)
                .range(light_ubo_size);
            let light_infos = [light_info];
            let light_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(LIGHT_UBO_BINDING)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&light_infos);
            let light_writes = [light_write];
            unsafe {
                self.device.update_descriptor_sets(&light_writes, &[]);
            }
        }

        log::info!("Descriptor 初始化完成（{} 帧）", max_frames);
        Ok(())
    }

    /// ⛔ 传统 VERTEX 管线【已冻结维护】（2026-08-16）：仅 WSLg/dzn 无 VK_EXT_mesh_shader
    /// 时回退使用（地形 LOD 网格 + 地面实例场）。新渲染功能一律走 mesh 路径
    /// （init_mesh_pipeline），本管线不再新增功能。
    fn init_pipeline(&mut self) -> Result<(), String> {
        // 2026-08-28 终极修正：使用 build.rs 内嵌 SPIR-V（OUT_DIR/shaders.rs 常量），
        // 不再加载外置 assets/triangle.*.spv（两者曾长期不同步：外置为旧版，color 通道被 UV 顶替）
        let vs_spirv = crate::shaders::VS_SPIRV.to_vec();
        let fs_spirv = crate::shaders::FS_SPIRV.to_vec();
        let vs_module = self.create_shader_module(&vs_spirv)?;
        let fs_module = self.create_shader_module(&fs_spirv)?;

        let vs_entry = c"vs_main";
        let fs_entry = c"fs_main";

        let vs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs_module)
            .name(vs_entry);
        let fs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_module)
            .name(fs_entry);
        let shader_stages = [vs_stage, fs_stage];

        let vertex_binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<Vertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);

        let vertex_attributes = [
            // location 0: position vec3
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, pos) as u32),
            // location 1: color vec3
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, color) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, uv) as u32),
        ];

        log::info!(
            "gun-attr: stride={} pos@{} color@{} uv@{}",
            std::mem::size_of::<Vertex>(),
            std::mem::offset_of!(Vertex, pos),
            std::mem::offset_of!(Vertex, color),
            std::mem::offset_of!(Vertex, uv)
        );
        let vertex_bindings = [vertex_binding];
        let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vertex_bindings)
            .vertex_attribute_descriptions(&vertex_attributes);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        let viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.swapchain_extent.width as f32)
            .height(self.swapchain_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(self.swapchain_extent);
        let viewports = [viewport];
        let scissors = [scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);

        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(self.msaa_samples);

        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            // 2026-09-04：主管线**打开深度测试**。此前它是 false，意味着当前唯一在跑的
            // legacy 路径完全没有深度遮挡，楼与楼只按绘制顺序互相穿透（mesh 管线一直是
            // 开的，所以这个差异只有在两条管线对比时才看得出来）。
            // 枪模对"不测深度"的依赖已拆到下面的 `gun_depth_stencil`，因此这里可以安全打开。
            // 保持 LESS_OR_EQUAL：项目里有大量刻意共面/零厚度的装饰件，用 LESS 会让它们
            // 被自己先前写入的深度挡住而闪烁。
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_depth_bounds(0.0)
            .max_depth_bounds(1.0);

        // 枪模专用管线：不测深度、**也不写深度**。不写是必要的——否则枪模会把自身深度
        // 留在缓冲里，之后与它重叠的 HUD/粒子反而会被一把枪挡住。
        let gun_depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_depth_bounds(0.0)
            .max_depth_bounds(1.0);

        let color_write_mask = vk::ColorComponentFlags::R

            | vk::ColorComponentFlags::G
            | vk::ColorComponentFlags::B
            | vk::ColorComponentFlags::A;
        // 2026-08-15：主 pass 开启 alpha 混合——现有几何 color.a 恒为 1.0（不受影响），
        // 自发光实体（爆炸等）设 alpha<1 即实现半透明（球形火光/冲击波可透出背景）
        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(color_write_mask)
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD);
        let color_blend_attachments = [color_blend_attachment];
        let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&color_blend_attachments);

        // ---- 管线布局：挂上 descriptor_set_layout ----
        let set_layouts = [self.descriptor_set_layout];
        let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&[]);

        self.pipeline_layout = unsafe {
            self.device
                .create_pipeline_layout(&pipeline_layout_create_info, None)
                .map_err(|e| format!("创建管线布局失败: {}", e))?
        };

        // 动态 viewport/scissor：resize 后每帧用当前 swapchain_extent 重设，
        // 避免全屏/窗口变化后画面卡在旧尺寸左上角（2026-08-15 修复）
        let dynamic_states = [
            vk::DynamicState::VIEWPORT,
            vk::DynamicState::SCISSOR,
        ];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&dynamic_states);

        let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .dynamic_state(&dynamic_state)
            .color_blend_state(&color_blend_state)
            .layout(self.pipeline_layout)
            .render_pass(self.render_pass)
            .subpass(0);

        self.pipeline = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_create_info], None)
                .map_err(|(_, e)| format!("创建图形管线失败: {}", e))?
                .remove(0)
        };

        // 枪模管线：除 depth 状态外与主管线逐字段相同。必须在销毁 shader module **之前**
        // 创建——create_graphics_pipelines 是同步的，模块在返回后即可释放。
        let gun_pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&gun_depth_stencil)
            .dynamic_state(&dynamic_state)
            .color_blend_state(&color_blend_state)
            .layout(self.pipeline_layout)
            .render_pass(self.render_pass)
            .subpass(0);

        self.gun_pipeline = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[gun_pipeline_create_info], None)
                .map_err(|(_, e)| format!("创建枪模管线失败: {}", e))?
                .remove(0)
        };

        unsafe {
            self.device.destroy_shader_module(vs_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }

        self.create_vertex_buffer()?;
        self.create_index_buffer()?;
        self.create_far_geometry()?;
        self.create_ground_geometry()?;
        self.create_sphere_geometry()?;
        self.create_cylinder_geometry()?;
        self.create_terrain_lods()?;
        log::info!("图形管线创建完成");
        Ok(())
    }

    /// 可选网格着色器管线（VK_EXT_mesh_shader）：
    /// - 阶段 = MESH_EXT + FRAGMENT（片元着色器与主管线同一模块，原样复用）；
    /// - 无 vertex input state / input assembly state（VK_EXT_mesh_shader 要求二者为 NULL）；
    /// - rasterization（Back cull + CLOCKWISE）/ depth / blend / viewport 与主管线完全一致；
    /// - pipeline layout 复用同一 descriptor set layout，仅追加 MESH_EXT push constant
    ///   （base_slot，16 字节）；传统管线共用同一 descriptor set layout 不受影响。
    /// mesh_enabled=false（本机 WSLg/dzn）时直接返回，不加载 mesh.spv、不创建任何资源。
    fn init_mesh_pipeline(&mut self) -> Result<(), String> {
        if !self.mesh_enabled {
            return Ok(());
        }
        // maxMeshWorkGroupCount[0]：地面场 65536 workgroup 超最低保证 65535，须分块绘制。
        let mut mesh_props = vk::PhysicalDeviceMeshShaderPropertiesEXT::default();
        let mut p2 = vk::PhysicalDeviceProperties2::default();
        p2.p_next = &mut mesh_props as *mut _ as *mut std::ffi::c_void;
        unsafe {
            self.instance
                .get_physical_device_properties2(self.physical_device, &mut p2);
        }
        self.mesh_max_wg_x = mesh_props.max_mesh_work_group_count[0].max(1);
        log::info!(
            "网格着色器 maxMeshWorkGroupCount[0] = {}（地面场 65536 按此分块）",
            self.mesh_max_wg_x
        );
        let mesh_spirv = load_spirv("assets/mesh.spv")?;
        let fs_spirv = load_spirv("assets/triangle.frag.spv")?;
        let mesh_module = self.create_shader_module(&mesh_spirv)?;
        let fs_module = self.create_shader_module(&fs_spirv)?;

        let mesh_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::MESH_EXT)
            .module(mesh_module)
            .name(c"mesh_main");
        let fs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_module)
            .name(c"fs_main");
        let shader_stages = [mesh_stage, fs_stage];

        let viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.swapchain_extent.width as f32)
            .height(self.swapchain_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(self.swapchain_extent);
        let viewports = [viewport];
        let scissors = [scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);

        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(self.msaa_samples);

        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_depth_bounds(0.0)
            .max_depth_bounds(1.0);

        let color_write_mask = vk::ColorComponentFlags::R
            | vk::ColorComponentFlags::G
            | vk::ColorComponentFlags::B
            | vk::ColorComponentFlags::A;
        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(color_write_mask)
            .blend_enable(false);
        let color_blend_attachments = [color_blend_attachment];
        let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&color_blend_attachments);

        // 同一 descriptor set layout + MESH_EXT push constant（base_slot，16 字节）
        let set_layouts = [self.descriptor_set_layout];
        let push_constant = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::MESH_EXT)
            .offset(0)
            .size(16);
        let mesh_layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(std::slice::from_ref(&push_constant));
        self.mesh_pipeline_layout = unsafe {
            self.device
                .create_pipeline_layout(&mesh_layout_info, None)
                .map_err(|e| format!("创建网格管线布局失败: {}", e))?
        };

        // mesh 管线：pVertexInputState / pInputAssemblyState 必须为 NULL（ash 默认即 null）
        let mesh_dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let mesh_dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&mesh_dynamic_states);
        let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend_state)
            .dynamic_state(&mesh_dynamic_state)
            .layout(self.mesh_pipeline_layout)
            .render_pass(self.render_pass)
            .subpass(0);

        self.mesh_pipeline = unsafe {
            self.device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    &[pipeline_create_info],
                    None,
                )
                .map_err(|(_, e)| format!("创建网格着色器管线失败: {}", e))?
                .remove(0)
        };

        unsafe {
            self.device.destroy_shader_module(mesh_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }
        log::info!("网格着色器管线创建完成（VK_EXT_mesh_shader）");
        Ok(())
    }

    /// 初始化 HUD 覆盖层：自包含 pipeline（无描述符、depth off、alpha 混合）+ 独立 HOST_VISIBLE 顶点缓冲
    fn init_hud(&mut self) -> Result<(), String> {
        let vs_spirv = load_spirv("assets/hud.vert.spv")?;
        let fs_spirv = load_spirv("assets/hud.frag.spv")?;
        let vs_module = self.create_shader_module(&vs_spirv)?;
        let fs_module = self.create_shader_module(&fs_spirv)?;

        let vs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs_module)
            .name(c"vs_main");
        let fs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_module)
            .name(c"fs_main");
        let shader_stages = [vs_stage, fs_stage];

        let hud_binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<HudVertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);
        let hud_attributes = [
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(std::mem::size_of::<[f32; 2]>() as u32),
        ];
        let hud_bindings = [hud_binding];
        let hud_vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&hud_bindings)
            .vertex_attribute_descriptions(&hud_attributes);

        let hud_input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        let hud_viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.swapchain_extent.width as f32)
            .height(self.swapchain_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let hud_scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(self.swapchain_extent);
        let hud_viewports = [hud_viewport];
        let hud_scissors = [hud_scissor];
        let hud_viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&hud_viewports)
            .scissors(&hud_scissors);

        let hud_rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let hud_multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(self.msaa_samples);

        let hud_depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::ALWAYS);

        let hud_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            )
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD);
        let hud_blend_attachments = [hud_blend_attachment];
        let hud_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&hud_blend_attachments);

        // 独立 pipeline layout：无描述符
        let hud_layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&[])
            .push_constant_ranges(&[]);
        self.hud_pipeline_layout = unsafe {
            self.device
                .create_pipeline_layout(&hud_layout_info, None)
                .map_err(|e| format!("创建 HUD 管线布局失败: {}", e))?
        };

        let hud_dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let hud_dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&hud_dynamic_states);
        let hud_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&hud_vertex_input)
            .input_assembly_state(&hud_input_assembly)
            .viewport_state(&hud_viewport_state)
            .rasterization_state(&hud_rasterizer)
            .multisample_state(&hud_multisampling)
            .depth_stencil_state(&hud_depth_stencil)
            .color_blend_state(&hud_blend_state)
            .dynamic_state(&hud_dynamic_state)
            .layout(self.hud_pipeline_layout)
            .render_pass(self.render_pass)
            .subpass(0);
        self.hud_pipeline = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[hud_create_info], None)
                .map_err(|(_, e)| format!("创建 HUD 图形管线失败: {}", e))?
                .remove(0)
        };

        unsafe {
            self.device.destroy_shader_module(vs_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }

        // 独立 HOST_VISIBLE 顶点缓冲（容量 4096 quad × 6 顶点 × 24B）
        let hud_size =
            (self.hud_capacity_quads as usize * 6 * std::mem::size_of::<HudVertex>()) as u64;
        let (buffer, memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, hud_size)?;
        self.hud_vertex_buffer = buffer;
        self.hud_vertex_buffer_memory = memory;
        self.hud_mapped = unsafe {
            self.device
                .map_memory(memory, 0, hud_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射 HUD 顶点缓冲失败: {}", e))?
        };
        log::info!(
            "HUD 覆盖层初始化完成（独立 pipeline，容量 {} quads）",
            self.hud_capacity_quads
        );
        Ok(())
    }

    /// 上传 HUD quad 列表（屏幕像素坐标 → NDC 顶点）；render 前调用，随主 command buffer 绘制
    pub fn set_hud_quads(&mut self, quads: &[crate::ui::Quad]) {
        let (w, h) = (
            self.swapchain_extent.width.max(1) as f32,
            self.swapchain_extent.height.max(1) as f32,
        );
        let count = quads.len().min(self.hud_capacity_quads as usize);
        // 2026-09-22 复查补：HUD 是全仓**最后一处静默截断** —— NPC / 尸体 / 道具那几处
        // 早就有一次性告警闩（`warn_npc_cap_once` 形态），只有这里超容不提示。
        // 仍按容量截断（行为不变，防越界写映射内存），但把"少画了东西"变成可诊断的一行日志。
        if quads.len() > self.hud_capacity_quads as usize {
            use std::sync::atomic::{AtomicBool, Ordering};
            static WARNED: AtomicBool = AtomicBool::new(false);
            if !WARNED.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "HUD quad 超容：需要 {} 个，容量 {}，超出部分本帧不绘制（一次性告警）",
                    quads.len(),
                    self.hud_capacity_quads
                );
            }
        }
        self.hud_vertex_count = (count * 6) as u32;
        if count == 0 || self.hud_mapped.is_null() {
            return;
        }
        let mut verts: Vec<HudVertex> = Vec::with_capacity(count * 6);
        for q in quads.iter().take(count) {
            let x0 = q.rect.x / w * 2.0 - 1.0;
            let y0 = 1.0 - q.rect.y / h * 2.0;
            let x1 = (q.rect.x + q.rect.w) / w * 2.0 - 1.0;
            let y1 = 1.0 - (q.rect.y + q.rect.h) / h * 2.0;
            let color = [q.color.r, q.color.g, q.color.b, q.color.a];
            for (px, py) in [
                (x0, y0),
                (x1, y0),
                (x0, y1),
                (x1, y0),
                (x1, y1),
                (x0, y1),
            ] {
                verts.push(HudVertex { pos: [px, py], color });
            }
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                verts.as_ptr() as *const u8,
                self.hud_mapped as *mut u8,
                verts.len() * std::mem::size_of::<HudVertex>(),
            );
        }
    }

    /// 当前交换链尺寸（main.rs 每帧与窗口尺寸比对，不一致即重建——防 DPI/全屏错位）
    pub fn swapchain_size(&self) -> (u32, u32) {
        (self.swapchain_extent.width, self.swapchain_extent.height)
    }

    /// 上一帧统计：near/far 可见实例数与地形 LOD 名（供 HUD / 日志）
    pub fn last_stats(&self) -> (u32, u32, &'static str) {
        (
            self.last_near_count,
            self.last_far_count,
            self.last_terrain_lod_name,
        )
    }

    pub fn perf_snapshot(&self) -> PerfSnapshot {
        PerfSnapshot {
            frame_us: self.last_frame_us,
            cull_us: self.last_cull_us,
            terrain_us: self.stage_terrain_us,
            wait_fence_us: self.stage_wait_fence_us,
            acquire_us: self.stage_acquire_us,
            record_us: self.stage_record_us,
            submit_us: self.stage_submit_us,
            present_us: self.stage_present_us,
        }
    }

    /// GPU 设备名（性能日志头部用）
    pub fn gpu_name(&self) -> String {
        self.device_name.clone()
    }

    /// 更新光照 uniform（每帧渲染前调用；默认全零 = 光照关闭）
    pub fn set_lights(&mut self, lights: &LightUniform) {
        self.light_data = *lights;
        // RV3D_DEBUG_SHADOW=1：片元直出 shadow_factor 灰度（阴影诊断）
        if std::env::var("RV3D_DEBUG_SHADOW").as_deref() == Ok("1") {
            self.light_data.shadow.config.y = 1.0;
            // D3诊断：仅打印一次实际传入GPU的light_view_proj矩阵（列主序16元素）
            static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let m = self.light_data.shadow.light_view_proj.to_cols_array();
                log::info!(
                    "D3 light_view_proj (col-major): [{:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}]",
                    m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7],
                    m[8], m[9], m[10], m[11], m[12], m[13], m[14], m[15]
                );
            }
        }
    }

    fn create_shader_module(&self, spirv: &[u32]) -> Result<vk::ShaderModule, String> {
        let create_info = vk::ShaderModuleCreateInfo::default().code(spirv);
        unsafe {
            self.device
                .create_shader_module(&create_info, None)
                .map_err(|e| format!("创建着色器模块失败: {}", e))
        }
    }

    /// 选择内存类型：prefer_device_local=true 优先 DEVICE_LOCAL（否则回退任意可用）；
    /// 否则要求 HOST_VISIBLE | HOST_COHERENT
    fn pick_memory_type(
        &self,
        requirements: vk::MemoryRequirements,
        prefer_device_local: bool,
    ) -> Result<u32, String> {
        let mem_properties = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let find = |flags: vk::MemoryPropertyFlags| {
            mem_properties
                .memory_types
                .iter()
                .enumerate()
                .find(|(i, mem_type)| {
                    let type_mask = 1 << i;
                    (requirements.memory_type_bits & type_mask) != 0
                        && mem_type.property_flags.contains(flags)
                })
                .map(|(i, _)| i as u32)
        };
        if prefer_device_local {
            find(vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .or_else(|| find(vk::MemoryPropertyFlags::empty()))
                .ok_or_else(|| "没有找到合适的内存类型（Device Local）".to_string())
        } else {
            find(
                vk::MemoryPropertyFlags::HOST_VISIBLE
                    | vk::MemoryPropertyFlags::HOST_COHERENT,
            )
            .ok_or_else(|| "没有找到合适的内存类型（Host Buffer）".to_string())
        }
    }

    /// 创建 buffer 并分配 HOST_VISIBLE | HOST_COHERENT 内存
    fn create_host_buffer(
        &self,
        usage: vk::BufferUsageFlags,
        size: u64,
    ) -> Result<(vk::Buffer, vk::DeviceMemory), String> {
        let buffer_create_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe {
            self.device
                .create_buffer(&buffer_create_info, None)
                .map_err(|e| format!("创建缓冲失败: {}", e))?
        };
        let mem_requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let memory_type = self.pick_memory_type(mem_requirements, false)?;
        let mut alloc_flags = vk::MemoryAllocateFlagsInfo::default();
        alloc_flags.flags = if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
            vk::MemoryAllocateFlags::DEVICE_ADDRESS
        } else {
            vk::MemoryAllocateFlags::empty()
        };
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_requirements.size)
            .memory_type_index(memory_type)
            .push_next(&mut alloc_flags);
        let memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配缓冲内存失败: {}", e))?
        };
        unsafe {
            self.device
                .bind_buffer_memory(buffer, memory, 0)
                .map_err(|e| format!("绑定缓冲内存失败: {}", e))?;
        }
        Ok((buffer, memory))
    }

    /// 创建 DEVICE_LOCAL 静态缓冲（一次性：staging 上传后即释放）。
    /// 用于地形等一次性数据，避免 GPU 每帧从 host 内存读顶点/索引。
    fn create_device_local_buffer(
        &self,
        usage: vk::BufferUsageFlags,
        data: &[u8],
        label: &str,
    ) -> Result<(vk::Buffer, vk::DeviceMemory), String> {
        let size = data.len() as u64;

        // 1. staging buffer（HOST_VISIBLE | HOST_COHERENT，TRANSFER_SRC）
        let staging_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let staging_buffer = unsafe {
            self.device
                .create_buffer(&staging_info, None)
                .map_err(|e| format!("创建 {} staging buffer 失败: {}", label, e))?
        };
        let staging_reqs = unsafe { self.device.get_buffer_memory_requirements(staging_buffer) };
        let staging_type = self.pick_memory_type(staging_reqs, false)?;
        let mut st_flags = vk::MemoryAllocateFlagsInfo::default();
        st_flags.flags = if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
            vk::MemoryAllocateFlags::DEVICE_ADDRESS
        } else {
            vk::MemoryAllocateFlags::empty()
        };
        let staging_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(staging_reqs.size)
            .memory_type_index(staging_type)
            .push_next(&mut st_flags);
        let staging_memory = unsafe {
            self.device
                .allocate_memory(&staging_alloc, None)
                .map_err(|e| format!("分配 {} staging 内存失败: {}", label, e))?
        };
        unsafe {
            self.device
                .bind_buffer_memory(staging_buffer, staging_memory, 0)
                .map_err(|e| format!("绑定 {} staging buffer 失败: {}", label, e))?;
            let ptr = self
                .device
                .map_memory(staging_memory, 0, size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射 {} staging 内存失败: {}", label, e))?;
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
            self.device.unmap_memory(staging_memory);
        }

        // 2. 目标 buffer（DEVICE_LOCAL 优先，usage | TRANSFER_DST）
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage | vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe {
            self.device
                .create_buffer(&buffer_info, None)
                .map_err(|e| format!("创建 {} buffer 失败: {}", label, e))?
        };
        let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let memory_type = self.pick_memory_type(mem_reqs, true)?;
        let mut fin_flags = vk::MemoryAllocateFlagsInfo::default();
        fin_flags.flags = if (usage | vk::BufferUsageFlags::TRANSFER_DST).contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
            vk::MemoryAllocateFlags::DEVICE_ADDRESS
        } else {
            vk::MemoryAllocateFlags::empty()
        };
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(memory_type)
            .push_next(&mut fin_flags);
        let memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配 {} 内存失败: {}", label, e))?
        };
        unsafe {
            self.device
                .bind_buffer_memory(buffer, memory, 0)
                .map_err(|e| format!("绑定 {} buffer 失败: {}", label, e))?;
        }

        // 3. staging → 目标 一次性拷贝
        self.run_single_time_commands(|cmd| {
            let region = vk::BufferCopy::default().size(size);
            unsafe {
                self.device.cmd_copy_buffer(cmd, staging_buffer, buffer, &[region]);
            }
        })?;

        // 4. 释放 staging
        unsafe {
            self.device.free_memory(staging_memory, None);
            self.device.destroy_buffer(staging_buffer, None);
        }
        Ok((buffer, memory))
    }

    /// 创建立方体顶点缓冲（24 顶点）
    fn create_vertex_buffer(&mut self) -> Result<(), String> {
        let buffer_size = std::mem::size_of_val(&VERTICES) as u64;
        let (buffer, memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, buffer_size)?;
        self.vertex_buffer = buffer;
        self.vertex_buffer_memory = memory;

        let data_ptr = unsafe {
            self.device
                .map_memory(
                    self.vertex_buffer_memory,
                    0,
                    buffer_size,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|e| format!("映射顶点缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                VERTICES.as_ptr() as *const u8,
                data_ptr as *mut u8,
                buffer_size as usize,
            );
            self.device.unmap_memory(self.vertex_buffer_memory);
        }
        Ok(())
    }

    /// 创建立方体索引缓冲（36 索引，UINT32）
    fn create_index_buffer(&mut self) -> Result<(), String> {
        let buffer_size = std::mem::size_of_val(&INDICES) as u64;
        let (buffer, memory) =
            self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, buffer_size)?;
        self.index_buffer = buffer;
        self.index_buffer_memory = memory;

        let data_ptr = unsafe {
            self.device
                .map_memory(
                    self.index_buffer_memory,
                    0,
                    buffer_size,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|e| format!("映射索引缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                INDICES.as_ptr() as *const u8,
                data_ptr as *mut u8,
                buffer_size as usize,
            );
            self.device.unmap_memory(self.index_buffer_memory);
        }
        Ok(())
    }

    /// 创建远档 LOD 十字双 quad 的顶点/索引缓冲（8 顶点 / 12 索引）
    fn create_far_geometry(&mut self) -> Result<(), String> {
        let vert_size = std::mem::size_of_val(&FAR_VERTS) as u64;
        let (v_buffer, v_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, vert_size)?;
        self.far_vertex_buffer = v_buffer;
        self.far_vertex_buffer_memory = v_memory;

        let v_ptr = unsafe {
            self.device
                .map_memory(v_memory, 0, vert_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射远档顶点缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                FAR_VERTS.as_ptr() as *const u8,
                v_ptr as *mut u8,
                vert_size as usize,
            );
            self.device.unmap_memory(v_memory);
        }

        let idx_size = std::mem::size_of_val(&FAR_INDICES) as u64;
        let (i_buffer, i_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, idx_size)?;
        self.far_index_buffer = i_buffer;
        self.far_index_buffer_memory = i_memory;

        let i_ptr = unsafe {
            self.device
                .map_memory(i_memory, 0, idx_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射远档索引缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                FAR_INDICES.as_ptr() as *const u8,
                i_ptr as *mut u8,
                idx_size as usize,
            );
            self.device.unmap_memory(i_memory);
        }
        log::info!("远档 LOD 几何创建完成: {} 顶点 / {} 索引（十字双 quad）", FAR_VERTS.len(), FAR_INDICES.len());
        Ok(())
    }

    /// 创建地面平铺 quad 的顶点/索引缓冲（4 顶点 / 6 索引），近档+远档地面 draw 共用。
    fn create_ground_geometry(&mut self) -> Result<(), String> {
        let vert_size = std::mem::size_of_val(&GROUND_VERTS) as u64;
        let (v_buffer, v_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, vert_size)?;
        self.ground_vertex_buffer = v_buffer;
        self.ground_vertex_buffer_memory = v_memory;

        let v_ptr = unsafe {
            self.device
                .map_memory(v_memory, 0, vert_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射地面顶点缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                GROUND_VERTS.as_ptr() as *const u8,
                v_ptr as *mut u8,
                vert_size as usize,
            );
            self.device.unmap_memory(v_memory);
        }

        let idx_size = std::mem::size_of_val(&GROUND_INDICES) as u64;
        let (i_buffer, i_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, idx_size)?;
        self.ground_index_buffer = i_buffer;
        self.ground_index_buffer_memory = i_memory;

        let i_ptr = unsafe {
            self.device
                .map_memory(i_memory, 0, idx_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射地面索引缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                GROUND_INDICES.as_ptr() as *const u8,
                i_ptr as *mut u8,
                idx_size as usize,
            );
            self.device.unmap_memory(i_memory);
        }
        log::info!(
            "地面平铺 quad 几何创建完成: {} 顶点 / {} 索引（无侧壁）",
            GROUND_VERTS.len(),
            GROUND_INDICES.len()
        );
        Ok(())
    }


    /// 创建 UV 球体几何（爆炸球形扩散用）：24 经 × 12 纬段，CPU 生成顶点/索引。
    /// 球面坐标 (u,v) → 单位球 (sinφ·cosθ, cosφ, sinφ·sinθ)，白化颜色走 tint。
    fn create_sphere_geometry(&mut self) -> Result<(), String> {
        const SEGS: u32 = 24; // 经线
        const RINGS: u32 = 12; // 纬线
        let mut verts: Vec<Vertex> = Vec::with_capacity(((SEGS + 1) * (RINGS + 1)) as usize);
        for j in 0..=RINGS {
            let phi = std::f32::consts::PI * j as f32 / RINGS as f32; // 0..π
            let (sp, cp) = phi.sin_cos();
            for i in 0..=SEGS {
                let theta = std::f32::consts::TAU * i as f32 / SEGS as f32;
                let (st, ct) = theta.sin_cos();
                verts.push(Vertex {
                    pos: [sp * ct, cp, sp * st],
                    color: [1.0, 1.0, 1.0],
                    uv: [i as f32 / SEGS as f32, 1.0 - j as f32 / RINGS as f32],
                });
            }
        }
        let mut indices: Vec<u32> = Vec::with_capacity((SEGS * RINGS * 6) as usize);
        for j in 0..RINGS {
            for i in 0..SEGS {
                let a = j * (SEGS + 1) + i;
                let b = a + 1;
                let c = a + SEGS + 1;
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        let vert_size = (verts.len() * std::mem::size_of::<Vertex>()) as u64;
        let (v_buffer, v_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, vert_size)?;
        self.sphere_vertex_buffer = v_buffer;
        self.sphere_vertex_buffer_memory = v_memory;
        let v_ptr = unsafe {
            self.device
                .map_memory(v_memory, 0, vert_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射球体顶点缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(verts.as_ptr() as *const u8, v_ptr as *mut u8, vert_size as usize);
            self.device.unmap_memory(v_memory);
        }
        let idx_size = (indices.len() * std::mem::size_of::<u32>()) as u64;
        let (i_buffer, i_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, idx_size)?;
        self.sphere_index_buffer = i_buffer;
        self.sphere_index_buffer_memory = i_memory;
        let i_ptr = unsafe {
            self.device
                .map_memory(i_memory, 0, idx_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射球体索引缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(indices.as_ptr() as *const u8, i_ptr as *mut u8, idx_size as usize);
            self.device.unmap_memory(i_memory);
        }
        self.sphere_index_count = indices.len() as u32;
        log::info!("球体几何创建完成: {} 顶点 / {} 索引（爆炸球形扩散）", verts.len(), indices.len());
        Ok(())
    }

    /// 圆柱几何数据（上传与绕序回归测试共用）：单位圆柱 r=1 h=1 沿 Y，24 段，含上下盖。
    /// 水平盖的绕序必须与地面 quad 同约定（从上方可见 ⇔ (x,z) 有向面积 > 0），
    /// 立方体顶/底面曾因反绕被上方剔除，判据见 `horizontal_winding_tests`。
    fn cylinder_mesh_data() -> (Vec<Vertex>, Vec<u32>) {
        const SEGS: u32 = 24;
        let mut verts: Vec<Vertex> = Vec::with_capacity((SEGS * 2 + 2) as usize);
        // 侧壁：上下两圈（y = ±0.5）
        for j in 0..2 {
            let y = if j == 0 { -0.5 } else { 0.5 };
            for i in 0..SEGS {
                let theta = std::f32::consts::TAU * i as f32 / SEGS as f32;
                let (st, ct) = theta.sin_cos();
                verts.push(Vertex {
                    pos: [ct, y, st],
                    color: [1.0, 1.0, 1.0],
                    uv: [i as f32 / SEGS as f32, j as f32],
                });
            }
        }
        // 上下盖中心顶点
        let top_center = verts.len() as u32;
        verts.push(Vertex { pos: [0.0, 0.5, 0.0], color: [1.0, 1.0, 1.0], uv: [0.5, 1.0] });
        let bottom_center = verts.len() as u32;
        verts.push(Vertex { pos: [0.0, -0.5, 0.0], color: [1.0, 1.0, 1.0], uv: [0.5, 0.0] });
        let mut indices: Vec<u32> = Vec::with_capacity((SEGS * 6 + SEGS * 6) as usize);
        for i in 0..SEGS {
            let a = i;
            let b = (i + 1) % SEGS;
            // 侧壁三角形（a=下圈, b=下圈+1, c=上圈... 下圈顶点 0..SEGS，上圈 SEGS..2*SEGS）
            let t0 = a;
            let t1 = b;
            let t2 = SEGS + a;
            let t3 = SEGS + b;
            indices.extend_from_slice(&[t0, t2, t1, t1, t2, t3]);
            // 上盖 fan（绕序同地面 quad：从上方看是正面）
            indices.extend_from_slice(&[top_center, t2, t3]);
            // 下盖 fan（反向：从下方看才是正面）
            indices.extend_from_slice(&[bottom_center, t1, t0]);
        }
        (verts, indices)
    }

    /// 创建 NPC 人体圆柱几何（四肢用）：单位圆柱 r=1 h=1 沿 Y，24 段，含上下盖。
    fn create_cylinder_geometry(&mut self) -> Result<(), String> {
        let (verts, indices) = Self::cylinder_mesh_data();
        let vert_size = (verts.len() * std::mem::size_of::<Vertex>()) as u64;
        let (v_buffer, v_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, vert_size)?;
        self.cylinder_vertex_buffer = v_buffer;
        self.cylinder_vertex_buffer_memory = v_memory;
        let v_ptr = unsafe {
            self.device
                .map_memory(v_memory, 0, vert_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射圆柱顶点缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(verts.as_ptr() as *const u8, v_ptr as *mut u8, vert_size as usize);
            self.device.unmap_memory(v_memory);
        }
        let idx_size = (indices.len() * std::mem::size_of::<u32>()) as u64;
        let (i_buffer, i_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, idx_size)?;
        self.cylinder_index_buffer = i_buffer;
        self.cylinder_index_buffer_memory = i_memory;
        let i_ptr = unsafe {
            self.device
                .map_memory(i_memory, 0, idx_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射圆柱索引缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(indices.as_ptr() as *const u8, i_ptr as *mut u8, idx_size as usize);
            self.device.unmap_memory(i_memory);
        }
        self.cylinder_index_count = indices.len() as u32;
        log::info!("圆柱几何创建完成: {} 顶点 / {} 索引（NPC 四肢）", verts.len(), indices.len());
        Ok(())
    }
    /// 创建 3 级地形 LOD 网格（高 257² / 中 129² / 低 65² 顶点）。
    /// 高度用与实例 Y 完全相同的 terrain_height() 生成；顶点缓冲 HOST_VISIBLE，
    /// 过渡带内每帧 morph 高度后整块重传；索引缓冲一次性上传。
    fn create_terrain_lods(&mut self) -> Result<(), String> {
        // 1. 各级网格原始高度（粗网格顶点恰为细网格顶点子集）
        let grid_heights: Vec<Vec<f32>> = TERRAIN_LOD_CELLS
            .iter()
            .map(|&cells| {
                let w = cells + 1;
                let cell = 512.0 / cells as f32;
                let mut hs = Vec::with_capacity(w * w);
                for iz in 0..w {
                    let z = -TERRAIN_HALF + iz as f32 * cell;
                    for ix in 0..w {
                        let x = -TERRAIN_HALF + ix as f32 * cell;
                        hs.push(terrain_height(x, z));
                    }
                }
                hs
            })
            .collect();

        for idx in 0..TERRAIN_LOD_CELLS.len() {
            let level = TerrainLod::from_idx(idx);
            let cells = level.cells();
            let w = level.verts();
            let cell = level.cell_size();
            let heights = &grid_heights[idx];

            // 顶点（UV 用世界坐标，保证各级贴图对齐；颜色白）
            let mut verts: Vec<Vertex> = Vec::with_capacity(w * w);
            let mut base_heights: Vec<f32> = Vec::with_capacity(w * w);
            for iz in 0..w {
                for ix in 0..w {
                    let x = -TERRAIN_HALF + ix as f32 * cell;
                    let z = -TERRAIN_HALF + iz as f32 * cell;
                    let y = heights[iz * w + ix] - TERRAIN_RENDER_SINK;
                    base_heights.push(y);
                    verts.push(Vertex {
                        pos: [x, y, z],
                        color: [1.0, 1.0, 1.0],
                        uv: [
                            (x + TERRAIN_HALF) / TERRAIN_UV_SCALE,
                            (z + TERRAIN_HALF) / TERRAIN_UV_SCALE,
                        ],
                    });
                }
            }

            // morph 目标高度：下一级（更粗）曲面三角形插值；Low 级无下一级
            let coarse_heights: Vec<f32> = if idx + 1 < TERRAIN_LOD_CELLS.len() {
                let coarse = &grid_heights[idx + 1];
                let coarse_cells = TERRAIN_LOD_CELLS[idx + 1];
                (0..w)
                    .flat_map(|iz| {
                        (0..w).map(move |ix| {
                            let x = -TERRAIN_HALF + ix as f32 * cell;
                            let z = -TERRAIN_HALF + iz as f32 * cell;
                            terrain_coarse_height(x, z, coarse, coarse_cells) - TERRAIN_RENDER_SINK
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };

            // 索引（与原有地形相同的三角形剖分：cell 对角 v0→v2）
            let mut idx_buf: Vec<u32> = Vec::with_capacity(cells * cells * 6);
            for iz in 0..cells {
                for ix in 0..cells {
                    let v0 = (iz * w + ix) as u32;
                    let v1 = v0 + 1;
                    let v2 = v0 + w as u32 + 1;
                    let v3 = v0 + w as u32;
                    idx_buf.push(v0);
                    idx_buf.push(v2);
                    idx_buf.push(v1);
                    idx_buf.push(v0);
                    idx_buf.push(v3);
                    idx_buf.push(v2);
                }
            }

            // 顶点缓冲：HOST_VISIBLE 并持久映射（每帧 morph 后整块重传）
            let vert_bytes = unsafe {
                std::slice::from_raw_parts(
                    verts.as_ptr() as *const u8,
                    verts.len() * std::mem::size_of::<Vertex>(),
                )
            };
            let (v_buffer, v_memory) = self.create_host_buffer(
                vk::BufferUsageFlags::VERTEX_BUFFER,
                vert_bytes.len() as u64,
            )?;
            let v_ptr = unsafe {
                self.device
                    .map_memory(
                        v_memory,
                        0,
                        vert_bytes.len() as u64,
                        vk::MemoryMapFlags::empty(),
                    )
                    .map_err(|e| format!("映射地形 LOD[{}] 顶点内存失败: {}", idx, e))?
            };
            unsafe {
                std::ptr::copy_nonoverlapping(vert_bytes.as_ptr(), v_ptr as *mut u8, vert_bytes.len());
            }

            // 索引缓冲：静态数据，DEVICE_LOCAL 一次性上传（staging 拷贝）
            let idx_bytes = unsafe {
                std::slice::from_raw_parts(
                    idx_buf.as_ptr() as *const u8,
                    idx_buf.len() * std::mem::size_of::<u32>(),
                )
            };
            let (i_buffer, i_memory) = self.create_device_local_buffer(
                vk::BufferUsageFlags::INDEX_BUFFER,
                idx_bytes,
                "地形索引",
            )?;

            self.terrain_lods.push(TerrainLodMesh {
                vertex_buffer: v_buffer,
                vertex_memory: v_memory,
                vertex_mapped: v_ptr,
                index_buffer: i_buffer,
                index_memory: i_memory,
                index_count: level.index_count(),
                verts,
                base_heights,
                coarse_heights,
            });

            log::info!(
                "地形 LOD[{}] 创建完成: {} 顶点 / {} 索引（{}×{} 网格，间距 {}）",
                idx,
                w * w,
                cells * cells * 6,
                w,
                w,
                cell
            );
        }
        log::info!("地形 3 级 LOD 全部创建完成（高/中/低）");
        Ok(())
    }

    /// 标量地形 LOD morph 高度：y = base + (coarse − base) × blend（回退路径/基准语义）
    fn morph_heights_scalar(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        for i in 0..out.len() {
            out[i] = base[i] + (coarse[i] - base[i]) * blend;
        }
    }

    /// AVX-512 地形 morph：16 顶点/批。运算顺序与标量一致（先 sub 再 mul 再 add，无 FMA），
    /// IEEE 逐位一致。★ AVX-512 加速说明：Zen4/Zen5（7000/9000 系）双 256 单元合并执行
    /// 512 位请求；选路走 cpu::avx512_enabled()（Intel 11 代能效差 / 12 代起大小核自动禁用）。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx512f")]
    unsafe fn morph_heights_avx512(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        use std::arch::x86_64::*;
        let b = _mm512_set1_ps(blend);
        let mut i = 0usize;
        while i + 16 <= out.len() {
            let bv = _mm512_loadu_ps(base.as_ptr().add(i));
            let cv = _mm512_loadu_ps(coarse.as_ptr().add(i));
            let diff = _mm512_sub_ps(cv, bv);
            let y = _mm512_add_ps(bv, _mm512_mul_ps(diff, b));
            _mm512_storeu_ps(out.as_mut_ptr().add(i), y);
            i += 16;
        }
        // 尾部不足 16 个走标量（与 cull 尾部队列策略一致）
        for j in i..out.len() {
            out[j] = base[j] + (coarse[j] - base[j]) * blend;
        }
    }

    /// AVX2 地形 morph：8 顶点/批（与标量逐位一致，非 FMA）
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn morph_heights_avx2(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        use std::arch::x86_64::*;
        let b = _mm256_set1_ps(blend);
        let mut i = 0usize;
        while i + 8 <= out.len() {
            let bv = _mm256_loadu_ps(base.as_ptr().add(i));
            let cv = _mm256_loadu_ps(coarse.as_ptr().add(i));
            let diff = _mm256_sub_ps(cv, bv);
            let y = _mm256_add_ps(bv, _mm256_mul_ps(diff, b));
            _mm256_storeu_ps(out.as_mut_ptr().add(i), y);
            i += 8;
        }
        for j in i..out.len() {
            out[j] = base[j] + (coarse[j] - base[j]) * blend;
        }
    }

    /// AVX（非 AVX2，3/4 代酷睿与初代锐龙）地形 morph：8 顶点/批
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx")]
    unsafe fn morph_heights_avx(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        use std::arch::x86_64::*;
        let b = _mm256_set1_ps(blend);
        let mut i = 0usize;
        while i + 8 <= out.len() {
            let bv = _mm256_loadu_ps(base.as_ptr().add(i));
            let cv = _mm256_loadu_ps(coarse.as_ptr().add(i));
            let diff = _mm256_sub_ps(cv, bv);
            let y = _mm256_add_ps(bv, _mm256_mul_ps(diff, b));
            _mm256_storeu_ps(out.as_mut_ptr().add(i), y);
            i += 8;
        }
        for j in i..out.len() {
            out[j] = base[j] + (coarse[j] - base[j]) * blend;
        }
    }

    /// SSE4.2 地形 morph：4 顶点/批（2008 年后所有 Intel/AMD 消费级）
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "sse4.2")]
    unsafe fn morph_heights_sse(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        use std::arch::x86_64::*;
        let b = _mm_set1_ps(blend);
        let mut i = 0usize;
        while i + 4 <= out.len() {
            let bv = _mm_loadu_ps(base.as_ptr().add(i));
            let cv = _mm_loadu_ps(coarse.as_ptr().add(i));
            let diff = _mm_sub_ps(cv, bv);
            let y = _mm_add_ps(bv, _mm_mul_ps(diff, b));
            _mm_storeu_ps(out.as_mut_ptr().add(i), y);
            i += 4;
        }
        for j in i..out.len() {
            out[j] = base[j] + (coarse[j] - base[j]) * blend;
        }
    }

    /// NEON（AArch64，Apple Silicon/Android/高通 X Elite）地形 morph：4 顶点/批
    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon")]
    unsafe fn morph_heights_neon(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        use std::arch::aarch64::*;
        let b = vdupq_n_f32(blend);
        let mut i = 0usize;
        while i + 4 <= out.len() {
            let bv = vld1q_f32(base.as_ptr().add(i));
            let cv = vld1q_f32(coarse.as_ptr().add(i));
            let diff = vsubq_f32(cv, bv);
            let y = vaddq_f32(bv, vmulq_f32(diff, b));
            vst1q_f32(out.as_mut_ptr().add(i), y);
            i += 4;
        }
        for j in i..out.len() {
            out[j] = base[j] + (coarse[j] - base[j]) * blend;
        }
    }

    /// 地形 morph 高度选路（与剔除同策略，见 cull_spheres_dispatch）：
    /// x86_64：AVX-512(16) > AVX2(8) > AVX(8) > SSE4.2(4) > 标量；aarch64：NEON(4) > 标量。
    fn morph_heights_dispatch(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        #[cfg(target_arch = "x86_64")]
        {
            // 基准用强制选路（RV3D_FORCE_SIMD，见 cpu::forced_simd_path）；仍要求硬件支持
            if let Some(forced) = crate::engine::cpu::forced_simd_path() {
                let supported = match forced {
                    "avx512" => std::is_x86_feature_detected!("avx512f"),
                    "avx2" => std::is_x86_feature_detected!("avx2"),
                    "avx" => std::is_x86_feature_detected!("avx"),
                    "sse4.2" => std::is_x86_feature_detected!("sse4.2"),
                    "scalar" => true,
                    _ => false,
                };
                if supported {
                    match forced {
                        "avx512" => {
                            // safety: 上面已确认 avx512f 硬件支持
                            unsafe {
                                Self::morph_heights_avx512(base, coarse, blend, out);
                            }
                        }
                        "avx2" => {
                            // safety: 上面已确认 avx2 硬件支持
                            unsafe {
                                Self::morph_heights_avx2(base, coarse, blend, out);
                            }
                        }
                        "avx" => {
                            // safety: 上面已确认 avx 硬件支持
                            unsafe {
                                Self::morph_heights_avx(base, coarse, blend, out);
                            }
                        }
                        "sse4.2" => {
                            // safety: 上面已确认 sse4.2 硬件支持
                            unsafe {
                                Self::morph_heights_sse(base, coarse, blend, out);
                            }
                        }
                        _ => Self::morph_heights_scalar(base, coarse, blend, out),
                    }
                    return;
                }
                // 每帧调用（morph 每级 / 剔除每段）⇒ 走一次性告警，见 `simd::warn_forced_simd_unsupported`
                crate::engine::simd::warn_forced_simd_unsupported(forced);
            }
            if crate::engine::cpu::avx512_enabled() {
                // safety: 上面已运行时检测 AVX-512，CPU 支持才进入该分支
                unsafe {
                    Self::morph_heights_avx512(base, coarse, blend, out);
                }
            } else if std::is_x86_feature_detected!("avx2") {
                // safety: 上面已运行时检测 AVX2
                unsafe {
                    Self::morph_heights_avx2(base, coarse, blend, out);
                }
            } else if std::is_x86_feature_detected!("avx") {
                // safety: 上面已运行时检测 AVX
                unsafe {
                    Self::morph_heights_avx(base, coarse, blend, out);
                }
            } else if std::is_x86_feature_detected!("sse4.2") {
                // safety: 上面已运行时检测 SSE4.2
                unsafe {
                    Self::morph_heights_sse(base, coarse, blend, out);
                }
            } else {
                Self::morph_heights_scalar(base, coarse, blend, out);
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            if std::arch::is_aarch64_feature_detected!("neon") {
                // safety: NEON 在 AArch64 是基线特性（此处仍运行时确认）
                unsafe {
                    Self::morph_heights_neon(base, coarse, blend, out);
                }
            } else {
                Self::morph_heights_scalar(base, coarse, blend, out);
            }
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self::morph_heights_scalar(base, coarse, blend, out);
        }
    }

    /// 每帧按 morph 进度 t 更新当前 LOD 网格顶点高度：
    /// h = 细高度 + t × (下一级曲面插值高度 − 细高度)。t=1 时几何与下一级完全重合，
    /// 因此切换级别无 popping。仅在过渡带内（0<t<1）执行。
    /// 计算走 SIMD 选路（AVX-512 > AVX2 > AVX > SSE4.2 > NEON > 标量，逐位一致），
    /// 写回按段并行（scene_pool：AMD CCD0 / Intel P-core，与渲染主线程同簇）。
    /// 仅写 y 分量 4B/顶点：其余顶点分量上一帧已就位，映射内存常驻无需整块重传。
    fn update_terrain_lod_morph(&mut self, level: TerrainLod, blend: f32) {
        if blend <= 0.0 || blend >= 1.0 {
            return;
        }
        let idx = level as usize;
        let mesh = match self.terrain_lods.get_mut(idx) {
            Some(m) => m,
            None => return,
        };
        if mesh.coarse_heights.is_empty() {
            return;
        }
        let base = &mesh.base_heights;
        let coarse = &mesh.coarse_heights;
        let n = mesh.verts.len();
        // 1) SIMD 计算 y 数组（n ≤ 65536 → 最多 256KB，过渡带内才执行）
        let mut ys = vec![0.0f32; n];
        Self::morph_heights_dispatch(base, coarse, blend, &mut ys);
        // 2) 并行写回 verts.pos[1] + 映射内存 y 分量（段间不相交，join 后才返回）
        let stride = std::mem::size_of::<Vertex>();
        let mapped = crate::engine::cpu::SendPtr(mesh.vertex_mapped as *mut u8);
        let pool = crate::engine::cpu::scene_pool();
        pool.par_for_each_mut(&mut mesh.verts, move |_seg, start, slice| {
            for (k, v) in slice.iter_mut().enumerate() {
                let y = ys[start + k];
                v.pos[1] = y;
                // SAFETY: mapped 指向 HOST_VISIBLE 顶点缓冲（常驻映射，本帧未写入该区段）；
                // 各段只写 [ (start+k)*stride+4, +4 ) 的 y 分量，互不相交。
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        &y as *const f32 as *const u8,
                        mapped.get().add((start + k) * stride + 4),
                        4,
                    );
                }
            }
        });
    }

    /// 生成 256×256 网格实例；按 frame-in-flight 数量双缓冲
    /// （每帧一份 HOST_VISIBLE|HOST_COHERENT buffer，剔除后压缩上传到当前帧 slot）
    fn create_instance_buffer(&mut self) -> Result<(), String> {
        debug_assert!(
            std::mem::size_of::<InstanceData>() == 80,
            "InstanceData 必须对齐 std430 步长 80 字节"
        );

        // 256×256 网格：间距 2.0、以原点为中心、y=0 平面（场地 512×512）。
        // 地面实例用专用平铺 quad 几何（GROUND_VERTS，几何无侧壁），矩阵纯平移，
        // 高度 = terrain_height + 0.05（略高于地形网格 y=0，避免深度冲突）。
        // 旧版 2×2×2 立方体（顶面 +0.95）与压扁薄片（侧壁 0.2m）都有可见侧壁，
        // 视觉上像一格一格"掀盖纸箱"铺地；平铺 quad 无任何竖立面，地面真正连续。
        self.instances = Vec::with_capacity(INSTANCE_COUNT as usize);
        for iz in 0..GRID_SIZE {
            for ix in 0..GRID_SIZE {
                let x = (ix as f32 - (GRID_SIZE as f32 - 1.0) * 0.5) * 2.0;
                let z = (iz as f32 - (GRID_SIZE as f32 - 1.0) * 0.5) * 2.0;
                let y = terrain_height(x, z) + 0.05;
                let model = glam::Mat4::from_translation(glam::Vec3::new(x, y, z));
                // 半径 = 2×2m quad 半对角线 √(1²+1²)=√2≈1.414（2026-08-15 修正：
                // 旧 0.5×√2=0.707 低估一半 → 屏幕四角边缘实例被激进剔除穿帮）
                let r = 2.0f32.sqrt();
                self.instance_radii.push(r);
                self.instance_center_x.push(x);
                self.instance_center_y.push(y);
                self.instance_center_z.push(z);
                self.instances.push(InstanceData {
                    model: model.to_cols_array(),
                    tint: [0.7, 0.7, 0.7, 1.0],
                });
            }
        }
        // 并行剔除暂存：一次分配整场容量（每段可见索引上限 = 段实例数）
        self.culled_scratch = vec![0u32; INSTANCE_COUNT as usize];

        // 末尾保留 1 个 slot 存 identity 实例（地形 draw 用，仅创建时写入一次），
        // 元素数由 INSTANCE_BUFFER_ELEMS 单一定义（= 最高槽位 + 1），不再在此抄写副本：
        // 历史上这里是三份互不同步的硬编码，漏改任一份都会让 shader 越界读到全零矩阵、
        // 几何静默消失（无日志、无 VUID）。详见该常量的注释。
        let buffer_elems = INSTANCE_BUFFER_ELEMS;
        let buffer_size = buffer_elems * std::mem::size_of::<InstanceData>() as u64;
        let identity = InstanceData {
            model: glam::Mat4::IDENTITY.to_cols_array(),
            tint: [1.0, 1.0, 1.0, 1.0],
        };
        // 每帧一份 HOST_VISIBLE | HOST_COHERENT buffer，STORAGE_BUFFER（每帧 CPU 直接写）
        let buffer_info = vk::BufferCreateInfo::default()
            .size(buffer_size)
            .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        for _ in 0..self.max_frames_in_flight {
            let buffer = unsafe {
                self.device
                    .create_buffer(&buffer_info, None)
                    .map_err(|e| format!("创建实例 buffer 失败: {}", e))?
            };
            let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(buffer) };
            let memory_type = self.pick_memory_type(mem_reqs, false)?;
            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_reqs.size)
                .memory_type_index(memory_type);
            let memory = unsafe {
                self.device
                    .allocate_memory(&alloc_info, None)
                    .map_err(|e| format!("分配实例 buffer 内存失败: {}", e))?
            };
            unsafe {
                self.device
                    .bind_buffer_memory(buffer, memory, 0)
                    .map_err(|e| format!("绑定实例 buffer 内存失败: {}", e))?;
            }
            let mapped = unsafe {
                self.device
                    .map_memory(memory, 0, buffer_size, vk::MemoryMapFlags::empty())
                    .map_err(|e| format!("映射实例 buffer 失败: {}", e))?
            };
            self.instance_buffers.push(buffer);
            self.instance_buffers_memory.push(memory);
            self.instance_mapped.push(mapped);
            // 写入 identity 实例到槽位 INSTANCE_COUNT（地形 draw 读取，永不覆盖）。
            // 必须写对槽位偏移：旧实现写到了槽位 0，被 cull_and_upload 每帧覆盖，
            // 槽位 65536 恒为未初始化内存 → 地形矩阵塌缩到原点（主 pass 被地面
            // quad 遮住未暴露，阴影 pass 里地形整片消失，阴影图 99.7% 空白）。
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &identity as *const InstanceData as *const u8,
                    (mapped as *mut u8).add(
                        INSTANCE_COUNT as usize * std::mem::size_of::<InstanceData>(),
                    ),
                    std::mem::size_of::<InstanceData>(),
                );
                // 枪模 identity 槽（GUN_INSTANCE_INDEX）：主管线 flat=1 纯色路径用
                std::ptr::copy_nonoverlapping(
                    &identity as *const InstanceData as *const u8,
                    (mapped as *mut u8).add(
                        GUN_INSTANCE_INDEX as usize * std::mem::size_of::<InstanceData>(),
                    ),
                    std::mem::size_of::<InstanceData>(),
                );
                // 道具 identity 槽（PROP_INSTANCE_INDEX）：identity 矩阵 + Authored 标签。
                // tint.rgb 必须全 1，否则片元的 `input.color = vertexColor × tint.rgb`
                // 会把烘焙好的顶点色整体染色。
                let authored = InstanceData {
                    model: glam::Mat4::IDENTITY.to_cols_array(),
                    tint: [1.0, 1.0, 1.0, crate::engine::geom::Shape::Authored.tag()],
                };
                std::ptr::copy_nonoverlapping(
                    &authored as *const InstanceData as *const u8,
                    (mapped as *mut u8).add(
                        PROP_INSTANCE_INDEX as usize * std::mem::size_of::<InstanceData>(),
                    ),
                    std::mem::size_of::<InstanceData>(),
                );
            }
        }

        if self.mesh_enabled {
            // mesh 路径：地面实例场完全静态（创建后永不修改），初始化时一次性写入全部
            // 槽位（0..INSTANCE_COUNT）到每帧 buffer；此后每帧只上传 marker/NPC/自发光
            // 增量，完全跳过 CPU SIMD 剔除与压缩上传（5.24MB 一次性带宽换每帧 CPU 减负）。
            let bytes = self.instances.len() * std::mem::size_of::<InstanceData>();
            for &mapped in &self.instance_mapped {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        self.instances.as_ptr() as *const u8,
                        mapped as *mut u8,
                        bytes,
                    );
                }
            }
        }

        log::info!(
            "实例缓冲创建完成: {} 个实例，stride {} 字节，{} 帧双缓冲 HOST_VISIBLE|HOST_COHERENT（每帧压缩上传）",
            INSTANCE_COUNT,
            std::mem::size_of::<InstanceData>(),
            self.max_frames_in_flight
        );
        log::info!("instances={} draw_calls=1", INSTANCE_COUNT);
        Ok(())
    }

    /// Gribb–Hartmann：从 proj*view 提取 6 个视锥平面（法线朝内、归一化）
    ///
    /// 公式来源：G. Gribb, K. Hartmann, "Fast Extraction of Viewing Frustum
    /// Planes from the World-View-Projection Matrix" (2001)。
    /// 平面系数来自 M 的行向量组合：左=r3+r0、右=r3−r0、下=r3+r1、上=r3−r1；
    /// Vulkan NDC z∈[0,1]，故近=r2、远=r3−r2。内部满足 dot(n,c)+d ≥ 0。
    fn extract_frustum_planes(
        view: glam::Mat4,
        proj: glam::Mat4,
    ) -> [[f32; 4]; 6] {
        Self::extract_frustum_planes_from(proj * view)
    }

    /// 与 `extract_frustum_planes` 同一套数学，但直接吃**已相乘**的矩阵。
    ///
    /// 加它的理由：阴影 pass 需要的是**光源**视锥（`light_data.shadow.light_view_proj`），
    /// 而那一条路径上只有乘积、没有分开的 view/proj。把平面提取收敛到一处，
    /// 比在阴影 pass 里手抄一遍 6 个平面的符号组合安全 —— 抄错一个符号就是
    /// "影子随机缺一块"，且不报任何错。
    fn extract_frustum_planes_from(m4: glam::Mat4) -> [[f32; 4]; 6] {
        let m = m4.to_cols_array_2d(); // m[col][row]
        let row = |i: usize| [m[0][i], m[1][i], m[2][i], m[3][i]];
        let r0 = row(0);
        let r1 = row(1);
        let r2 = row(2);
        let r3 = row(3);

        let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
        let sub = |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
        let normalize = |p: [f32; 4]| {
            let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            [p[0] / len, p[1] / len, p[2] / len, p[3] / len]
        };

        [
            normalize(add(r3, r0)), // 左
            normalize(sub(r3, r0)), // 右
            normalize(add(r3, r1)), // 下
            normalize(sub(r3, r1)), // 上
            normalize(r2),          // 近
            normalize(sub(r3, r2)), // 远
        ]
    }

    /// 标量视锥剔除（回退路径）：对每个实例判 6 平面，d = dot(n,c)+d，
    /// 任一平面 d < -r 即剔除；全部平面 d >= -r 才可见。
    fn cull_spheres_scalar(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        for i in 0..cx.len() {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[i] + p[1] * cy[i] + p[2] * cz[i] + p[3];
                if d < -radii[i] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(i as u32);
            }
        }
    }

    /// AVX2 批量视锥剔除：8 实例/批（256 位 8×f32），6 平面点积全部向量化。
    /// 与标量版逐位一致：非 FMA，累加顺序严格 ((nx*x+ny*y)+nz*z)+d，
    /// 比较 d >= -r 得保留掩码并按实例序输出，无 NaN 输入（全部有限数）。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn cull_spheres_avx2(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        use std::arch::x86_64::*;
        let n = cx.len();
        debug_assert_eq!(n, cy.len());
        debug_assert_eq!(n, cz.len());
        debug_assert_eq!(n, radii.len());

        let mut i = 0usize;
        while i + 8 <= n {
            let xv = _mm256_loadu_ps(cx.as_ptr().add(i));
            let yv = _mm256_loadu_ps(cy.as_ptr().add(i));
            let zv = _mm256_loadu_ps(cz.as_ptr().add(i));
            let rv = _mm256_loadu_ps(radii.as_ptr().add(i));
            // 可见掩码：初始全 1，任一平面剔除则清零
            let mut vis = _mm256_castsi256_ps(_mm256_set1_epi32(-1));
            let neg_r = _mm256_sub_ps(_mm256_setzero_ps(), rv);
            for p in planes {
                let nx = _mm256_set1_ps(p[0]);
                let ny = _mm256_set1_ps(p[1]);
                let nz = _mm256_set1_ps(p[2]);
                let pd = _mm256_set1_ps(p[3]);
                // d = ((nx*x + ny*y) + nz*z) + pd，与标量加法顺序一致（无 FMA）
                let d = _mm256_add_ps(
                    _mm256_add_ps(
                        _mm256_add_ps(_mm256_mul_ps(nx, xv), _mm256_mul_ps(ny, yv)),
                        _mm256_mul_ps(nz, zv),
                    ),
                    pd,
                );
                // 保留条件 d >= -r（NaN 不可能出现，有序比较安全）
                let keep = _mm256_cmp_ps(d, neg_r, _CMP_GE_OQ);
                vis = _mm256_and_ps(vis, keep);
            }
            let mask = _mm256_movemask_ps(vis) as u32;
            for k in 0..8u32 {
                if mask & (1 << k) != 0 {
                    out.push((i + k as usize) as u32);
                }
            }
            i += 8;
        }
        // 尾部不足 8 个走标量
        for j in i..n {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[j] + p[1] * cy[j] + p[2] * cz[j] + p[3];
                if d < -radii[j] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(j as u32);
            }
        }
    }

    /// AVX-512 批量视锥剔除：16 实例/批（512 位 16×f32），6 平面点积全部向量化。
    /// 与标量版逐位一致：非 FMA，累加顺序严格 ((nx*x+ny*y)+nz*z)+d，
    /// 比较 d >= -r 得 16 位掩码并按实例序输出，无 NaN 输入（全部有限数）。
    /// 适用 Zen4/Zen5（7000/9000 系）原生 512 位单元，功耗增量可忽略。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx512f")]
    unsafe fn cull_spheres_avx512(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        use std::arch::x86_64::*;
        let n = cx.len();
        debug_assert_eq!(n, cy.len());
        debug_assert_eq!(n, cz.len());
        debug_assert_eq!(n, radii.len());

        let mut i = 0usize;
        while i + 16 <= n {
            let xv = _mm512_loadu_ps(cx.as_ptr().add(i));
            let yv = _mm512_loadu_ps(cy.as_ptr().add(i));
            let zv = _mm512_loadu_ps(cz.as_ptr().add(i));
            let rv = _mm512_loadu_ps(radii.as_ptr().add(i));
            // 可见掩码：初始全 1，任一平面剔除则清零
            let mut vis: __mmask16 = 0xFFFF;
            let neg_r = _mm512_sub_ps(_mm512_setzero_ps(), rv);
            for p in planes {
                let nx = _mm512_set1_ps(p[0]);
                let ny = _mm512_set1_ps(p[1]);
                let nz = _mm512_set1_ps(p[2]);
                let pd = _mm512_set1_ps(p[3]);
                // d = ((nx*x + ny*y) + nz*z) + pd，与标量加法顺序一致（无 FMA）
                let d = _mm512_add_ps(
                    _mm512_add_ps(
                        _mm512_add_ps(_mm512_mul_ps(nx, xv), _mm512_mul_ps(ny, yv)),
                        _mm512_mul_ps(nz, zv),
                    ),
                    pd,
                );
                // 保留条件 d >= -r（NaN 不可能出现，有序比较安全）
                let keep = _mm512_cmp_ps_mask(d, neg_r, _CMP_GE_OQ);
                vis &= keep;
            }
            for k in 0..16u32 {
                if vis & (1 << k) != 0 {
                    out.push((i + k as usize) as u32);
                }
            }
            i += 16;
        }
        // 尾部不足 16 个走标量
        for j in i..n {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[j] + p[1] * cy[j] + p[2] * cz[j] + p[3];
                if d < -radii[j] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(j as u32);
            }
        }
    }

    /// AVX（非 AVX2，第 3/4 代酷睿与初代锐龙）批量剔除：8 实例/批（256 位 8×f32）。
    /// 与标量逐位一致（非 FMA 累加顺序相同）；浮点 add/mul/cmp 在 AVX 即已具备，
    /// 无 AVX2 的 FMA/gather 需求，故可独立成档。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx")]
    unsafe fn cull_spheres_avx(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        use std::arch::x86_64::*;
        let n = cx.len();
        debug_assert_eq!(n, cy.len());
        debug_assert_eq!(n, cz.len());
        debug_assert_eq!(n, radii.len());

        let mut i = 0usize;
        while i + 8 <= n {
            let xv = _mm256_loadu_ps(cx.as_ptr().add(i));
            let yv = _mm256_loadu_ps(cy.as_ptr().add(i));
            let zv = _mm256_loadu_ps(cz.as_ptr().add(i));
            let rv = _mm256_loadu_ps(radii.as_ptr().add(i));
            let mut vis = _mm256_castsi256_ps(_mm256_set1_epi32(-1));
            let neg_r = _mm256_sub_ps(_mm256_setzero_ps(), rv);
            for p in planes {
                let nx = _mm256_set1_ps(p[0]);
                let ny = _mm256_set1_ps(p[1]);
                let nz = _mm256_set1_ps(p[2]);
                let pd = _mm256_set1_ps(p[3]);
                let d = _mm256_add_ps(
                    _mm256_add_ps(
                        _mm256_add_ps(_mm256_mul_ps(nx, xv), _mm256_mul_ps(ny, yv)),
                        _mm256_mul_ps(nz, zv),
                    ),
                    pd,
                );
                let keep = _mm256_cmp_ps(d, neg_r, _CMP_GE_OQ);
                vis = _mm256_and_ps(vis, keep);
            }
            let mask = _mm256_movemask_ps(vis) as u32;
            for k in 0..8u32 {
                if mask & (1 << k) != 0 {
                    out.push((i + k as usize) as u32);
                }
            }
            i += 8;
        }
        // 尾部不足 8 个走标量
        for j in i..n {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[j] + p[1] * cy[j] + p[2] * cz[j] + p[3];
                if d < -radii[j] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(j as u32);
            }
        }
    }

    /// SSE4.2（2008 年后所有 Intel/AMD 消费级）批量剔除：4 实例/批（128 位 4×f32）。
    /// 与标量逐位一致（非 FMA 累加顺序相同）；比标量约 2-3×，覆盖无 AVX 的老平台。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "sse4.2")]
    unsafe fn cull_spheres_sse(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        use std::arch::x86_64::*;
        let n = cx.len();
        debug_assert_eq!(n, cy.len());
        debug_assert_eq!(n, cz.len());
        debug_assert_eq!(n, radii.len());

        let mut i = 0usize;
        while i + 4 <= n {
            let xv = _mm_loadu_ps(cx.as_ptr().add(i));
            let yv = _mm_loadu_ps(cy.as_ptr().add(i));
            let zv = _mm_loadu_ps(cz.as_ptr().add(i));
            let rv = _mm_loadu_ps(radii.as_ptr().add(i));
            let mut vis = _mm_castsi128_ps(_mm_set1_epi32(-1));
            let neg_r = _mm_sub_ps(_mm_setzero_ps(), rv);
            for p in planes {
                let nx = _mm_set1_ps(p[0]);
                let ny = _mm_set1_ps(p[1]);
                let nz = _mm_set1_ps(p[2]);
                let pd = _mm_set1_ps(p[3]);
                let d = _mm_add_ps(
                    _mm_add_ps(
                        _mm_add_ps(_mm_mul_ps(nx, xv), _mm_mul_ps(ny, yv)),
                        _mm_mul_ps(nz, zv),
                    ),
                    pd,
                );
                let keep = _mm_cmp_ps(d, neg_r, _CMP_GE_OQ);
                vis = _mm_and_ps(vis, keep);
            }
            let mask = _mm_movemask_ps(vis) as u32;
            for k in 0..4u32 {
                if mask & (1 << k) != 0 {
                    out.push((i + k as usize) as u32);
                }
            }
            i += 4;
        }
        // 尾部不足 4 个走标量
        for j in i..n {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[j] + p[1] * cy[j] + p[2] * cz[j] + p[3];
                if d < -radii[j] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(j as u32);
            }
        }
    }

    /// NEON（AArch64，Apple Silicon / Android / 高通 X Elite 通用）批量剔除：
    /// 4 实例/批（128 位 4×f32）。与标量逐位一致（非 FMA 累加顺序相同）；
    /// Apple Silicon 的 SIMD 即标准 NEON（AMX 为私有协处理器，SVE 苹果不支持）。
    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon")]
    unsafe fn cull_spheres_neon(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        use std::arch::aarch64::*;
        let n = cx.len();
        debug_assert_eq!(n, cy.len());
        debug_assert_eq!(n, cz.len());
        debug_assert_eq!(n, radii.len());

        let mut i = 0usize;
        while i + 4 <= n {
            let xv = vld1q_f32(cx.as_ptr().add(i));
            let yv = vld1q_f32(cy.as_ptr().add(i));
            let zv = vld1q_f32(cz.as_ptr().add(i));
            let rv = vld1q_f32(radii.as_ptr().add(i));
            // 可见掩码：初始全 1（u32 lane），任一平面剔除则清零
            let mut vis = vdupq_n_u32(0xFFFF_FFFF);
            let neg_r = vsubq_f32(vdupq_n_f32(0.0), rv);
            for p in planes {
                let nx = vdupq_n_f32(p[0]);
                let ny = vdupq_n_f32(p[1]);
                let nz = vdupq_n_f32(p[2]);
                let pd = vdupq_n_f32(p[3]);
                // d = ((nx*x + ny*y) + nz*z) + pd，与标量加法顺序一致（无 FMA）
                let d = vaddq_f32(
                    vaddq_f32(
                        vaddq_f32(vmulq_f32(nx, xv), vmulq_f32(ny, yv)),
                        vmulq_f32(nz, zv),
                    ),
                    pd,
                );
                // 保留条件 d >= -r（NaN 不可能出现）
                let keep = vcgeq_f32(d, neg_r);
                vis = vandq_u32(vis, keep);
            }
            // 提取 4 位可见掩码（每 lane 全 1/全 0，取最低位）
            let mask = (vgetq_lane_u32(vis, 0) & 1)
                | ((vgetq_lane_u32(vis, 1) & 1) << 1)
                | ((vgetq_lane_u32(vis, 2) & 1) << 2)
                | ((vgetq_lane_u32(vis, 3) & 1) << 3);
            for k in 0..4u32 {
                if mask & (1 << k) != 0 {
                    out.push((i + k as usize) as u32);
                }
            }
            i += 4;
        }
        // 尾部不足 4 个走标量
        for j in i..n {
            let mut visible = true;
            for p in planes {
                let d = p[0] * cx[j] + p[1] * cy[j] + p[2] * cz[j] + p[3];
                if d < -radii[j] {
                    visible = false;
                    break;
                }
            }
            if visible {
                out.push(j as u32);
            }
        }
    }

    /// 每帧视锥剔除 + 距离 LOD 分档：
    /// 设置世界障碍 marker（关卡切换时由 main.rs 调用；容量截断到 MAX_MARKER_INSTANCES）
    pub fn set_world_markers(&mut self, markers: &[WorldMarker]) {
        self.markers = markers
            .iter()
            .take(MAX_MARKER_INSTANCES as usize)
            .map(|m| InstanceData {
                model: m.model.to_cols_array(),
                tint: m.tint,
            })
            .collect();
    }

    /// 追加一批世界 marker（弹孔等每帧变化的实例），容量仍截断到 `MAX_MARKER_INSTANCES`。
    ///
    /// 与 `set_world_markers` 分开的理由是 **PT 场景不吃这一批**：弹孔每次开枪都变，
    /// 混进 `pt_set_scene_markers` 会让 BLAS 指纹每帧判"场景变了"而重建，
    /// 还会白占 PT 盒容量（`PT_MAX_BOXES`）。
    pub fn append_markers(&mut self, extra: &[WorldMarker]) {
        let room = (MAX_MARKER_INSTANCES as usize).saturating_sub(self.markers.len());
        self.markers
            .extend(extra.iter().take(room).map(|m| InstanceData {
                model: m.model.to_cols_array(),
                tint: m.tint,
            }));
    }

    /// 每帧上传世界障碍 marker 到实例 buffer 的 MARKER_SLOT_BASE 之后区域
    /// （跳过 65536 identity slot，见 MARKER_SLOT_BASE 注释），返回 (近档, 远档) 计数。
    /// marker 量小（≤64），不做视锥剔除，仅按距离分近/远档。
    fn upload_markers(&mut self, cam_pos: glam::Vec3) -> (u32, u32) {
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return (0, 0),
        };
        let stride = std::mem::size_of::<InstanceData>();
        if self.mesh_enabled {
            // mesh 路径：不做近/远压缩，顺序写槽位（几何由 shader 按距离自选）。
            // 返回 (count, 0)：计数仍供 draw 范围与性能日志使用。
            let count = self.markers.len() as u32;
            for (i, inst) in self.markers.iter().enumerate() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((MARKER_SLOT_BASE + i as u32) as usize) * stride),
                        stride,
                    );
                }
            }
            return (count, 0);
        }
        // 近/远档分界距离随画质预设变化（Medium 与原 LOD_DISTANCE 一致）。
        // 障碍 marker 恒走近档立方体：远档十字 quad 俯视呈"方块贴图+缝隙"（用户反馈）。
        let near_sq = f32::MAX;
        let mut near_count = 0u32;
        // 近档先写（base..base+near-1），远档紧随（base+near..），两遍遍历避免槽位交错
        for inst in &self.markers {
            let dx = inst.model[12] - cam_pos.x;
            let dy = inst.model[13] - cam_pos.y;
            let dz = inst.model[14] - cam_pos.z;
            if dx * dx + dy * dy + dz * dz < near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((MARKER_SLOT_BASE + near_count) as usize) * stride),
                        stride,
                    );
                }
                near_count += 1;
            }
        }
        let mut far_count = 0u32;
        for inst in &self.markers {
            let dx = inst.model[12] - cam_pos.x;
            let dy = inst.model[13] - cam_pos.y;
            let dz = inst.model[14] - cam_pos.z;
            if dx * dx + dy * dy + dz * dz >= near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(
                            ((MARKER_SLOT_BASE + near_count + far_count) as usize) * stride,
                        ),
                        stride,
                    );
                }
                far_count += 1;
            }
        }
        (near_count, far_count)
    }

    /// 设置自发光实体（爆炸闪光等；每帧由 main.rs 传入，容量截断到 MAX_EMISSIVE_INSTANCES）
    pub fn set_emissive_markers(&mut self, markers: &[WorldMarker]) {
        self.emissive_markers = markers
            .iter()
            .take(MAX_EMISSIVE_INSTANCES as usize)
            .map(|m| InstanceData {
                model: m.model.to_cols_array(),
                tint: m.tint,
            })
            .collect();
    }

    /// 每帧上传自发光实体到实例 buffer 的 EMISSIVE_SLOT_BASE 之后区域，返回 (近档, 远档) 计数。
    /// 与 marker 同构：量小不剔除，仅按距离分近/远档（shader 侧 flat+fade>1 直出自发光色）。
    fn upload_emissive(&mut self, cam_pos: glam::Vec3) -> (u32, u32) {
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return (0, 0),
        };
        let stride = std::mem::size_of::<InstanceData>();
        if self.mesh_enabled {
            let count = self.emissive_markers.len() as u32;
            for (i, inst) in self.emissive_markers.iter().enumerate() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((EMISSIVE_SLOT_BASE + i as u32) as usize) * stride),
                        stride,
                    );
                }
            }
            return (count, 0);
        }
        let near_sq = quality_params(self.quality).instance_lod_distance;
        let near_sq = near_sq * near_sq;
        let mut near_count = 0u32;
        for inst in &self.emissive_markers {
            let dx = inst.model[12] - cam_pos.x;
            let dy = inst.model[13] - cam_pos.y;
            let dz = inst.model[14] - cam_pos.z;
            if dx * dx + dy * dy + dz * dz < near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((EMISSIVE_SLOT_BASE + near_count) as usize) * stride),
                        stride,
                    );
                }
                near_count += 1;
            }
        }
        let mut far_count = 0u32;
        for inst in &self.emissive_markers {
            let dx = inst.model[12] - cam_pos.x;
            let dy = inst.model[13] - cam_pos.y;
            let dz = inst.model[14] - cam_pos.z;
            if dx * dx + dy * dy + dz * dz >= near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(
                            ((EMISSIVE_SLOT_BASE + near_count + far_count) as usize) * stride,
                        ),
                        stride,
                    );
                }
                far_count += 1;
            }
        }
        (near_count, far_count)
    }

    /// 计算一名 NPC 的 15 段人形士兵实例数据（大腿/小腿/脚/骨盆/胸/颈/头/上臂/前臂/枪）。
    /// 比例贴近真人：总高约 1.78m，肩宽 ~0.55m，腿/臂分上下两段，行走时髋/膝/肩/肘
    /// 各自绕枢轴摆动（不再是积木式整体摆动）。全部同 tint。
    /// 每段矩阵 = T(pos) * R_y(yaw) * T(枢轴) * R_anim * T(段心) * S(尺寸)：
    /// 动画旋转在枢轴平移之后、段心平移之前，绕枢轴（髋/膝/肩/肘）旋转。
    /// 枪局部偏移在 +Z（yaw=0 时枪口朝向 +Z），随 yaw 绕 y 轴旋转。
    /// 动画：moving 时髋/膝/肩/肘按 phase 正弦对向摆动（走路步态）；
    /// firing 时枪沿 -Z 后坐脉冲（高频正弦），胸微俯。
    /// 返回 (盒体组, 圆柱组, 球体组) 三段实例：躯干/脚/枪为盒体，
    /// 四肢为圆柱（半径 = 盒宽/厚的一半），头为球体 —— 真人比例、非方块人。
    fn soldier_part_matrices(
        pos: [f32; 3],
        yaw: f32,
        tint: [f32; 4],
        phase: f32,
        moving: bool,
        firing: bool,
    ) -> (Vec<InstanceData>, Vec<InstanceData>, Vec<InstanceData>) {
        // (枢轴, 缩放, 段心相对枢轴偏移, 动画类型, 几何: 0盒 1圆柱 2球)
        // 动画类型：0 无 1 左大腿 2 右大腿 3 左小腿 4 右小腿 5 左上臂 6 右上臂
        //          7 左前臂 8 右前臂 9 枪(后坐) 10 胸(前俯)
        // 人形比例（总高 ~1.79m）：头小、躯干桶形（圆柱）、四肢粗细适中、有脚。
        // 圆柱 scale = (半径, 高, 半径)；盒 scale = (宽, 高, 厚)。
        // 段数预算：255 人 × 每组 3072 ⇒ **每组每人最多 12 段**。本表 = 9 盒 + 8 圆柱
        // （2295 / 2040，分别占 75% / 66%），都留了余量。加段之前先复核这个预算。
        // ── 持枪姿态角（2026-09-12 第④条修）：负角 = 肢体向 +Z（面朝方向）抬起。
        //
        // 旧版两条手臂只有走路摆动（kind 5~8 绕 X 随 `stride` 摆），而枪固定在身前
        // `(x=+0.16, z=+0.36, y=1.18)` ⇒ **枪悬在胸前、手臂垂在身侧，两者永不相交**。
        // 实机放大图（`screenshots/soldier_zoom.png`）里读作"左肩伸出一根悬空的横条"、
        // 且"看不见手臂" —— 这是用户说的"神人样子"的主要来源。
        //
        // 改成持枪后**手臂不再随步伐摆动**：真人端枪行进时本来就不摆臂，比摆动更真。
        // 只改姿态角、**不加段数**，实例预算（9 盒 + 8 圆柱）不变。
        const HOLD_R_UPPER: f32 = -1.00; // 右上臂前抬 ~57°
        const HOLD_R_FORE: f32 = -0.85;  // 右前臂再前抬 ~49° ⇒ 手到握把高度
        const HOLD_L_UPPER: f32 = -1.15; // 左上臂前抬 ~66°（托护木，比右手更前）
        const HOLD_L_FORE: f32 = -0.70;  // 左前臂 ~40°
        // 段尾的 f32 = **逐段明暗系数**（2026-09-12 第④条修）。
        //
        // 引擎对 NPC 走 `flat_flag=2` 纯色路径，`tint` **就是**外观色，顶点色是白化的。
        // 因此 17 段共用一个 tint ⇒ 整个人是一个**均匀饱和色块**，
        // 平着色下没有任何结构可读（实机放大图上就是一团橙色塑料）——
        // 这是"神人样子"的第二大来源（第一大是枪悬空，已修）。
        //
        // 真人身上的装备本来就有明显的**明度层次**：盔最暗、背心次之、作训服中、靴最暗。
        // 这里给每段一个亮度系数，**不加段数、不加 draw call**（还是同一批实例）。
        let parts: [([f32; 3], [f32; 3], [f32; 3], u8, u8); 18] = [
            // ── 四肢：保持圆柱（转动最自然），但**加粗到真人尺寸**。旧值大腿 φ0.13 /
            //    小腿 φ0.10 / 前臂 φ0.084 —— 比真人细一倍，远看只剩躯干那根柱子，
            //    这就是"远距离读作蓝色平板"的来源。
            ([-0.10, 0.84, 0.0], [0.085, 0.38, 0.085], [0.0, -0.19, 0.0], 1, 1), // 左大腿（髋）圆柱
            ([0.10, 0.84, 0.0], [0.085, 0.38, 0.085], [0.0, -0.19, 0.0], 2, 1),  // 右大腿（髋）圆柱
            ([-0.095, 0.46, 0.0], [0.062, 0.36, 0.062], [0.0, -0.18, 0.0], 3, 1), // 左小腿（膝）圆柱
            ([0.095, 0.46, 0.0], [0.062, 0.36, 0.062], [0.0, -0.18, 0.0], 4, 1),  // 右小腿（膝）圆柱
            ([-0.09, 0.10, 0.0], [0.11, 0.10, 0.26], [0.0, -0.05, 0.02], 0, 0),   // 左脚（盒，抬到踝高）
            ([0.09, 0.10, 0.0], [0.11, 0.10, 0.26], [0.0, -0.05, 0.02], 0, 0),    // 右脚（盒）
            // ── 躯干：**扁箱**而不是圆柱。真人胸廓宽 > 深（0.36 × 0.24），圆柱躯干
            //    在剪影上就是一根柱子；而且旧胸半径 0.20 < 上臂挂点 0.28，
            //    手臂是**悬空挂在躯干外面**的。
            ([0.0, 0.96, 0.0], [0.32, 0.20, 0.24], [0.0, -0.10, 0.0], 0, 0),      // 骨盆（扁箱）
            // 🔴 2026-09-12 第④条收窄（第 122 轮）：实宽由 0.72 收到 **0.48**。
            //    依据：`tools/measure_silhouette.py` 量出躯干**投影**宽 1.12m；
            //    扣除投影角度（宽 0.78/深 0.58、约 37 度 ⇒ 最大 0.97）与透视放大（1.07）
            //    后，模型实宽约 0.78m —— 真人含护甲约 0.52m ⇒ **宽约 1.5 倍**。
            //    （第 120 轮误记为"2.2~2.5 倍"，那是把投影宽当成了模型宽。）
            ([0.0, 1.26, 0.0], [0.36, 0.46, 0.24], [0.0, -0.01, -0.01], 10, 0),   // 胸廓（扁箱，含前俯）
            // 🔴 2026-09-12 第④条修：背心/头的 y 区间原本**重叠 0.13m**
            //    （背心 1.23~1.53 vs 头 1.40~1.64）⇒ **头有 43% 埋在背心里**，
            //    平着色下读作"一整块大方块"，这就是"神人样子"最直接的一条。
            //    按真人重排：背心 1.20~1.50（pivot 1.35），头 1.50~1.74（pivot 1.62），
            //    头盔 1.645~1.795（pivot 1.72）⇒ 总高 1.795，且**三段首尾相接不重叠**。
            ([0.0, 1.35, 0.0], [0.39, 0.30, 0.29], [0.0, 0.0, 0.0], 10, 0),       // 防弹背心（套在胸外）
            // ── 头：方块 + 头盔两段。旧版是一个 φ0.30 的**球**，没有下颌、没有头盔、
            //    没有朝向；平着色下一个球就是一块均匀色斑。
            // 🔴🔴 2026-09-12 第 136 轮**回退第 133 轮的收小**：同上是"半宽"外推的产物。
            //    盒 scale 是全尺寸 ⇒ 头 0.17x0.24x0.20、盔 0.205x0.15x0.235 **本来就是真人尺寸**
            //    （真人头约 0.16x0.24x0.20、盔约 0.22x0.15x0.28）。第 133 轮把它们改小了一半，是错的。
            ([0.0, 1.62, 0.0], [0.17, 0.24, 0.20], [0.0, 0.0, 0.0], 0, 0),        // 头（含下颌，方块）
            ([0.0, 1.72, 0.0], [0.205, 0.15, 0.235], [0.0, 0.0, 0.0], 0, 0),      // 头盔壳
            // ── 手臂：圆柱加粗，并把挂点从 ±0.28 收到 ±0.235 —— 贴着胸廓外侧
            ([-0.235, 1.38, 0.02], [0.068, 0.26, 0.068], [0.0, -0.13, 0.0], 5, 1), // 左上臂（肩）圆柱
            ([0.235, 1.38, 0.02], [0.068, 0.26, 0.068], [0.0, -0.13, 0.0], 6, 1),  // 右上臂（肩）圆柱
            ([-0.235, 1.10, 0.02], [0.058, 0.24, 0.058], [0.0, -0.12, 0.02], 7, 1), // 左前臂（肘）圆柱
            ([0.235, 1.10, 0.02], [0.058, 0.24, 0.058], [0.0, -0.12, 0.02], 8, 1),  // 右前臂（肘）圆柱
            // ── 武器：枪身 + 枪托两段（旧版是一个 0.26×0.10×0.95 的**纯方块**）
            // 🔴 2026-09-12 第 137 轮回退第 132 轮：scale[2] 0.47 -> 0.62（原值）。上一条注释已写明旧版是 0.26x0.10x0.95 的整块 => 0.95m 正是 AK-12 长度，枪身0.62+枪托0.24 就是拆成两段的结果 => 原值本来就对。第132轮按错误的"半宽"前提把0.62读成1.24m才去缩短；第136轮定案盒scale是全尺寸。
            //    （真 AK-12 全枪约 0.94 m）。原值在侧视投影里是一根 1.24m 的"水平细长横杆"，
            //    我连续三轮（114/129/130）把它误认成手臂 —— 直到第 131 轮"改上臂角 26° 它却不动"才定案。
            ([0.16, 1.18, 0.36], [0.07, 0.10, 0.62], [0.0, 0.0, 0.0], 9, 0),      // 枪身
            ([0.16, 1.14, -0.04], [0.06, 0.13, 0.24], [0.0, 0.0, 0.0], 9, 0),     // 枪托
            // ── 背包（2026-09-12 第④条加）：盒体第 10 段。盒预算 9→10，
            //    10×255 = 2550 / 3072 = 83%，**仍留 17% 余量**（上限 12 段 = 3060/3072）。
            //    纯侧影改造：平着色下"背上有东西"是区分士兵与方柱最省的一段几何。
            // 🔴 2026-09-12 修正：初版做成 0.30 x **0.42** x 0.16 —— 从背后看**整个背面被它盖住**，
            //    实机特写（`screenshots/soldier_zoom5.png`）里读作"一个大方块躯干"，
            //    反而比不加更糟。真人背包约 0.30 宽 x 0.40 高但**贴身**（深 0.16），
            //    关键是它不该高过肩胛 —— 收到 0.26 x 0.30，读作"背上有东西"即可，不抢主体。
            ([0.0, 1.28, -0.20], [0.26, 0.30, 0.14], [0.0, 0.0, 0.0], 10, 0),     // 背包（跟着胸俯仰）
        ];
        // 逐段明暗系数（顺序严格对应上面的 17 段）：盔最暗、靴最暗、枪近黑、背心最亮。
        // 真人装备本来就有明显明度层次，平着色下这是**唯一**能读出结构的手段。
        let shade: [f32; 18] = [
            0.72, 0.72, // 左/右大腿
            0.66, 0.66, // 左/右小腿
            0.42, 0.42, // 左/右脚（靴，最暗）
            0.80, // 骨盆
            0.95, // 胸廓
            1.00, // 防弹背心（装备主体，最亮）
            // 🔴 2026-09-12 修：这两个系数原本是**反的**（头 0.70 亮于盔 0.55）
            //    ⇒ 平着色下"脸比盔亮"，读作一块均匀方块，**头没有正面**。
            //    真人脸上有盔影 + 护目镜，是全身最暗的一块；盔是受光面，最亮之一。
            //    对调之后头才第一次有了"朝向"。
            0.42, // 头/脸（暗：盔影 + 护目镜）
            0.78, // 头盔壳（亮：受光面）
            0.78, 0.78, // 左/右上臂
            0.74, 0.74, // 左/右前臂
            0.30, 0.30, // 枪身/枪托（近黑）
            0.62, // 背包（比胸廓暗、比盔亮，读作"背上的织物"）
        ];
        let trans = glam::Mat4::from_translation(glam::Vec3::from(pos));
        let rot = glam::Mat4::from_rotation_y(yaw);
        // 步态：髋/膝/肩/肘绕各自枢轴对向摆动，频率 ~2.2Hz 视觉节奏
        let stride = if moving {
            (phase * 13.8).sin().clamp(-1.0, 1.0) * 0.55
        } else {
            0.0
        };
        // 开火后坐：枪沿 -Z 脉冲（~7Hz 快速衰减），胸轻微前俯
        let (kick, torso_lean) = if firing {
            let k = ((phase * 44.0).sin().abs()).min(1.0);
            (0.09 * k, -0.06 * k)
        } else {
            (0.0, 0.0)
        };
        let mut box_out: Vec<InstanceData> = Vec::with_capacity(6);
        let mut cyl_out: Vec<InstanceData> = Vec::with_capacity(8);
        let mut sph_out: Vec<InstanceData> = Vec::with_capacity(1);
        for (i, (pivot, scale, center, kind, geom)) in parts.iter().enumerate() {
            let mut anim = glam::Mat4::IDENTITY;
            match kind {
                1 => anim *= glam::Mat4::from_rotation_x(stride),        // 左大腿
                2 => anim *= glam::Mat4::from_rotation_x(-stride),       // 右大腿
                3 => anim *= glam::Mat4::from_rotation_x(-stride * 0.5), // 左小腿（膝弯反向）
                4 => anim *= glam::Mat4::from_rotation_x(stride * 0.5),  // 右小腿
                5 => anim *= glam::Mat4::from_rotation_x(HOLD_L_UPPER), // 左上臂：持枪（托护木）
                6 => anim *= glam::Mat4::from_rotation_x(HOLD_R_UPPER), // 右上臂：持枪（握把）
                7 => anim *= glam::Mat4::from_rotation_x(HOLD_L_FORE),  // 左前臂
                8 => anim *= glam::Mat4::from_rotation_x(HOLD_R_FORE),  // 右前臂
                9 => anim *= glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, -kick)), // 枪后坐
                10 => anim *= glam::Mat4::from_rotation_x(torso_lean),   // 胸微俯
                _ => {}
            }
            let model = trans
                * rot
                * glam::Mat4::from_translation(glam::Vec3::from(*pivot))
                * anim
                * glam::Mat4::from_translation(glam::Vec3::from(*center))
                * glam::Mat4::from_scale(glam::Vec3::from(*scale));
            let inst = InstanceData {
                model: model.to_cols_array(),
                // 逐段明暗：NPC 走纯色路径，`tint` 就是外观色；乘上本段系数即得装备层次。
                tint: [tint[0] * shade[i], tint[1] * shade[i], tint[2] * shade[i], tint[3]],
            };
            match geom {
                1 => cyl_out.push(inst),
                2 => sph_out.push(inst),
                _ => box_out.push(inst),
            }
        }
        (box_out, cyl_out, sph_out)
    }

    /// 倒地尸体姿态：14 段人体绕 X 轴躺倒（-90° 侧卧）贴地摊开，枪横置身侧，
    /// tint 按阵营保留（尸体可辨识）。
    fn dead_part_matrices(
        pos: [f32; 3],
        yaw: f32,
        tint: [f32; 4],
    ) -> (Vec<InstanceData>, Vec<InstanceData>, Vec<InstanceData>) {
        // (立姿局部偏移, 缩放, 几何: 0盒 1圆柱 2球)：躺倒时偏移 (x, y_stand*0.3, rest_z)，
        // 经 lie 旋转后世界位置 = (x, rest_z, -y_stand*0.3)：rest_z 保证部件贴地不埋入。
        let parts: [([f32; 3], [f32; 3], u8); 14] = [
            ([-0.10, 0.285, 0.24], [0.065, 0.38, 0.065], 1),  // 左大腿（圆柱）
            ([0.10, 0.285, 0.24], [0.065, 0.38, 0.065], 1),   // 右大腿（圆柱）
            ([-0.09, 0.153, 0.23], [0.05, 0.36, 0.05], 1),    // 左小腿（圆柱）
            ([0.09, 0.153, 0.23], [0.05, 0.36, 0.05], 1),     // 右小腿（圆柱）
            ([-0.09, 0.015, 0.16], [0.09, 0.05, 0.24], 0),    // 左脚（盒）
            ([0.09, 0.015, 0.16], [0.09, 0.05, 0.24], 0),     // 右脚（盒）
            ([0.0, 0.294, 0.15], [0.17, 0.18, 0.19], 1),      // 骨盆（圆柱）
            ([0.0, 0.372, 0.24], [0.20, 0.48, 0.20], 1),      // 胸（桶形圆柱）
            ([0.0, 0.441, 0.07], [0.05, 0.06, 0.05], 0),      // 颈（盒）
            ([0.0, 0.489, 0.155], [0.15, 0.17, 0.15], 2),     // 头（球体，φ≈0.30m）
            ([-0.28, 0.42, 0.17], [0.05, 0.26, 0.05], 1),     // 左上臂（圆柱）
            ([0.28, 0.42, 0.17], [0.05, 0.26, 0.05], 1),      // 右上臂（圆柱）
            ([-0.28, 0.33, 0.16], [0.042, 0.24, 0.042], 1),   // 左前臂（圆柱）
            ([0.28, 0.33, 0.16], [0.042, 0.24, 0.042], 1),    // 右前臂（圆柱）
        ];
        let trans = glam::Mat4::from_translation(glam::Vec3::from(pos));
        let rot = glam::Mat4::from_rotation_y(yaw);
        let lie = glam::Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let mut box_out: Vec<InstanceData> = Vec::with_capacity(6);
        let mut cyl_out: Vec<InstanceData> = Vec::with_capacity(8);
        let mut sph_out: Vec<InstanceData> = Vec::with_capacity(1);
        for (off, scale, geom) in parts.iter() {
            let model = trans
                * rot
                * lie
                * glam::Mat4::from_translation(glam::Vec3::from(*off))
                * glam::Mat4::from_scale(glam::Vec3::from(*scale));
            let inst = InstanceData {
                model: model.to_cols_array(),
                tint,
            };
            match geom {
                1 => cyl_out.push(inst),
                2 => sph_out.push(inst),
                _ => box_out.push(inst),
            }
        }
        // 枪横置身侧：绕 Y 转 90° 使枪管沿 +X，贴地平放
        let gun = trans
            * rot
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.08, 0.62))
            * glam::Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2)
            * glam::Mat4::from_scale(glam::Vec3::new(0.13, 0.10, 0.95));
        box_out.push(InstanceData {
            model: gun.to_cols_array(),
            tint,
        });
        (box_out, cyl_out, sph_out)
    }

    /// 设置 NPC 士兵可视化（由 main.rs 传入全部 NPC 的位置/朝向/配色/动画态；
    /// 15 段/人（尸体 15 段）展开存入 npc_parts，总段数截断到 MAX_NPC_INSTANCES）
    /// NPC 段数触顶时的**一次性**告警（2026-09-14）。
    ///
    /// 背景：`set_npc_visuals` / `set_dead_bodies` 里各有一处
    /// `if len < MAX_NPC_INSTANCES { push }`，超容时**静默丢弃** ——
    /// 不崩、不报 VUID，画面上只是"少了几个兵"。这就是未结案 #12 那一类失败模式
    /// （"溢出静默丢弃"），而它正是最难查的：没有任何东西告诉你人少了。
    ///
    /// 容量本身够用（`MAX_NPC_INSTANCES = 3072`，128v128 + 尸体实测峰值 2220），
    /// 所以这条**正常情况下永不触发**。它存在的意义是：一旦触发，
    /// 排查的人能立刻定位到"触顶"，而不必去怀疑剔除矩阵 / 模型 / 取景。
    ///
    /// 用闩而不是每次都记：这两个函数每帧都跑，每帧刷日志会把真正有用的行淹掉。
    fn warn_npc_cap_once(&mut self) {
        if self.npc_cap_warned {
            return;
        }
        self.npc_cap_warned = true;
        log::warn!(
            "npc: 段数触顶 MAX_NPC_INSTANCES={} (box={} cyl={} sph={}) => 超出部分被静默丢弃，场景里会少人",
            MAX_NPC_INSTANCES,
            self.npc_box_parts.len(),
            self.npc_cyl_parts.len(),
            self.npc_sph_parts.len()
        );
    }

    pub fn set_npc_visuals(&mut self, visuals: &[NpcVisual]) {
        // 临时埋点（RV3D_NPC_POS=1）：每 120 次调用打一次。放在 `clear()` **之前**，
        // 于是 `self.npc_*_parts` 里还是**上一次循环的最终结果** —— 不必去找函数尾部，
        // 也不会被 clear 掉。目的是回答"NPC 到底有没有被交给渲染器"（第 18 轮的结论：
        // 机位几何已经证明是对的，人却不在画面里 ⇒ 该问题不在取景侧）。
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static TICK: AtomicU32 = AtomicU32::new(0);
            if std::env::var("RV3D_NPC_POS").as_deref() == Ok("1")
                && TICK.fetch_add(1, Ordering::Relaxed) % 120 == 0
            {
                log::info!(
                    "npcvis: 收到 {} 个 NPC；上一帧段数 盒={} 柱={} 球={}（各组上限 {}）",
                    visuals.len(),
                    self.npc_box_parts.len(),
                    self.npc_cyl_parts.len(),
                    self.npc_sph_parts.len(),
                    MAX_NPC_INSTANCES
                );
                // 判据（第 25 轮）：把**前 3 段盒体段实例矩阵的平移分量**打出来，与
                // `visuals[0].pos + 设计偏移` 比对。设计值（见 `soldier_part_matrices`）：
                // 段0/1 = 左右脚，中心 = pos + (∓0.09, +0.05, +0.02)。
                //   相等 ⇒ 矩阵组装正确，问题在槽位/绘制侧；
                //   不等 ⇒ 矩阵组装错了，`soldier_part_matrices` 的输出没落在 pos 上。
                if let Some(v0) = visuals.first() {
                    for (k, part) in self.npc_box_parts.iter().take(3).enumerate() {
                        log::info!(
                            "npcvis: v0.pos=({:.1},{:.1},{:.1}) 盒段[{k}] 平移=({:.2},{:.2},{:.2}) 期望x={:.2}",
                            v0.pos[0],
                            v0.pos[1],
                            v0.pos[2],
                            part.model[12],
                            part.model[13],
                            part.model[14],
                            v0.pos[0] - 0.09
                        );
                    }
                }
            }
        }
        self.npc_box_parts.clear();
        self.npc_cyl_parts.clear();
        self.npc_sph_parts.clear();
        self.soldier_parts.clear();
        let soldier_on = self.soldier_vertex_count > 0;
        for v in visuals {
            // 🪖 士兵 GLB：每个 NPC **一个实例**（根变换 = 位置 + yaw），而不是 18 段。
            // 只有在网格上传成功时才建 —— 否则白算一遍再被 `upload_soldiers` 丢掉。
            if soldier_on && (self.soldier_parts.len() as u32) < MAX_SOLDIER_INSTANCES {
                // 🚶 **步态**（2026-09-13）：GLB 是静态网格，所以用**实例矩阵**补回动作 ——
                // 不接这一段，士兵会僵直地"滑行"，那是我换掉 18 段箱体时引入的倒退
                // （箱体路径原来靠逐段矩阵摆腿）。
                //
                // 只做两件最显眼的事，不值得为此上骨骼：
                //   * **上下起伏**：一步一次，±4cm。人走路时质心确实在上下动，
                //     幅度取小 —— 大了会读成"跳"而不是"走"。
                //   * **前后倾**：与起伏同相，约 ±3°。给"迈步"一个方向感。
                // 相位用 `v.phase`（main.rs 累积时钟，`moving` 为假时冻结），
                // 所以站住的士兵是静止的，不会原地抖。
                //
                // 开火时再加一次**向后的短促后坐**（枪口冲击把人往后推），
                // 同样用 `phase` 取脉冲，避免引入新的状态量。
                let gait = if v.moving { (v.phase * 2.0).sin() } else { 0.0 };
                let bob = gait * 0.04;
                let lean = gait * 0.05;
                let kick = if v.firing { (v.phase * 18.0).sin().max(0.0) * 0.05 } else { 0.0 };
                let rot = glam::Quat::from_rotation_y(v.yaw)
                    * glam::Quat::from_rotation_x(lean - kick);
                let m = glam::Mat4::from_scale_rotation_translation(
                    glam::Vec3::ONE,
                    rot,
                    glam::Vec3::new(v.pos[0], v.pos[1] + bob, v.pos[2]),
                );
                // 🔴🔴 2026-09-13 定案（第一版整身饱和红的真正原因）：
                //
                // 士兵实例槽 `SOLDIER_INSTANCE_BASE`(83011) **≥ `NPC_SLOT_BASE`**，
                // 于是顶点着色器把它归进 NPC 那条**纯色路径**（`flat_flag = 2`）——
                // 那条路上 **`tint` 就是外观色本身，顶点色是被白化掉的**
                // （见 AGENTS.md 铁律 B：18 段箱体的顶点色全部白化）。
                // 所以第一版照抄 `v.tint` 的结果是"一整块饱和的红"，军服细节全丢；
                // 我随后改成"与白色混三成"也只减轻了饱和度，**因为问题不是强度而是语义**。
                //
                // 道具（槽位 83010，同样落在那段范围里）却渲染正常 —— **靠的是 `tint.w`**：
                // `Shape::Authored` 用 `tint.w = 6.0` 作标记 ⇒ 片元判 `authored`
                // ⇒ 跳过四条程序化表面效果、`tint.rgb` 只作乘数 ⇒ 外观回到顶点色。
                //
                // ⇒ 士兵照打同一个标记，于是它就和道具一样**用 `soldier.glb` 里烘焙的
                // 军服/护甲/皮肤色**渲染。`tint.rgb = 1` 表示"顶点色原样输出"。
                //
                // ⚠️ 代价：**红蓝阵营眼下没有颜色区分**（GLB 只有一套橄榄绿军服）。
                // 要区分就得在 Blender 里出两套顶点色变体（蓝/红臂章或迷彩），
                // 那是资产侧的事，不是这里调 tint 能解决的 —— 调了只会把模型涂成一块色。
                // 🔴🔴 2026-09-13：**`tint.w = 6.0` 是"Authored"标记**（`build.rs:99`：
                // `tint.w ∈ (5.5, 6.5)` ⇒ `flat_flag = 1.25`）⇒ 片元走顶点色路径，
                // `tint.rgb` 只作**乘数**。不接这一条，槽位 83011 ≥ `NPC_SLOT_BASE`
                // 会落进 NPC 的纯色路径，`tint` 被当成颜色本身 ⇒ 士兵变成一整块阵营色。
                //
                // ⚠️ **`tint.rgb` 取"向白靠拢的阵营色"，不是原色**。
                // 我早前试过"原色 × 0.30"却仍是纯红，**那次实验是被污染的** ——
                // 当时 18 段箱体还在同时画，红来自箱体。箱体关掉后才量得准。
                // 取 0.35：橄榄绿军服被染成"偏红/偏蓝的军服"，敌我一眼可辨，
                // 而布料与护甲的明暗层次**保留**（这正是"像人"的前提）。
                //
                // ⇒ 若要把阵营差异做得更硬（臂章/迷彩），正解是在 Blender 里出两套
                // 顶点色变体，而不是继续加 tint 强度 —— 那只会把模型重新涂成一块色。
                let t = v.tint;
                const TEAM_MIX: f32 = 0.35;
                let tint = [
                    1.0 - (1.0 - t[0]) * TEAM_MIX,
                    1.0 - (1.0 - t[1]) * TEAM_MIX,
                    1.0 - (1.0 - t[2]) * TEAM_MIX,
                    6.0,
                ];
                self.soldier_parts.push(InstanceData {
                    model: m.to_cols_array(),
                    tint,
                });
            }
            // 🔴🔴 2026-09-13 定案：**GLB 生效时不再生成 18 段箱体**。
            //
            // 此前两条路**同时在画**，玩家看到的是"GLB 士兵叠在箱体堆上"：
            // 形状大体是对的（所以前几轮我没看出来），但同一个身上有两种颜色。
            // 是**品红探针**把它逼出来的 —— 把 tint 临时改成 `[1,0,1,6]` 后
            // **四肢变品红、躯干仍是红的** ⇒ 红的那部分根本不是我的 draw。
            //
            // 这同时把每个 NPC 的实例数从 **18 降到 1**。
            //
            // ⚠️ `soldier_part_matrices` **不删** —— 它是 `soldier.glb` 缺失/上传失败时的
            // 回退路径（`soldier_on == false`），也是将来做"远距 LOD 用箱体"的现成备选。
            if !soldier_on {
                let (box_parts, cyl_parts, sph_parts) = Self::soldier_part_matrices(
                    v.pos, v.yaw, v.tint, v.phase, v.moving, v.firing,
                );
                for part in box_parts {
                    if (self.npc_box_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_box_parts.push(part);
                    }
                }
                for part in cyl_parts {
                    if (self.npc_cyl_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_cyl_parts.push(part);
                    }
                }
                for part in sph_parts {
                    if (self.npc_sph_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_sph_parts.push(part);
                    }
                }
                if (self.npc_box_parts.len() as u32) >= MAX_NPC_INSTANCES
                    && (self.npc_cyl_parts.len() as u32) >= MAX_NPC_INSTANCES
                    && (self.npc_sph_parts.len() as u32) >= MAX_NPC_INSTANCES
                {
                    self.warn_npc_cap_once();
                    break;
                }
            }
        }
    }

    /// 追加倒地尸体（由 main.rs 传入位置/朝向/阵营）。
    ///
    /// 🪖 2026-09-13：**GLB 生效时改用同一套实例化**（原来走 15 段躺倒姿态）。
    /// 一具尸体 = 一个实例，矩阵把 `soldier.glb` 放倒：绕 X 转 −90°，
    /// **绕原点（脚底）转** ⇒ 脚留在 `pos`、身体沿 +Z 平躺、**自然贴地**
    /// （GLB 的 y∈[0,1.84] 转到 z∈[0,1.84]，正好躺在地上）。
    /// 再抬 0.12m，免得半个身子陷进路面。
    ///
    /// 与活体共用 `soldier_parts` / 同一段实例区 ⇒ 一次 draw 里全画完。
    pub fn set_dead_bodies(&mut self, bodies: &[NpcVisual]) {
        let soldier_on = self.soldier_vertex_count > 0;
        for v in bodies {
            if soldier_on && (self.soldier_parts.len() as u32) < MAX_SOLDIER_INSTANCES {
                let rot = glam::Quat::from_rotation_y(v.yaw)
                    * glam::Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2);
                let m = glam::Mat4::from_scale_rotation_translation(
                    glam::Vec3::ONE,
                    rot,
                    glam::Vec3::new(v.pos[0], v.pos[1] + 0.12, v.pos[2]),
                );
                // 阵营色沿用与活体同一套（含 Authored 标记）
                let t = v.tint;
                const TEAM_MIX: f32 = 0.35;
                self.soldier_parts.push(InstanceData {
                    model: m.to_cols_array(),
                    tint: [
                        1.0 - (1.0 - t[0]) * TEAM_MIX,
                        1.0 - (1.0 - t[1]) * TEAM_MIX,
                        1.0 - (1.0 - t[2]) * TEAM_MIX,
                        6.0,
                    ],
                });
            }
            if !soldier_on {
                let (box_parts, cyl_parts, sph_parts) =
                    Self::dead_part_matrices(v.pos, v.yaw, v.tint);
                for part in box_parts {
                    if (self.npc_box_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_box_parts.push(part);
                    }
                }
                for part in cyl_parts {
                    if (self.npc_cyl_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_cyl_parts.push(part);
                    }
                }
                for part in sph_parts {
                    if (self.npc_sph_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_sph_parts.push(part);
                    }
                }
                if (self.npc_box_parts.len() as u32) >= MAX_NPC_INSTANCES
                    && (self.npc_cyl_parts.len() as u32) >= MAX_NPC_INSTANCES
                    && (self.npc_sph_parts.len() as u32) >= MAX_NPC_INSTANCES
                {
                    self.warn_npc_cap_once();
                    break;
                }
            }
        }
    }

    /// 上传第一人称枪模程序化网格（2026-08-16 高模路线）：顶点已是世界空间
    /// （main.rs 用 view⁻¹ × 锚点烘焙），颜色已含材质×烘焙光照。
    /// 用主管线（深度测试关）以 identity 实例（槽 INSTANCE_COUNT）绘制——
    /// 深度测试关闭 = 枪模恒可见（不再需要 z 覆盖 hack，也不写脏深度）。
    pub fn set_first_person_gun_mesh(
        &mut self,
        verts: &[crate::engine::meshgen::GVertex],
        indices: &[u32],
    ) {
        // 🔴 2026-09-22 复查：先记下"旧枪模的计数"。
        // 扩容失败时旧 buffer 仍完好，只有把计数也恢复成旧值，两者才继续自洽；
        // 否则会出现"旧缓冲 + 新计数" ⇒ 按新计数抓取旧缓冲的索引 = 越界（静默的错误几何）。
        let prev_vcount = self.gun_vertex_count;
        let prev_icount = self.gun_index_count;
        self.gun_vertex_count = verts.len() as u32;
        self.gun_index_count = indices.len() as u32;
        if verts.is_empty() || indices.is_empty() {
            return;
        }
        // 枪模缓冲容量：预分配全局最大（当前最大 verts=63283 / idx=70479 ⇒
        // next_power_of_two = 65536 / 262144）。**只增不减**：切枪永不重建缓冲
        // （重建会 destroy 正在被 GPU 使用的 buffer → NVIDIA 驱动 device lost）。
        //
        // 🔴 2026-09-15 修的正是这句注释与代码不符：旧判据是
        // `need != capacity` 就重建，于是**换成更小的枪也会重建** ——
        // 实测按一下 "2"（AK-12M 63283 顶点 / 容量 65536 → AK-104 11705 顶点 /
        // 需要 32768 ≠ 65536）当场 `vkQueueSubmit` 返回 `VK_ERROR_DEVICE_LOST`，
        // 画面上是"切枪 = 整台设备消失"。
        // ⇒ 判据必须是 `need > capacity`（与 `set_props` 同一写法），
        //   并且真扩容前无条件 `device_wait_idle()`。
        let need_verts = 32768u32.max((verts.len() as u32).next_power_of_two());
        let need_idx = 262_144u32.max((indices.len() as u32).next_power_of_two());
        if need_verts > self.gun_buffer_capacity_verts
            || need_idx > self.gun_buffer_capacity_idx
            || self.gun_mapped.is_null()
            || self.gun_vertex_buffer == vk::Buffer::null()
        {
            // 真扩容（或首次创建）才等：等待发生在帧与帧之间、不在命令缓冲记录期间，安全。
            unsafe {
                let _ = self.device.device_wait_idle();
            }
            // 🔴 2026-09-22 复查（灰色地带修复）：**先建新的，成功了再毁旧的**。
            // 旧写法是「先 destroy 两个旧 buffer，再 `create_host_buffer(..).expect(..)`」：
            //  ① 分配失败（显存碎片 / OOM）直接 panic —— 游戏在切枪瞬间整个进程没了；
            //  ② 更糟的是**即使不 panic，旧句柄也已经毁掉了**（自留悬空句柄，
            //     之后任何一次 destroy/free 都是二次释放）。
            // 现在失败路径只 log::error 并**保留原缓冲**（降级：这一枪不换，其余照常跑）；
            // 成功路径多占一份旧缓冲的显存（几 MB）直到销毁，代价可忽略。
            let v_size = need_verts as u64 * std::mem::size_of::<Vertex>() as u64;
            let i_size = need_idx as u64 * 4; // 索引容量独立按实际索引数
            let (vb, vm) = match self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, v_size)
            {
                Ok(pair) => pair,
                Err(e) => {
                    log::error!("枪模顶点缓冲扩容失败，保留原枪模：{e}");
                    self.gun_vertex_count = prev_vcount;
                    self.gun_index_count = prev_icount;
                    return;
                }
            };
            let (ib, im) = match self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, i_size) {
                Ok(pair) => pair,
                Err(e) => {
                    log::error!("枪模索引缓冲扩容失败，保留原枪模：{e}");
                    unsafe {
                        self.device.destroy_buffer(vb, None);
                        self.device.free_memory(vm, None);
                    }
                    self.gun_vertex_count = prev_vcount;
                    self.gun_index_count = prev_icount;
                    return;
                }
            };
            let mapped = match unsafe {
                self.device
                    .map_memory(vm, 0, v_size, vk::MemoryMapFlags::empty())
            } {
                Ok(p) => p,
                Err(e) => {
                    log::error!("枪模顶点缓冲映射失败，保留原枪模：{e}");
                    unsafe {
                        self.device.destroy_buffer(vb, None);
                        self.device.free_memory(vm, None);
                        self.device.destroy_buffer(ib, None);
                        self.device.free_memory(im, None);
                    }
                    self.gun_vertex_count = prev_vcount;
                    self.gun_index_count = prev_icount;
                    return;
                }
            };
            // 新的三件套齐了，才拆旧的（前面已 `device_wait_idle()`，不会撞在飞帧）
            if self.gun_vertex_buffer != vk::Buffer::null() {
                unsafe { self.device.destroy_buffer(self.gun_vertex_buffer, None) };
            }
            if self.gun_vertex_buffer_memory != vk::DeviceMemory::null() {
                unsafe { self.device.free_memory(self.gun_vertex_buffer_memory, None) };
            }
            if self.gun_index_buffer != vk::Buffer::null() {
                unsafe { self.device.destroy_buffer(self.gun_index_buffer, None) };
            }
            if self.gun_index_buffer_memory != vk::DeviceMemory::null() {
                unsafe { self.device.free_memory(self.gun_index_buffer_memory, None) };
            }
            self.gun_vertex_buffer = vb;
            self.gun_vertex_buffer_memory = vm;
            self.gun_index_buffer = ib;
            self.gun_index_buffer_memory = im;
            self.gun_mapped = mapped;
            self.gun_buffer_capacity_verts = need_verts;
            self.gun_buffer_capacity_idx = need_idx;
        }

        // 写入顶点（GVertex → Vertex: pos/color/uv；color 已含烘焙光照）
        let vptr = self.gun_mapped as *mut Vertex;
        for (i, v) in verts.iter().enumerate() {
            unsafe {
                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    color: v.color,
                    uv: v.uv,
                };
            }
        }
        // 2026-08-28 终极可见性修复：unmap → remap（host-coherent 亦可能被驱动缓存延迟可见）
        //
        // 🔴 2026-09-22 复查补（灰色地带）：这里原来是 `.expect("枪模顶点缓冲重映射失败")` ——
        // 映射失败 = 切枪瞬间 panic。更要紧的是失败时 `gun_mapped` 会**停在悬空指针上**：
        // unmap 已经执行，旧指针指向已解除映射的内存 ⇒ 下一枪的顶点写入是野地址写。
        // 现在改成与其它失败路径同款：报错 + 指针置空（入口判据 `gun_mapped.is_null()`
        // 会让**下一枪**走重建分支、重新上传并恢复）。
        // ⚠️ 计数**不回退**：新顶点数据此刻已经写进新缓冲了，回退计数反而会让 draw
        // 按旧数量去抓新缓冲（也就是 5227 行注释里那种"新缓冲 + 旧计数"的错配）。
        unsafe {
            self.device.unmap_memory(self.gun_vertex_buffer_memory);
            match self.device.map_memory(
                self.gun_vertex_buffer_memory,
                0,
                self.gun_buffer_capacity_verts as u64 * std::mem::size_of::<Vertex>() as u64,
                vk::MemoryMapFlags::empty(),
            ) {
                Ok(p) => self.gun_mapped = p,
                Err(e) => {
                    log::error!(
                        "枪模顶点缓冲重映射失败：本枪仍按已写入的数据绘制，指针置空，下一枪重建: {e}"
                    );
                    self.gun_mapped = std::ptr::null_mut();
                }
            }
        }
        // 索引上传（独立映射窗口，用一次性的暂存：直接再 map 索引内存）
        unsafe {
            let iptr = match self.device.map_memory(
                self.gun_index_buffer_memory,
                0,
                self.gun_index_count as u64 * 4,
                vk::MemoryMapFlags::empty(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    // 索引没写进去 ⇒ 缓冲里是**上一把枪的索引**（或未初始化数据）。
                    // 按新计数抓取 = 画错几何，最坏是越界索引 ⇒ 设备消失。
                    // ⇒ 本枪不画（计数归零），并把顶点指针置空，让下一枪整体重建后重传两件套。
                    log::error!("枪模索引缓冲映射失败：本枪不画，下一枪重建: {e}");
                    self.gun_index_count = 0;
                    self.gun_mapped = std::ptr::null_mut();
                    return;
                }
            };
            std::ptr::copy_nonoverlapping(
                indices.as_ptr() as *const u8,
                iptr as *mut u8,
                self.gun_index_count as usize * 4,
            );
            self.device.unmap_memory(self.gun_index_buffer_memory);
        }
    }

    /// 上传 GLB 道具：把摆放列表在 CPU 上烘成一份静态几何，再传上 GPU。
    ///
    /// 只在**地图重载**时调用（`main.rs` 用 `Game::map_generation()` 判定），不要每帧调：
    /// 一次合并是百万级顶点的拷贝。
    ///
    /// 缓冲**只增不减**，且扩容前无条件 `device_wait_idle()`。这两条都是照着枪模的
    /// 事故写的：2026-08-18 那次"切到小网格武器触发重建"直接 destroy 了正在被 GPU 使用
    /// 的 buffer，NVIDIA 驱动 device lost、画面卡死。地图重载发生在帧与帧之间、不在命令
    /// 缓冲记录期间，所以这里的等待是安全的；缩小容量同样走这条路，因此必须等。
    pub fn set_props(
        &mut self,
        set: &crate::engine::props::PropSet,
        placements: &[crate::engine::props::PropPlacement],
    ) {
        let merged =
            crate::engine::props::merge_binned(set, placements, PROP_BIN_CELL_M, |x, z| {
                terrain_height_at(x, z)
            });
        self.prop_vertex_count = 0;
        self.prop_index_count = 0;
        self.prop_bins.clear();
        // 任何提前返回都先把阴影几何清零：shadow loop 会退回全量，绝不会引用
        // 上一张地图的分桶。
        self.prop_sh_index_count = 0;
        self.prop_sh_bins.clear();
        // 🏢 PT 道具属性表同理随道具一起作废（重建在下面上传成功后进行）。
        // 先静默再销毁：旧表可能正被上一帧的 PT dispatch 读着。
        unsafe {
            let _ = self.device.device_wait_idle();
            if self.prop_attr_buf != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_attr_buf, None);
                self.prop_attr_buf = vk::Buffer::null();
            }
            if self.prop_attr_mem != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_attr_mem, None);
                self.prop_attr_mem = vk::DeviceMemory::null();
            }
        }
        self.prop_attr_tris = 0;
        if merged.verts.is_empty() || merged.indices.is_empty() {
            log::info!("props: 无摆放几何（套件 {} 件 / 摆放 {} 处）", set.len(), placements.len());
            return;
        }
        // RV3D_NO_PROPS=1：完全跳过道具上传（于是 draw 因 index_count==0 自然不发）。
        // 这是性能 A/B 的诊断门，与 RV3D_NO_SHADOW / RV3D_NO_GROUND_TEX 同一套惯例——
        // 优化前先量清"这个东西到底值多少帧"，否则很可能在优化错的对象。
        if std::env::var("RV3D_NO_PROPS").as_deref() == Ok("1") {
            log::info!("props: RV3D_NO_PROPS=1，跳过上传（合并结果 {} 顶点未提交）", merged.verts.len());
            return;
        }
        let need_v = merged.verts.len() as u32;
        let need_i = merged.indices.len() as u32;
        let mapped_ok = self.prop_mapped != std::ptr::null_mut()
            && self.prop_vertex_buffer != vk::Buffer::null();
        if !mapped_ok || need_v > self.prop_capacity_verts || need_i > self.prop_capacity_idx {
            unsafe {
                let _ = self.device.device_wait_idle();
            }
            if self.prop_mapped != std::ptr::null_mut() {
                unsafe { self.device.unmap_memory(self.prop_vertex_memory) };
                self.prop_mapped = std::ptr::null_mut();
            }
            for (buf, mem) in [
                (self.prop_vertex_buffer, self.prop_vertex_memory),
                (self.prop_index_buffer, self.prop_index_memory),
            ] {
                if buf != vk::Buffer::null() {
                    unsafe { self.device.destroy_buffer(buf, None) };
                }
                if mem != vk::DeviceMemory::null() {
                    unsafe { self.device.free_memory(mem, None) };
                }
            }
            // 2 的幂向上取整：地图尺寸只会小幅波动，避免每次重载都重建
            let cap_v = need_v.next_power_of_two().max(65_536);
            let cap_i = need_i.next_power_of_two().max(65_536);
            // 🏢 道具进 BLAS（2026-09-19，用户决策）：PT 的第二个三角形几何**零拷贝**
            //   直接引用这两个缓冲 ⇒ usage 必须叠加 SHADER_DEVICE_ADDRESS +
            //   ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY；顶点色还要给 PT 着色器当
            //   albedo ⇒ VB 再加 STORAGE_BUFFER。主 pass 不受影响（usage 只做加法）。
            let (vb, vm) = match self
                .create_host_buffer(
                    vk::BufferUsageFlags::VERTEX_BUFFER
                        | vk::BufferUsageFlags::STORAGE_BUFFER
                        | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                        | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
                    cap_v as u64 * std::mem::size_of::<Vertex>() as u64,
                )
            {
                Ok(v) => v,
                Err(e) => {
                    log::error!("props: 顶点缓冲创建失败，跳过道具绘制: {e}");
                    return;
                }
            };
            let (ib, im) = match self
                .create_host_buffer(
                    vk::BufferUsageFlags::INDEX_BUFFER
                        | vk::BufferUsageFlags::STORAGE_BUFFER
                        | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                        | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
                    cap_i as u64 * 4,
                )
            {
                Ok(v) => v,
                Err(e) => {
                    log::error!("props: 索引缓冲创建失败，跳过道具绘制: {e}");
                    return;
                }
            };
            self.prop_vertex_buffer = vb;
            self.prop_vertex_memory = vm;
            self.prop_index_buffer = ib;
            self.prop_index_memory = im;
            self.prop_mapped = match unsafe {
                self.device.map_memory(vm, 0,
                    cap_v as u64 * std::mem::size_of::<Vertex>() as u64,
                    vk::MemoryMapFlags::empty())
            } {
                Ok(p) => p,
                Err(e) => {
                    log::error!("props: 顶点缓冲映射失败，跳过道具绘制: {e}");
                    return;
                }
            };
            self.prop_capacity_verts = cap_v;
            self.prop_capacity_idx = cap_i;
            log::info!(
                "props: 缓冲扩容 顶点 {}/{} 索引 {}/{}（{:.1} MB）",
                need_v, cap_v, need_i, cap_i,
                (cap_v as u64 * std::mem::size_of::<Vertex>() as u64
                    + cap_i as u64 * 4) as f64 / 1048576.0
            );
        }

        // [f32;11]（pos/normal/uv/color）→ Vertex（pos/color/uv）。
        // normal 与枪模一样在上传时丢弃：本引擎的着色法线由屏幕空间导数重建，
        // 顶点格式里没有它的槽位。因此**绕序必须正确**，反面的面会直接黑掉而不报错。
        let vptr = self.prop_mapped as *mut Vertex;
        for (i, v) in merged.verts.iter().enumerate() {
            unsafe {
                *vptr.add(i) = Vertex {
                    pos: [v[0], v[1], v[2]],
                    color: [v[8], v[9], v[10]],
                    uv: [v[6], v[7]],
                };
            }
        }
        // 与枪模同样的 unmap→remap：host-coherent 内存也可能被驱动延迟可见
        let v_bytes =
            self.prop_capacity_verts as u64 * std::mem::size_of::<Vertex>() as u64;
        unsafe {
            self.device.unmap_memory(self.prop_vertex_memory);
            match self
                .device
                .map_memory(self.prop_vertex_memory, 0, v_bytes, vk::MemoryMapFlags::empty())
            {
                Ok(p) => self.prop_mapped = p,
                Err(e) => {
                    log::error!("props: 顶点缓冲重映射失败，跳过道具绘制: {e}");
                    self.prop_mapped = std::ptr::null_mut();
                    return;
                }
            }
        }
        unsafe {
            match self.device.map_memory(
                self.prop_index_memory,
                0,
                need_i as u64 * 4,
                vk::MemoryMapFlags::empty(),
            ) {
                Ok(iptr) => {
                    std::ptr::copy_nonoverlapping(
                        merged.indices.as_ptr() as *const u8,
                        iptr as *mut u8,
                        need_i as usize * 4,
                    );
                    self.device.unmap_memory(self.prop_index_memory);
                }
                Err(e) => {
                    log::error!("props: 索引缓冲映射失败，跳过道具绘制: {e}");
                    return;
                }
            }
        }
        self.prop_vertex_count = need_v;
        self.prop_index_count = need_i;
        // 桶数量级只有几十，clone 成本可忽略；存下来供 record_command_buffer 逐桶剔除
        self.prop_bins = merged.bins.clone();
        // 🏢 PT 道具逐三角属性表（2026-09-19）：每三角 2×u32 = 量化面法线（(v·127+127)
        //   每轴 u8）+ 平均顶点色（u8×3）。PT 着色器一次命中读一个 8B u32x2——
        //   device-local，**不再随机读 host-visible 主 VB**（pt3 实测那是 80× 的 PCIe 风暴）。
        //   烘焙本体是纯函数 pt_bake_prop_attrs（判据在 pt_prop_attrs_tests）。
        {
            let attrs = pt_bake_prop_attrs(&merged.verts, &merged.indices);
            let ntri = attrs.len() / 2;
            let mut bytes: Vec<u8> = Vec::with_capacity(attrs.len() * 4);
            for w in &attrs {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
            match self.create_device_local_buffer(
                vk::BufferUsageFlags::STORAGE_BUFFER,
                &bytes,
                "prop-attr",
            ) {
                Ok((b, m)) => {
                    self.prop_attr_buf = b;
                    self.prop_attr_mem = m;
                    self.prop_attr_tris = ntri as u32;
                }
                Err(e) => {
                    // 属性表失败 ⇒ 道具不进 BLAS（build_pt_as 以 prop_attr_tris 为准），
                    // PT 退回盒体原型——退化方向是"少几何"，不是越界读
                    log::error!("props/PT: 属性表上传失败，道具不进 BLAS: {e}");
                }
            }
        }
        log::info!(
            "props: 上传完成 顶点 {} / 三角 {} / 摆放 {} 处 / 分桶 {} 个（cell={}m），包围盒 x∈[{:.1},{:.1}] y∈[{:.1},{:.1}] z∈[{:.1},{:.1}]",
            need_v, need_i / 3, placements.len(), self.prop_bins.len(), PROP_BIN_CELL_M,
            merged.min[0], merged.max[0], merged.min[1], merged.max[1],
            merged.min[2], merged.max[2]
        );
        self.set_shadow_props(set, placements);
    }

    /// 阴影建筑 LOD 专用几何上传（2026-09-19 专项，PROGRESS §14）。
    /// 整体重建、无持久映射；任何失败都只把 `prop_sh_index_count` 留 0，
    /// shadow loop 自动退回全量道具几何——**退化方向是"多画三角形"，不是缺阴影**。
    fn set_shadow_props(
        &mut self,
        set: &crate::engine::props::PropSet,
        placements: &[crate::engine::props::PropPlacement],
    ) {
        if !self.shadow_lod || std::env::var("RV3D_NO_PROPS").as_deref() == Ok("1") {
            return;
        }
        let merged = crate::engine::props::merge_shadow_binned(
            set,
            placements,
            PROP_BIN_CELL_M,
            |x, z| terrain_height_at(x, z),
        );
        if merged.verts.is_empty() || merged.indices.is_empty() {
            return;
        }
        let need_v = merged.verts.len() as u32;
        let need_i = merged.indices.len() as u32;
        let vsize = need_v as u64 * std::mem::size_of::<Vertex>() as u64;
        let isz = need_i as u64 * 4;
        unsafe {
            // 与主缓冲同一套安全规矩：旧缓冲可能正被 GPU 引用，先等空闲再销毁
            let _ = self.device.device_wait_idle();
            for (buf, mem) in [
                (self.prop_sh_vertex_buffer, self.prop_sh_vertex_memory),
                (self.prop_sh_index_buffer, self.prop_sh_index_memory),
            ] {
                if buf != vk::Buffer::null() {
                    self.device.destroy_buffer(buf, None);
                }
                if mem != vk::DeviceMemory::null() {
                    self.device.free_memory(mem, None);
                }
            }
            self.prop_sh_vertex_buffer = vk::Buffer::null();
            self.prop_sh_index_buffer = vk::Buffer::null();
        }
        let (vb, vm) = match self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, vsize) {
            Ok(v) => v,
            Err(e) => {
                log::error!("props/shadow: 顶点缓冲创建失败，阴影退回全量几何: {e}");
                return;
            }
        };
        let (ib, im) = match self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, isz) {
            Ok(v) => v,
            Err(e) => {
                unsafe {
                    self.device.destroy_buffer(vb, None);
                    self.device.free_memory(vm, None);
                }
                log::error!("props/shadow: 索引缓冲创建失败，阴影退回全量几何: {e}");
                return;
            }
        };
        let mut ok = true;
        unsafe {
            match self.device.map_memory(vm, 0, vsize, vk::MemoryMapFlags::empty()) {
                Ok(ptr) => {
                    let dst = ptr as *mut Vertex;
                    for (i, v) in merged.verts.iter().enumerate() {
                        *dst.add(i) = Vertex {
                            pos: [v[0], v[1], v[2]],
                            color: [v[8], v[9], v[10]],
                            uv: [v[6], v[7]],
                        };
                    }
                    self.device.unmap_memory(vm);
                }
                Err(e) => {
                    log::error!("props/shadow: 顶点映射失败: {e}");
                    ok = false;
                }
            }
            if ok {
                match self.device.map_memory(im, 0, isz, vk::MemoryMapFlags::empty()) {
                    Ok(ptr) => {
                        std::ptr::copy_nonoverlapping(
                            merged.indices.as_ptr() as *const u8,
                            ptr as *mut u8,
                            merged.indices.len() * 4,
                        );
                        self.device.unmap_memory(im);
                    }
                    Err(e) => {
                        log::error!("props/shadow: 索引映射失败: {e}");
                        ok = false;
                    }
                }
            }
            if !ok {
                self.device.destroy_buffer(vb, None);
                self.device.destroy_buffer(ib, None);
                self.device.free_memory(vm, None);
                self.device.free_memory(im, None);
                return;
            }
        }
        self.prop_sh_vertex_buffer = vb;
        self.prop_sh_vertex_memory = vm;
        self.prop_sh_index_buffer = ib;
        self.prop_sh_index_memory = im;
        self.prop_sh_index_count = need_i;
        self.prop_sh_bins = merged.bins.clone();
        log::info!(
            "props/shadow: 建筑盒壳几何 顶点 {} / 三角 {} / 分桶 {} 个",
            need_v,
            need_i / 3,
            self.prop_sh_bins.len()
        );
    }

    /// 构建路径追踪加速结构：盒体场景 → BLAS + TLAS（2026-08-29 阶段2）
    /// PT 实时 v2（2026-08-29 常驻化）：首帧构建 AS/管线/图像，后帧只 dispatch+blit
    /// 启动时构建 PT 常驻资源（2026-08-29：与 run_pt_view 同时空——已验证可跑！）
    pub fn init_pt_resident(&mut self, w: u32, h: u32) -> Result<(), String> {
        if self.pt_resident.is_some() {
            return Ok(());
        }
        let boxes = vec![
            crate::engine::ray_tracer::PtBox { center: [0.0, -0.5, 0.0], half: [50.0, 0.5, 50.0], material: 0 },
            crate::engine::ray_tracer::PtBox { center: [1.0, 1.0, 0.0], half: [2.0, 2.0, 1.0], material: 1 },
            crate::engine::ray_tracer::PtBox { center: [-4.0, 1.5, -2.0], half: [1.5, 1.5, 1.5], material: 2 },
            crate::engine::ray_tracer::PtBox { center: [0.5, 1.0, 5.0], half: [0.8, 0.8, 0.8], material: 3 },
        ];
        let assets = self.build_pt_as(&boxes)?;
        let vs_module = self.create_shader_module(&crate::shaders::PT_FRAME_SPV.to_vec()).map_err(|e| format!("PT m: {e}"))?;
        let as_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(0).descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let img_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(1).descriptor_type(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let mat_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(2).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let acc_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(3).descriptor_type(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        // 🏢 binding 4 = 道具逐三角属性表（device-local，2×u32/三角）——道具进 BLAS 专项
        let propv_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(4).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let set_bindings = [as_layout, img_layout, mat_layout, acc_layout, propv_layout];
        let set_create = vk::DescriptorSetLayoutCreateInfo::default().bindings(&set_bindings);
        let sl = unsafe { self.device.create_descriptor_set_layout(&set_create, None) }.map_err(|e| format!("PT sl: {e}"))?;
        let pipe_layouts = [sl];
        // push constants：7×vec4 = 112B（pt_panorama.glsl 的 PC 块 a..g）
        let pc_ranges = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(112)];
        let pipe_create = vk::PipelineLayoutCreateInfo::default().set_layouts(&pipe_layouts).push_constant_ranges(&pc_ranges);
        let pl = unsafe { self.device.create_pipeline_layout(&pipe_create, None) }.map_err(|e| format!("PT pl: {e}"))?;
        let stage_info = vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::COMPUTE).module(vs_module).name(c"main");
        let compute_info = vk::ComputePipelineCreateInfo::default().stage(stage_info).layout(pl);
        let pipelines = unsafe { self.device.create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None).map_err(|e| format!("PT pipe {:?}", e.1))? };
        let pipeline = pipelines[0];
        // 🔴 **图像格式必须与 GLSL 里声明的 `rgba8` 逐位一致**（2026-09-15 修正）。
        //
        // 原来这里是 `B8G8R8A8_UNORM`，而 `assets/rt/pt_panorama.glsl` 写的是
        // `layout(set=0, binding=1, rgba8) uniform writeonly image2D OutImg;` ——
        // 两者**兼容但不相等**，于是验证层（`RV3D_VALIDATION=1`）报：
        //
        // ```text
        // vkCmdDispatch(): the storage image descriptor [... variable "OutImg"] is accessed by a
        // OpTypeImage that has a Format operand Rgba8 (VK_FORMAT_R8G8B8A8_UNORM) which doesn't match
        // the VkImageView format (VK_FORMAT_B8G8R8A8_UNORM). Any loads or stores with the variable
        // will produce undefined values to the whole image (not just the texel being accessed).
        // While the formats are compatible, Storage Images must exactly match.
        // ```
        //
        // ⇒ 一句话：**PT 一直在往这张图里写"未定义值"**，不崩、不报错、只是画面发灰发脏 ——
        // 这正是本项目一直在防的那一类「静默 UB」（同教训 15 的越界读）。修法是让图像跟着着色器走
        // （而不是改着色器去迁就图像）：blit 到 B8G8R8A8_SRGB 交换链时驱动会做通道映射，
        // 两者属于同一 format compatibility class，颜色不会错位。
        let pt_img_format = vk::Format::R8G8B8A8_UNORM;
        // 明确查一次：STORAGE_IMAGE 对具体格式是**可选**能力，不支持就大声失败，
        // 不要留下一张"能创建但写不进"的图（那又会退回静默 UB）
        let pt_fmt_props = unsafe {
            self.instance
                .get_physical_device_format_properties(self.physical_device, pt_img_format)
        };
        if !pt_fmt_props
            .optimal_tiling_features
            .contains(vk::FormatFeatureFlags::STORAGE_IMAGE)
        {
            return Err(format!(
                "设备不支持 {pt_img_format:?} 的 STORAGE_IMAGE —— PT 需要它来匹配 GLSL 的 rgba8"
            ));
        }
        let img_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D).format(pt_img_format)
            .extent(vk::Extent3D { width: w, height: h, depth: 1 }).mip_levels(1).array_layers(1).samples(vk::SampleCountFlags::TYPE_1)
            .usage(vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE).initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { self.device.create_image(&img_info, None) }.map_err(|e| format!("PT i: {e}"))?;
        let img_reqs = unsafe { self.device.get_image_memory_requirements(image) };
        let img_type = self.pick_memory_type(img_reqs, true).map_err(|e| format!("PT mt: {e}"))?;
        let img_alloc = vk::MemoryAllocateInfo::default().allocation_size(img_reqs.size).memory_type_index(img_type);
        let img_mem = unsafe { self.device.allocate_memory(&img_alloc, None) }.map_err(|e| format!("PT im: {e}"))?;
        unsafe { self.device.bind_image_memory(image, img_mem, 0) }.map_err(|e| format!("PT ib: {e}"))?;
        let img_view_info = vk::ImageViewCreateInfo::default()
            .image(image).view_type(vk::ImageViewType::TYPE_2D).format(pt_img_format)
            .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
        let view = unsafe { self.device.create_image_view(&img_view_info, None) }.map_err(|e| format!("PT iv: {e}"))?;
        // 时域累积图像：RGBA32F（rgb=Σ线性样本，a=已累积 spp）。必须 STORAGE 且常驻，
        // 每帧只累加不丢弃 => 布局转换只在创建时做一次，逐帧 barrier 用 GENERAL->GENERAL。
        let acc_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D).format(vk::Format::R32G32B32A32_SFLOAT)
            .extent(vk::Extent3D { width: w, height: h, depth: 1 }).mip_levels(1).array_layers(1).samples(vk::SampleCountFlags::TYPE_1)
            .usage(vk::ImageUsageFlags::STORAGE)
            .sharing_mode(vk::SharingMode::EXCLUSIVE).initial_layout(vk::ImageLayout::UNDEFINED);
        let acc_image = unsafe { self.device.create_image(&acc_info, None) }.map_err(|e| format!("PT acc: {e}"))?;
        let acc_reqs = unsafe { self.device.get_image_memory_requirements(acc_image) };
        let acc_type = self.pick_memory_type(acc_reqs, true).map_err(|e| format!("PT acc mt: {e}"))?;
        let acc_alloc = vk::MemoryAllocateInfo::default().allocation_size(acc_reqs.size).memory_type_index(acc_type);
        let acc_mem = unsafe { self.device.allocate_memory(&acc_alloc, None) }.map_err(|e| format!("PT acc mem: {e}"))?;
        unsafe { self.device.bind_image_memory(acc_image, acc_mem, 0) }.map_err(|e| format!("PT acc bind: {e}"))?;
        let acc_view_info = vk::ImageViewCreateInfo::default()
            .image(acc_image).view_type(vk::ImageViewType::TYPE_2D).format(vk::Format::R32G32B32A32_SFLOAT)
            .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
        let acc_view = unsafe { self.device.create_image_view(&acc_view_info, None) }.map_err(|e| format!("PT acc view: {e}"))?;
        let pool_sizes = [
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).descriptor_count(1),
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(2),
            // STORAGE_BUFFER ×2：binding 2（盒材质）+ binding 4（道具逐三角属性表）
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(2),
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&pool_sizes);
        let pool = unsafe { self.device.create_descriptor_pool(&pool_info, None) }.map_err(|e| format!("PT dp: {e}"))?;
        let dset_layouts = [sl];
        let dset_alloc = vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&dset_layouts);
        let dset = unsafe { self.device.allocate_descriptor_sets(&dset_alloc) }.map_err(|e| format!("PT ds: {e}"))?[0];
        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
            p_next: std::ptr::null(), acceleration_structure_count: 1,
            p_acceleration_structures: std::slice::from_ref(&assets.tlas).as_ptr(),
            _marker: std::marker::PhantomData,
        };
        let img_info_desc = vk::DescriptorImageInfo { sampler: vk::Sampler::null(), image_view: view, image_layout: vk::ImageLayout::GENERAL };
        let acc_info_desc = vk::DescriptorImageInfo { sampler: vk::Sampler::null(), image_view: acc_view, image_layout: vk::ImageLayout::GENERAL };
        let mat_buf_info = vk::DescriptorBufferInfo {
            buffer: assets.mat_buf,
            offset: 0,
            range: (crate::engine::ray_tracer::PT_MAX_BOXES * 16) as u64,
        };
        // 🏢 binding 4 = 道具逐三角属性表（device-local）；表未就绪时占位 verts_buf——
        // 那时 BLAS 没有道具几何，着色器道具分支按几何索引必然不可达
        let propv_buf_info = vk::DescriptorBufferInfo {
            buffer: if self.prop_attr_tris > 0 && self.prop_attr_buf != vk::Buffer::null() {
                self.prop_attr_buf
            } else {
                assets.verts_buf
            },
            offset: 0,
            range: vk::WHOLE_SIZE,
        };
        let writes = [
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: &accel_write as *const _ as *const std::ffi::c_void,
                dst_set: dset, dst_binding: 0, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
                p_image_info: std::ptr::null(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 1, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_IMAGE,
                p_image_info: std::slice::from_ref(&img_info_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 2, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(), p_buffer_info: std::slice::from_ref(&mat_buf_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 3, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_IMAGE,
                p_image_info: std::slice::from_ref(&acc_info_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 4, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(), p_buffer_info: std::slice::from_ref(&propv_buf_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        // AS 一次性构建 + 等待（与 run_pt_view 同款——已验证路径！）
        let alloc = vk::CommandBufferAllocateInfo::default().command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
        let cb = unsafe { self.device.allocate_command_buffers(&alloc) }.map_err(|e| format!("PT cb: {e}"))?[0];
        unsafe {
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))
                .map_err(|e| format!("PT cb begin: {e}"))?;
            // 累积图像只做一次 UNDEFINED->GENERAL：之后每帧 barrier 必须是 GENERAL->GENERAL，
            // old_layout 用 UNDEFINED 等于告诉驱动"内容可丢弃" = 累积白做
            let acc_bar = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::NONE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED).new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(acc_image)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &[acc_bar]);
            self.record_pt_build(cb, &assets, boxes.len())?;
            self.device.end_command_buffer(cb).map_err(|e| format!("PT cb end: {e}"))?;
            let cbs = [cb];
            let submit = vk::SubmitInfo::default().command_buffers(&cbs);
            self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("PT sc: {e}"))?;
            self.device.queue_wait_idle(self.graphics_queue).map_err(|e| format!("PT sw: {e}"))?;
            self.device.free_command_buffers(self.command_pool, &[cb]);
        }
        self.pt_resident = Some(Box::new(assets));
        self.pt_img = image;
        self.pt_img_mem = img_mem;
        self.pt_view = view;
        self.pt_pipeline = pipeline;
        self.pt_layout = pl;
        self.pt_setl = sl;
        self.pt_pool = pool;
        self.pt_dset = dset;
        self.pt_module = vs_module;
        self.pt_acc = acc_image;
        self.pt_acc_mem = acc_mem;
        self.pt_acc_view = acc_view;
        self.pt_size = (w, h);
        // RV3D_PT_SPP 覆盖累积目标（默认 256；调参/快速预览可设小值）
        self.pt_spp_target = std::env::var("RV3D_PT_SPP")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| (1..=4096).contains(v))
            .unwrap_or(256);
        self.pt_frame.set(0);
        self.pt_reset.set(true);
        self.pt_view_sig.set(0);
        log::info!("PT-RESIDENT: {}x{} spp 目标 {}（时域累积）", w, h, self.pt_spp_target);
        Ok(())
    }

    pub fn destroy_pt_resident(&mut self) {
        if self.pt_resident.is_none() {
            return;
        }
        unsafe {
            if self.pt_pipeline != vk::Pipeline::null() { self.device.destroy_pipeline(self.pt_pipeline, None); }
            if self.pt_layout != vk::PipelineLayout::null() { self.device.destroy_pipeline_layout(self.pt_layout, None); }
            if self.pt_setl != vk::DescriptorSetLayout::null() { self.device.destroy_descriptor_set_layout(self.pt_setl, None); }
            if self.pt_pool != vk::DescriptorPool::null() { self.device.destroy_descriptor_pool(self.pt_pool, None); }
            if self.pt_module != vk::ShaderModule::null() { self.device.destroy_shader_module(self.pt_module, None); }
            if self.pt_view != vk::ImageView::null() { self.device.destroy_image_view(self.pt_view, None); }
            if self.pt_img != vk::Image::null() { self.device.destroy_image(self.pt_img, None); }
            if self.pt_img_mem != vk::DeviceMemory::null() { self.device.free_memory(self.pt_img_mem, None); }
            if self.pt_acc_view != vk::ImageView::null() { self.device.destroy_image_view(self.pt_acc_view, None); }
            if self.pt_acc != vk::Image::null() { self.device.destroy_image(self.pt_acc, None); }
            if self.pt_acc_mem != vk::DeviceMemory::null() { self.device.free_memory(self.pt_acc_mem, None); }
            if let Some(assets) = self.pt_resident.take() {
                let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
                ext.destroy_acceleration_structure(assets.tlas, None);
                ext.destroy_acceleration_structure(assets.blas, None);
                self.device.destroy_buffer(assets.verts_buf, None);
                self.device.free_memory(assets.verts_mem, None);
                self.device.destroy_buffer(assets.idx_buf, None);
                self.device.free_memory(assets.idx_mem, None);
                self.device.destroy_buffer(assets.inst_buf, None);
                self.device.free_memory(assets.inst_mem, None);
                self.device.destroy_buffer(assets.mat_buf, None);
                self.device.free_memory(assets.mat_mem, None);
                self.device.destroy_buffer(assets.scratch_buf, None);
                self.device.free_memory(assets.scratch_mem, None);
                self.device.destroy_buffer(assets.tlas_buf, None);
                self.device.free_memory(assets.tlas_mem, None);
                self.device.destroy_buffer(assets.blas_buf, None);
                self.device.free_memory(assets.blas_mem, None);
            }
        }
        self.pt_pipeline = vk::Pipeline::null();
        self.pt_layout = vk::PipelineLayout::null();
        self.pt_setl = vk::DescriptorSetLayout::null();
        self.pt_pool = vk::DescriptorPool::null();
        self.pt_module = vk::ShaderModule::null();
        self.pt_acc = vk::Image::null();
        self.pt_acc_mem = vk::DeviceMemory::null();
        self.pt_acc_view = vk::ImageView::null();
        self.pt_frame.set(0);
        self.pt_reset.set(true);
        self.pt_view_sig.set(0);
        self.pt_view = vk::ImageView::null();
        self.pt_img = vk::Image::null();
        self.pt_img_mem = vk::DeviceMemory::null();
    }

    pub fn build_pt_as(
        &mut self,
        boxes: &[crate::engine::ray_tracer::PtBox],
    ) -> Result<crate::engine::ray_tracer::PtAssets, String> {
        use crate::engine::ray_tracer::PT_MAX_BOXES;
        let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
        let n = boxes.len().min(PT_MAX_BOXES);
        // 🔴 2026-09-14：**把静默截断变成可诊断的一次告警**（与 `warn_npc_cap_once` 同一形态）。
        //
        // 上面那行 `.min()` 超出容量时什么都不说 —— 后果是"PT 画面里少了几栋楼"。
        // 而 PT 是低 spp 的噪点图，**少几个盒子肉眼根本看不出来**：
        // 未结案 #10 记的实测值就是 `marker=547 > PT_MAX_BOXES=512`，每次丢 35 个。
        //
        // 用闩而不是每次都记：这个函数在**场景重建**时调用，而重建由相机位移触发
        // （`signature()` 量化到 ~0.5m），移动时一秒能重建好几次 ⇒ 会刷屏，
        // 把 PT 那些真正有用的行淹掉（教训 26 的反面：噪声会训练人忽略日志）。
        if boxes.len() > PT_MAX_BOXES && !self.pt_box_cap_warned {
            self.pt_box_cap_warned = true;
            log::warn!(
                "PT: 盒数 {} 超过 PT_MAX_BOXES={} => 超出部分被静默丢弃，PT 画面里会少几何。\
                 两条出路：提高该常量（BLAS 按容量分配 ⇒ 显存同比上涨），\
                 或在 CPU 侧按视锥裁剪后再传进来。",
                boxes.len(),
                PT_MAX_BOXES
            );
        }
        // 顶点/索引/材质缓冲一次性按 PT_MAX_BOXES 分配（换场景只重写内容，句柄不动）
        let vb_len = PT_MAX_BOXES * 24 * 32;
        let ib_len = PT_MAX_BOXES * 36 * 4;
        let mb_len = PT_MAX_BOXES * 16;
        let (vbuf, vmem) = self
            .create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR, vb_len as u64)
            .map_err(|e| format!("PT 顶点缓冲: {e}"))?;
        let (ibuf, imem) = self
            .create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR, ib_len as u64)
            .map_err(|e| format!("PT 索引缓冲: {e}"))?;
        let (mbuf, mmem) = self
            .create_host_buffer(vk::BufferUsageFlags::STORAGE_BUFFER, mb_len as u64)
            .map_err(|e| format!("PT 材质缓冲: {e}"))?;
        let albedos: Vec<[f32; 3]> = boxes.iter().take(n).map(|b| pt_albedo_of(b)).collect();
        let mut assets = crate::engine::ray_tracer::PtAssets {
            tlas: vk::AccelerationStructureKHR::null(),
            blas: vk::AccelerationStructureKHR::null(),
            tlas_buf: vk::Buffer::null(),
            tlas_mem: vk::DeviceMemory::null(),
            blas_buf: vk::Buffer::null(),
            blas_mem: vk::DeviceMemory::null(),
            verts_buf: vbuf,
            verts_mem: vmem,
            idx_buf: ibuf,
            idx_mem: imem,
            inst_buf: vk::Buffer::null(),
            inst_mem: vk::DeviceMemory::null(),
            mat_buf: mbuf,
            mat_mem: mmem,
            scratch_buf: vk::Buffer::null(),
            scratch_mem: vk::DeviceMemory::null(),
            scratch_blas: 0,
            prop_tris: 0,
        };
        self.pt_fill_geom(&mut assets, &boxes[..n], &albedos)?;
        // 🏢 道具进 BLAS（2026-09-19，用户决策）：第二个三角形几何**零拷贝**引用道具主
        //   VB/IB——但**只在 BLAS 构建期被驱动读取**（烘进 BVH）。着色器命中时绝不读它们：
        //   那是 HOST_VISIBLE 内存，每命中随机读 = PCIe 风暴（pt3 实测 126fps→1.5fps）。
        //   道具的法线/颜色走 binding 4 的 device-local 逐三角属性表（set_props 构建）。
        //   分流按 rayQueryGetIntersectionGeometryIndexEXT（0=盒、1=道具）：ray query 的
        //   图元索引是**几何内局部**编号，不跨几何连续（pt3 灰树冠事故证伪了旧
        //   "hitPrim 全局连续 + pc.g.x 边界"假设——道具最前 21480 三角被当成盒查 boxMats）。
        // 🏢 道具几何只有在**属性表就绪**时才进 BLAS：着色器道具路径读的就是这张表，
        //   没有它分流就无意义（属性表失败时 set_props 已回退为不进）。
        let prop_tris = if self.prop_attr_tris > 0
            && self.prop_attr_tris * 3 == self.prop_index_count
            && self.prop_vertex_buffer != vk::Buffer::null()
            && self.prop_index_buffer != vk::Buffer::null()
        {
            self.prop_attr_tris
        } else {
            0
        };
        assets.prop_tris = prop_tris;
        let vaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.verts_buf); self.device.get_buffer_device_address(&i) };
        let iaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.idx_buf); self.device.get_buffer_device_address(&i) };
        let mut tri = vk::AccelerationStructureGeometryTrianglesDataKHR::default();
        tri.vertex_format = vk::Format::R32G32B32_SFLOAT;
        tri.max_vertex = (PT_MAX_BOXES * 24 - 1) as u32;
        tri.vertex_data = vk::DeviceOrHostAddressConstKHR { device_address: vaddr };
        tri.vertex_stride = 32;
        tri.index_type = vk::IndexType::UINT32;
        tri.index_data = vk::DeviceOrHostAddressConstKHR { device_address: iaddr };
        tri.transform_data = vk::DeviceOrHostAddressConstKHR { device_address: 0 };
        let mut geo = vk::AccelerationStructureGeometryKHR::default();
        geo.geometry_type = vk::GeometryTypeKHR::TRIANGLES;
        geo.geometry = vk::AccelerationStructureGeometryDataKHR { triangles: tri };
        geo.flags = vk::GeometryFlagsKHR::OPAQUE;
        let mut geos = vec![geo];
        // 尺寸查询按**容量**算盒、按**当前**算道具：盒数波动就地重建够用；道具三角形数
        // 变化会换 `pt_prop_key` ⇒ 走整体重建，不会撑爆这块 AS 存储。
        let mut counts: Vec<u32> = vec![(PT_MAX_BOXES * 12) as u32];
        if prop_tris > 0 {
            let pvaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(self.prop_vertex_buffer); self.device.get_buffer_device_address(&i) };
            let piaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(self.prop_index_buffer); self.device.get_buffer_device_address(&i) };
            let mut ptri = vk::AccelerationStructureGeometryTrianglesDataKHR::default();
            ptri.vertex_format = vk::Format::R32G32B32_SFLOAT;
            ptri.vertex_data = vk::DeviceOrHostAddressConstKHR { device_address: pvaddr };
            ptri.vertex_stride = 32;
            ptri.max_vertex = self.prop_vertex_count.saturating_sub(1);
            ptri.index_type = vk::IndexType::UINT32;
            ptri.index_data = vk::DeviceOrHostAddressConstKHR { device_address: piaddr };
            ptri.transform_data = vk::DeviceOrHostAddressConstKHR { device_address: 0 };
            let mut pgeo = vk::AccelerationStructureGeometryKHR::default();
            pgeo.geometry_type = vk::GeometryTypeKHR::TRIANGLES;
            pgeo.geometry = vk::AccelerationStructureGeometryDataKHR { triangles: ptri };
            pgeo.flags = vk::GeometryFlagsKHR::OPAQUE;
            geos.push(pgeo);
            counts.push(prop_tris);
        }
        let mut geom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        geom.ty = vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL;
        geom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        geom.geometry_count = geos.len() as u32;
        geom.p_geometries = geos.as_ptr();
        geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            // 尺寸按 PT_MAX_BOXES 容量算（不是当前盒数）：换场景只重建 BLAS，
            // 若按初始 4 盒分配，塞进 512 盒会越界写 AS 缓冲 -> device lost
            ext.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &geom, &counts, &mut size_info);
        }
        let count = size_info.acceleration_structure_size;
        log::info!("PT-BLAS: size={} scratch_build={} prims={}", count, size_info.build_scratch_size, n * 12);
        let (asbuf, asmem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; count as usize], "pt-blas")?;
        let as_info = vk::AccelerationStructureCreateInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
            .size(count)
            .buffer(asbuf);
        let blas = unsafe { ext.create_acceleration_structure(&as_info, None) }
            .map_err(|e| format!("create BLAS: {e}"))?;
        let blas_addr = unsafe {
            let a = vk::AccelerationStructureDeviceAddressInfoKHR::default().acceleration_structure(blas);
            ext.get_acceleration_structure_device_address(&a)
        };
        // TLAS（单实例 identity：整场盒体合并在一个 BLAS 内，实例数与场景规模无关）
        let instance = vk::AccelerationStructureInstanceKHR {
            transform: vk::TransformMatrixKHR { matrix: [1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0] },
            instance_custom_index_and_mask: vk::Packed24_8::new(0u32, 0xFFu8),
            instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(0u32, 0u8),
            acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_handle: blas_addr },
        };
        let inst_bytes: &[u8] = unsafe { std::slice::from_raw_parts(&instance as *const _ as *const u8, std::mem::size_of::<vk::AccelerationStructureInstanceKHR>()) };
        let (inst_buf, inst_mem) = self
            .create_host_buffer(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR, inst_bytes.len() as u64)
            .map_err(|e| format!("PT 实例缓冲: {e}"))?;
        unsafe {
            let ip = self.device.map_memory(inst_mem, 0, inst_bytes.len() as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("map inst: {e}"))?;
            std::ptr::copy_nonoverlapping(inst_bytes.as_ptr(), ip as *mut u8, inst_bytes.len());
            self.device.unmap_memory(inst_mem);
        }
        let inst_addr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(inst_buf); self.device.get_buffer_device_address(&i) };

        // TLAS 几何（实例）
        let mut inst_geo_data = vk::AccelerationStructureGeometryInstancesDataKHR::default();
        inst_geo_data.array_of_pointers = vk::FALSE;
        inst_geo_data.data = vk::DeviceOrHostAddressConstKHR { device_address: inst_addr };
        let mut tgeo = vk::AccelerationStructureGeometryKHR::default();
        tgeo.geometry_type = vk::GeometryTypeKHR::INSTANCES;
        tgeo.geometry = vk::AccelerationStructureGeometryDataKHR { instances: inst_geo_data };
        let mut tgeom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        tgeom.ty = vk::AccelerationStructureTypeKHR::TOP_LEVEL;
        tgeom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        tgeom.geometry_count = 1;
        tgeom.p_geometries = &tgeo;
        tgeom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let mut tsize = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            ext.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &tgeom, &[1], &mut tsize);
        }
        let tcount = tsize.acceleration_structure_size;
        log::info!("PT-TLAS: size={} scratch_build={}", tcount, tsize.build_scratch_size);
        let (tbuf, tmem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; tcount as usize], "pt-tlas")?;
        let tinfo = vk::AccelerationStructureCreateInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .size(tcount)
            .buffer(tbuf);
        let tlas = unsafe { ext.create_acceleration_structure(&tinfo, None) }
            .map_err(|e| format!("create TLAS: {e}"))?;
        // scratch 自有常驻：BLAS 用前段、TLAS 用后段（同地址连用两次构建 = 资源冲突）
        let align = 256u64;
        let b_scr = (size_info.build_scratch_size.max(align) + align - 1) & !(align - 1);
        let t_scr = (tsize.build_scratch_size.max(align) + align - 1) & !(align - 1);
        let (sbuf, smem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; (b_scr + t_scr) as usize], "pt-scratch")?;
        assets.tlas = tlas;
        assets.blas = blas;
        assets.tlas_buf = tbuf;
        assets.tlas_mem = tmem;
        assets.blas_buf = asbuf;
        assets.blas_mem = asmem;
        assets.inst_buf = inst_buf;
        assets.inst_mem = inst_mem;
        assets.scratch_buf = sbuf;
        assets.scratch_mem = smem;
        assets.scratch_blas = b_scr;
        self.pt_box_count = n;
        Ok(assets)
    }

    /// 把盒体几何/索引/材质写进已分配的容量缓冲（句柄不变，换场景只重写内容）
    fn pt_fill_geom(
        &self,
        assets: &crate::engine::ray_tracer::PtAssets,
        boxes: &[crate::engine::ray_tracer::PtBox],
        albedos: &[[f32; 3]],
    ) -> Result<(), String> {
        use crate::engine::ray_tracer::PT_MAX_BOXES;
        let boxidx = crate::engine::ray_tracer::box_indices();
        let mut verts = vec![0f32; PT_MAX_BOXES * 24 * 8];
        let mut idx = vec![0u32; PT_MAX_BOXES * 36];
        let mut mats = vec![0f32; PT_MAX_BOXES * 4];
        for (k, b) in boxes.iter().enumerate().take(PT_MAX_BOXES) {
            let mut v = [0.0f32; 192];
            crate::engine::ray_tracer::box_triangles(b, &mut v);
            verts[k * 192..k * 192 + 192].copy_from_slice(&v);
            let base = (k as u32) * 24;
            // 每盒索引加 base-vertex 偏移（否则所有盒都引用盒 0 顶点）
            for (j, &i) in boxidx.iter().enumerate() {
                idx[k * 36 + j] = i + base;
            }
            let a = albedos.get(k).copied().unwrap_or([0.5; 3]);
            mats[k * 4] = a[0];
            mats[k * 4 + 1] = a[1];
            mats[k * 4 + 2] = a[2];
            mats[k * 4 + 3] = 0.0;
        }
        unsafe {
            let vb = verts.len() * 4;
            let p = self.device.map_memory(assets.verts_mem, 0, vb as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("map v: {e}"))?;
            std::ptr::copy_nonoverlapping(verts.as_ptr() as *const u8, p as *mut u8, vb);
            self.device.unmap_memory(assets.verts_mem);
            let ib = idx.len() * 4;
            let p = self.device.map_memory(assets.idx_mem, 0, ib as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("map i: {e}"))?;
            std::ptr::copy_nonoverlapping(idx.as_ptr() as *const u8, p as *mut u8, ib);
            self.device.unmap_memory(assets.idx_mem);
            let mb = mats.len() * 4;
            let p = self.device.map_memory(assets.mat_mem, 0, mb as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("map m: {e}"))?;
            std::ptr::copy_nonoverlapping(mats.as_ptr() as *const u8, p as *mut u8, mb);
            self.device.unmap_memory(assets.mat_mem);
        }
        Ok(())
    }

    /// PT 场景热替换：重写 BLAS 内容并重建加速结构（关卡加载/据点变色时一次）。
    /// 与光栅化共用同一批 WorldMarker 矩阵 => PT 与画面几何逐米一致。
    pub fn pt_set_scene_markers(&mut self, markers: &[WorldMarker]) -> Result<(), String> {
        use crate::engine::ray_tracer::PT_MAX_BOXES;
        if self.pt_resident.is_none() || !self.pt_live_enabled {
            return Ok(());
        }
        let mut boxes: Vec<crate::engine::ray_tracer::PtBox> =
            Vec::with_capacity(markers.len() + 1);
        let mut albedos: Vec<[f32; 3]> = Vec::with_capacity(markers.len() + 1);
        // 盒 0 = 地面大盒（游戏地形中央压平，PT 用平面盒近似，烘焙参照足够）
        // 🏢 albedo 从旧沙色 [0.34,0.32,0.29] 改成沥青线性基色（与 procedural.rs zone 2
        //   同源）：§15 实测 PT 路面比光栅亮 2.14×，这颗地面盒是主因之一。
        boxes.push(crate::engine::ray_tracer::PtBox {
            center: [0.0, -1.0, 0.0],
            half: [400.0, 1.0, 400.0],
            material: 0,
        });
        albedos.push([0.115, 0.120, 0.128]);
        // 🔴 容量比对挪到 take **之前**（2026-09-19）：take 截断让 build_pt_as 里的告警闩
        //   永远不触发——marker=1789 > 旧容量 1024 静默丢 765 个就是从这里漏出去的
        //   （#10 这一族坑的第三次复发，容量现已提到 2048）。
        if markers.len() + 1 > PT_MAX_BOXES && !self.pt_box_cap_warned {
            self.pt_box_cap_warned = true;
            log::warn!(
                "PT: marker 数 {} + 地面盒超过盒容量 {} ⇒ 超出部分正被 take 截断，请提高 PT_MAX_BOXES",
                markers.len(),
                PT_MAX_BOXES - 1
            );
        }
        for m in markers.iter().take(PT_MAX_BOXES - 1) {
            let c = m.model.w_axis;
            let hx = m.model.x_axis.length() * 0.5;
            let hy = m.model.y_axis.length() * 0.5;
            let hz = m.model.z_axis.length() * 0.5;
            if !(hx > 0.01 && hy > 0.01 && hz > 0.01) {
                continue;
            }
            boxes.push(crate::engine::ray_tracer::PtBox {
                center: [c.x, c.y, c.z],
                half: [hx, hy, hz],
                material: 1,
            });
            albedos.push([m.tint[0], m.tint[1], m.tint[2]]);
        }
        let sig = pt_scene_sig(&boxes);
        // 🏢 道具几何句柄/三角数变了 ⇒ BLAS 的尺寸与引用都变 ⇒ 必须整体重建
        //（就地 rebuild 只重写盒体内容，改不了 AS 大小）。
        let prop_key = (
            ash::vk::Handle::as_raw(self.prop_vertex_buffer),
            ash::vk::Handle::as_raw(self.prop_attr_buf),
            self.prop_index_count,
        );
        let props_changed = prop_key != self.pt_prop_key;
        if sig == self.pt_scene_sig && !props_changed {
            return Ok(());
        }
        self.pt_scene_sig = sig;
        self.pt_prop_key = prop_key;
        let n = boxes.len();
        if props_changed {
            // 顺序：静默 → 建新（双几何尺寸查询+创建+填充）→ 重写描述符 → 构建+静默 → 销毁旧。
            // 帧内顺序（set_props → 本函数 → render）保证新旧之间没有 dispatch 引用旧缓冲。
            unsafe {
                let _ = self.device.device_wait_idle();
            }
            let old = self.pt_resident.take();
            let fresh = match self.build_pt_as(&boxes) {
                Ok(a) => a,
                Err(e) => {
                    self.pt_resident = old;
                    return Err(e);
                }
            };
            let fresh_prop_tris = fresh.prop_tris;
            self.pt_resident = Some(Box::new(fresh));
            self.pt_refresh_dset()?;
            let res = self.pt_scene_rebuild(
                self.pt_resident.as_ref().unwrap(),
                &boxes,
                &albedos,
                n,
            );
            if let Some(o) = old {
                unsafe { self.pt_destroy_assets(&o) };
            }
            res?;
            self.pt_box_count = n;
            self.pt_frame.set(0);
            self.pt_reset.set(true);
            log::info!(
                "PT-SCENE: 道具几何变化 → BLAS 整体重建：盒 {} + 道具三角 {}",
                n,
                fresh_prop_tris
            );
            return Ok(());
        }
        // 取出 assets（避免 &mut self.pt_resident 与随后的 &self 方法调用冲突）
        let assets = match self.pt_resident.take() {
            Some(a) => a,
            None => return Ok(()),
        };
        let res = self.pt_scene_rebuild(&assets, &boxes, &albedos, n);
        self.pt_resident = Some(assets);
        res?;
        self.pt_box_count = n;
        // 场景换了，旧累积全部作废
        self.pt_frame.set(0);
        self.pt_reset.set(true);
        log::info!("PT-SCENE: 盒 {} 个（WorldMarker 同源）", n);
        Ok(())
    }

    /// 重写几何 + 重建加速结构（一次性提交并等队列空闲——关卡加载级别的一次性开销）
    fn pt_scene_rebuild(
        &self,
        assets: &crate::engine::ray_tracer::PtAssets,
        boxes: &[crate::engine::ray_tracer::PtBox],
        albedos: &[[f32; 3]],
        n: usize,
    ) -> Result<(), String> {
        self.pt_fill_geom(assets, boxes, albedos)?;
        unsafe {
            let alloc = vk::CommandBufferAllocateInfo::default().command_pool(self.command_pool)
                .level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
            let cb = self.device.allocate_command_buffers(&alloc).map_err(|e| format!("PT cb: {e}"))?[0];
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))
                .map_err(|e| format!("PT cb begin: {e}"))?;
            self.record_pt_build(cb, assets, n)?;
            self.device.end_command_buffer(cb).map_err(|e| format!("PT cb end: {e}"))?;
            let cbs = [cb];
            let submit = vk::SubmitInfo::default().command_buffers(&cbs);
            self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("PT scene submit: {e}"))?;
            // 必须排空**整设备**在飞工作：上一帧的 PT dispatch 仍在读同一批 TLAS/顶点缓冲，
            // 只等 graphics queue 不够（重写正在被读的 AS 输入 = device lost / TDR）
            self.device.device_wait_idle().map_err(|e| format!("PT scene wait: {e}"))?;
            self.device.free_command_buffers(self.command_pool, &[cb]);
        }
        Ok(())
    }

    /// BLAS 整体重建后重写 PT 常驻描述符集：binding 0（新 TLAS）/ 2（新 mat_buf）/
    /// 4（道具 VB）。**必须在设备静默后调用**（更新在飞中的 set = UB）——调用方
    /// （pt_set_scene_markers 重建分支）已先 device_wait_idle。
    /// binding 1/3 指渲染器自有的输出/累积图像，句柄跨重建不变，不用动。
    fn pt_refresh_dset(&self) -> Result<(), String> {
        use crate::engine::ray_tracer::PT_MAX_BOXES;
        let assets = self.pt_resident.as_ref().ok_or("PT 未常驻")?;
        if self.pt_dset == vk::DescriptorSet::null() {
            return Ok(());
        }
        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
            p_next: std::ptr::null(),
            acceleration_structure_count: 1,
            p_acceleration_structures: std::slice::from_ref(&assets.tlas).as_ptr(),
            _marker: std::marker::PhantomData,
        };
        let mat_info = vk::DescriptorBufferInfo {
            buffer: assets.mat_buf,
            offset: 0,
            range: (PT_MAX_BOXES * 16) as u64,
        };
        let propv_info = vk::DescriptorBufferInfo {
            buffer: if assets.prop_tris > 0 { self.prop_attr_buf } else { assets.verts_buf },
            offset: 0,
            range: vk::WHOLE_SIZE,
        };
        let writes = [
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: &accel_write as *const _ as *const std::ffi::c_void,
                dst_set: self.pt_dset,
                dst_binding: 0,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
                p_image_info: std::ptr::null(),
                p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(),
                _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: self.pt_dset,
                dst_binding: 2,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(),
                p_buffer_info: std::slice::from_ref(&mat_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(),
                _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: self.pt_dset,
                dst_binding: 4,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(),
                p_buffer_info: std::slice::from_ref(&propv_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(),
                _marker: std::marker::PhantomData,
            },
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        Ok(())
    }

    /// 只销毁 PtAssets 内部的 GPU 资源（管线/图像归渲染器所有，本函数不动）。
    /// 供道具几何变化触发的整体重建在**新资源就绪后**释放旧的一份。
    unsafe fn pt_destroy_assets(&self, a: &crate::engine::ray_tracer::PtAssets) {
        let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
        if a.tlas != vk::AccelerationStructureKHR::null() {
            ext.destroy_acceleration_structure(a.tlas, None);
        }
        if a.blas != vk::AccelerationStructureKHR::null() {
            ext.destroy_acceleration_structure(a.blas, None);
        }
        for (buf, mem) in [
            (a.verts_buf, a.verts_mem),
            (a.idx_buf, a.idx_mem),
            (a.inst_buf, a.inst_mem),
            (a.mat_buf, a.mat_mem),
            (a.scratch_buf, a.scratch_mem),
            (a.tlas_buf, a.tlas_mem),
            (a.blas_buf, a.blas_mem),
        ] {
            if buf != vk::Buffer::null() {
                self.device.destroy_buffer(buf, None);
            }
            if mem != vk::DeviceMemory::null() {
                self.device.free_memory(mem, None);
            }
        }
    }

    /// 每帧取景参数（相机 + 太阳 + 曝光）
    pub fn set_pt_params(&mut self, p: crate::engine::ray_tracer::PtParams) {
        self.pt_params = p;
    }

    /// PT 参考帧渲染（2026-08-29 里程碑1/2）：相机射线 + 命中着色 + 图像输出 → PNG
    pub fn run_pt_view(
        &mut self,
        boxes: &[crate::engine::ray_tracer::PtBox],
        size: u32,
    ) -> Result<(), String> {
        let assets = self.build_pt_as(boxes)?;
        let vs_module = self
            .create_shader_module(&crate::shaders::PT_FRAME_SPV.to_vec())
            .map_err(|e| format!("PT_FRAME module: {e}"))?;
        let as_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let img_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let mat_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let acc_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(3)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        // 🏢 binding 4 = 道具逐三角属性表（device-local，2×u32/三角）——道具进 BLAS 专项
        let propv_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(4)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let set_bindings = [as_layout, img_layout, mat_layout, acc_layout, propv_layout];
        let set_create = vk::DescriptorSetLayoutCreateInfo::default().bindings(&set_bindings);
        let set_layout_handle = unsafe { self.device.create_descriptor_set_layout(&set_create, None) }
            .map_err(|e| format!("PT set: {e}"))?;
        let pipe_layouts = [set_layout_handle];
        let pc_ranges = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(112)];
        let pipe_create = vk::PipelineLayoutCreateInfo::default().set_layouts(&pipe_layouts).push_constant_ranges(&pc_ranges);
        let pipe_layout = unsafe { self.device.create_pipeline_layout(&pipe_create, None) }
            .map_err(|e| format!("PT layout: {e}"))?;
        let stage_info = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE).module(vs_module).name(c"main");
        let compute_info = vk::ComputePipelineCreateInfo::default().stage(stage_info).layout(pipe_layout);
        let pipelines = unsafe {
            self.device.create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None)
                .map_err(|e| format!("PT pipe: {:?}", e.1))?
        };
        let compute_pipeline = pipelines[0];
        // 输出存储图像（rgba8, size×size）
        let img_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D { width: size, height: size, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .usage(vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { self.device.create_image(&img_info, None) }
            .map_err(|e| format!("PT img: {e}"))?;
        let img_reqs = unsafe { self.device.get_image_memory_requirements(image) };
        let img_type = self.pick_memory_type(img_reqs, true)?;
        let img_alloc = vk::MemoryAllocateInfo::default().allocation_size(img_reqs.size).memory_type_index(img_type);
        let img_mem = unsafe { self.device.allocate_memory(&img_alloc, None) }
            .map_err(|e| format!("PT img mem: {e}"))?;
        unsafe { self.device.bind_image_memory(image, img_mem, 0) }
            .map_err(|e| format!("PT img bind: {e}"))?;
        let img_view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
        let view = unsafe { self.device.create_image_view(&img_view_info, None) }
            .map_err(|e| format!("PT view: {e}"))?;
        // 累积图像（RGBA32F）：参考帧一次派发多帧 spp，输出收敛结果而非 1 spp 噪声图
        let acc_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R32G32B32A32_SFLOAT)
            .extent(vk::Extent3D { width: size, height: size, depth: 1 })
            .mip_levels(1).array_layers(1).samples(vk::SampleCountFlags::TYPE_1)
            .usage(vk::ImageUsageFlags::STORAGE)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let acc_image = unsafe { self.device.create_image(&acc_info, None) }
            .map_err(|e| format!("PT acc: {e}"))?;
        let acc_reqs = unsafe { self.device.get_image_memory_requirements(acc_image) };
        let acc_type = self.pick_memory_type(acc_reqs, true)?;
        let acc_alloc = vk::MemoryAllocateInfo::default().allocation_size(acc_reqs.size).memory_type_index(acc_type);
        let acc_mem = unsafe { self.device.allocate_memory(&acc_alloc, None) }
            .map_err(|e| format!("PT acc mem: {e}"))?;
        unsafe { self.device.bind_image_memory(acc_image, acc_mem, 0) }
            .map_err(|e| format!("PT acc bind: {e}"))?;
        let acc_view_info = vk::ImageViewCreateInfo::default()
            .image(acc_image).view_type(vk::ImageViewType::TYPE_2D).format(vk::Format::R32G32B32A32_SFLOAT)
            .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
        let acc_view = unsafe { self.device.create_image_view(&acc_view_info, None) }
            .map_err(|e| format!("PT acc view: {e}"))?;
        // 描述符
        let pool_sizes = [
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).descriptor_count(1),
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(2),
            // STORAGE_BUFFER ×2：binding 2（盒材质）+ binding 4（道具逐三角属性表）
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(2),
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&pool_sizes);
        let dpool = unsafe { self.device.create_descriptor_pool(&pool_info, None) }
            .map_err(|e| format!("PT pool: {e}"))?;
        let dset_layouts = [set_layout_handle];
        let dset_alloc = vk::DescriptorSetAllocateInfo::default().descriptor_pool(dpool).set_layouts(&dset_layouts);
        let dset = unsafe { self.device.allocate_descriptor_sets(&dset_alloc) }
            .map_err(|e| format!("PT dset: {e}"))?[0];
        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
            p_next: std::ptr::null(),
            acceleration_structure_count: 1,
            p_acceleration_structures: std::slice::from_ref(&assets.tlas).as_ptr(),
            _marker: std::marker::PhantomData,
        };
        let img_info_desc = vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: view,
            image_layout: vk::ImageLayout::GENERAL,
        };
        let acc_info_desc = vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: acc_view,
            image_layout: vk::ImageLayout::GENERAL,
        };
        let mat_buf_info = vk::DescriptorBufferInfo {
            buffer: assets.mat_buf,
            offset: 0,
            range: (crate::engine::ray_tracer::PT_MAX_BOXES * 16) as u64,
        };
        // 🏢 binding 4 = 道具逐三角属性表（device-local）；表未就绪时占位 verts_buf——
        // 那时 BLAS 没有道具几何，着色器道具分支按几何索引必然不可达
        let propv_buf_info = vk::DescriptorBufferInfo {
            buffer: if self.prop_attr_tris > 0 && self.prop_attr_buf != vk::Buffer::null() {
                self.prop_attr_buf
            } else {
                assets.verts_buf
            },
            offset: 0,
            range: vk::WHOLE_SIZE,
        };
        let writes = [
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: &accel_write as *const _ as *const std::ffi::c_void,
                dst_set: dset, dst_binding: 0, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
                p_image_info: std::ptr::null(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 1, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_IMAGE,
                p_image_info: std::slice::from_ref(&img_info_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 2, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(), p_buffer_info: std::slice::from_ref(&mat_buf_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 3, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_IMAGE,
                p_image_info: std::slice::from_ref(&acc_info_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 4, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(), p_buffer_info: std::slice::from_ref(&propv_buf_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        // 命令：AS 构建 + dispatch + 拷贝回读
        let alloc = vk::CommandBufferAllocateInfo::default().command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
        let cb = unsafe { self.device.allocate_command_buffers(&alloc) }.map_err(|e| format!("PT cb: {e}"))?[0];
        let begin_info = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            self.device.begin_command_buffer(cb, &begin_info).map_err(|e| format!("PT cb begin: {e}"))?;
            self.record_pt_build(cb, &assets, boxes.len())?;
            let img_bar = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::NONE)
                .dst_access_mask(vk::AccessFlags::SHADER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &[img_bar]);
            let accel_bar = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR);
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[accel_bar], &[], &[]);
            self.device.cmd_bind_pipeline(cb, vk::PipelineBindPoint::COMPUTE, compute_pipeline);
            self.device.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::COMPUTE, pipe_layout, 0, &[dset], &[]);
            // 累积图像进 GENERAL（一次性；old_layout 用 UNDEFINED 只在首帧合法）
            let acc_bar0 = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::NONE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(acc_image)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &[acc_bar0]);
            // 一次派发多帧：帧索引逐帧推进 => 采样去相关 => 输出收敛参考帧而非 1 spp 噪声图
            let spp = std::env::var("RV3D_PT_SPP")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|v| (1..=4096).contains(v))
                .unwrap_or(64u32);
            let self_dep = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE);
            for i in 0..spp {
                let pc = self.pt_params.pack(size, size, i, i == 0, spp, 0.0, (self.pt_box_count * 12) as u32);
                self.device.cmd_push_constants(cb, pipe_layout, vk::ShaderStageFlags::COMPUTE, 0, bytemuck_bytes(&pc));
                self.device.cmd_dispatch(cb, (size + 7) / 8, (size + 7) / 8, 1);
                if i + 1 < spp {
                    // 相邻 dispatch 读写同一累积像素，必须 compute->compute 自依赖 barrier
                    self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[self_dep], &[], &[]);
                }
            }
            log::info!("PT-VIEW: spp={}", spp);
            // 回读缓冲
            let (read_buf, read_mem) = self.create_host_buffer(vk::BufferUsageFlags::TRANSFER_DST, (size * size * 4) as u64)?;
            let img_bar2 = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                .old_layout(vk::ImageLayout::GENERAL)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[img_bar2]);
            let cpy_regions = [vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                .image_extent(vk::Extent3D { width: size, height: size, depth: 1 })];
            self.device.cmd_copy_image_to_buffer(cb, image, vk::ImageLayout::GENERAL, read_buf, &cpy_regions);
            self.device.end_command_buffer(cb).map_err(|e| format!("PT cb end: {e}"))?;
            let cbs = [cb];
            let submit = vk::SubmitInfo::default().command_buffers(&cbs);
            self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("PT submit: {e}"))?;
            // 2026-08-29 修复：AS 构建必须在 dispatch 前完成（wait 保障 TLAS 可见）
            self.device.queue_wait_idle(self.graphics_queue).map_err(|e| format!("PT wait: {e}"))?;
            // 读回 + PNG
            let m = self.device.map_memory(read_mem, 0, (size * size * 4) as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("PT map: {e}"))?;
            let px: Vec<u8> = std::slice::from_raw_parts(m as *const u8, (size * size * 4) as usize).to_vec();
            self.device.unmap_memory(read_mem);
            log::info!("PT-VIEW px: [{},{},{}] [{},{},{}] [{},{},{}]", px[0], px[1], px[2], px[64*4], px[64*4+1], px[64*4+2], px[10*64*4+20*4], px[10*64*4+20*4+1], px[10*64*4+20*4+2]);
            // BMP 落盘（24bit，程序化写出，无依赖）
            {
                let row = size * 3;
                let pad = (4 - row % 4) % 4;
                let data_len = (row + pad) as usize * size as usize;
                let file_len = 54 + data_len;
                let mut bmp = Vec::with_capacity(file_len);
                bmp.extend_from_slice(b"BM");
                bmp.extend_from_slice(&(file_len as u32).to_le_bytes());
                bmp.extend_from_slice(&[0u8; 4]);
                bmp.extend_from_slice(&(54u32).to_le_bytes());
                bmp.extend_from_slice(&(40u32).to_le_bytes());
                bmp.extend_from_slice(&(size as i32).to_le_bytes());
                bmp.extend_from_slice(&(size as i32).to_le_bytes());
                bmp.push(1); bmp.push(24); bmp.push(0); bmp.push(0);
                bmp.extend_from_slice(&[0u8; 24]);
                for y in (0..size).rev() {
                    for x in 0..size {
                        let i = ((y * size + x) * 4) as usize;
                        // 逐像素 3 次 push 会被 clippy::same_item_push 误判成"重复推同一个
                        // 项"，而且这里本来就是"搬一段连续字节"，写成切片拷贝更贴原意。
                        // 字节序与顺序保持完全不变（BMP 那 3 个字节仍是 px 的前三分量）。
                        bmp.extend_from_slice(&px[i..i + 3]);
                    }
                    for _ in 0..pad { bmp.push(0); }
                }
                std::fs::write("screenshots/pt_ref.bmp", &bmp).map_err(|e| format!("PT bmp: {e}"))?;
            }
        }
        // 清理
        unsafe {
            self.device.destroy_pipeline(compute_pipeline, None);
            self.device.destroy_pipeline_layout(pipe_layout, None);
            self.device.destroy_descriptor_set_layout(set_layout_handle, None);
            self.device.destroy_descriptor_pool(dpool, None);
            self.device.destroy_shader_module(vs_module, None);
            self.device.free_command_buffers(self.command_pool, &[cb]);
            self.device.destroy_image_view(view, None);
            self.device.destroy_image(image, None);
            self.device.free_memory(img_mem, None);
            self.device.destroy_image_view(acc_view, None);
            self.device.destroy_image(acc_image, None);
            self.device.free_memory(acc_mem, None);
            let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
            ext.destroy_acceleration_structure(assets.tlas, None);
            ext.destroy_acceleration_structure(assets.blas, None);
        }
        Ok(())
    }

    /// RT 核心 纯求交吞吐基准（2026-08-29）：RT_BENCH_SPV 全遍历 × iterations
    /// 返回 (每秒射线 M, 命中数)
    pub fn run_pt_bench(
        &mut self,
        boxes: &[crate::engine::ray_tracer::PtBox],
        rays: u32,
        iterations: u32,
    ) -> Result<(f64, u32), String> {
        // 1) AS
        let assets = self.build_pt_as(boxes)?;
        // 2) compute 管线：RT_BENCH_SPV（内嵌!）
        let vs_module = self
            .create_shader_module(&crate::shaders::RT_BENCH_SPV.to_vec())
            .map_err(|e| format!("RT_BENCH module: {e}"))?;
        let set_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let hits_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let set_bindings = [set_layout, hits_layout];
        let set_create = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&set_bindings);
        let set_layout_handle = unsafe { self.device.create_descriptor_set_layout(&set_create, None) }
            .map_err(|e| format!("RT set layout: {e}"))?;
        let pipe_layouts = [set_layout_handle];
        let pc_ranges: [vk::PushConstantRange; 0] = [];
        let pipe_create = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&pipe_layouts)
            .push_constant_ranges(&pc_ranges);
        let pipe_layout = unsafe { self.device.create_pipeline_layout(&pipe_create, None) }
            .map_err(|e| format!("RT pipe layout: {e}"))?;
        let stage_info = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(vs_module)
            .name(c"main");
        let compute_info = vk::ComputePipelineCreateInfo::default()
            .stage(stage_info)
            .layout(pipe_layout);
        let pipelines = unsafe {
            self.device.create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None)
                .map_err(|e| format!("RT compute pipeline: {:?}", e.1))?
        };
        let compute_pipeline = pipelines[0];
        // 3) hits 缓冲（N u32，host 可见回读）
        let n = rays as usize;
        let (hits_buf, hits_mem) = self
            .create_host_buffer(vk::BufferUsageFlags::STORAGE_BUFFER, (n * 4) as u64)
            .map_err(|e| format!("hits: {e}"))?;
        let hits_mapped = unsafe {
            self.device
                .map_memory(hits_mem, 0, (n * 4) as u64, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("hits map: {e}"))?
        };
        unsafe {
            std::ptr::write_bytes(hits_mapped, 0, n * 4);
        }
        // 4) 描述符集（accel + hits）
        let dset_pool_info = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .descriptor_count(1);
        let dset_pool_info2 = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1);
        let pool_sizes = [dset_pool_info, dset_pool_info2];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        let dpool = unsafe { self.device.create_descriptor_pool(&pool_info, None) }
            .map_err(|e| format!("RT pool: {e}"))?;
        let dset_layouts = [set_layout_handle];
        let dset_alloc = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(dpool)
            .set_layouts(&dset_layouts);
        let dset = unsafe { self.device.allocate_descriptor_sets(&dset_alloc) }
            .map_err(|e| format!("RT dset: {e}"))?[0];
        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
            p_next: std::ptr::null(),
            acceleration_structure_count: 1,
            p_acceleration_structures: std::slice::from_ref(&assets.tlas).as_ptr(),
            _marker: std::marker::PhantomData,
        };
        let write0 = vk::WriteDescriptorSet {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
            p_next: &accel_write as *const _ as *const std::ffi::c_void,
            dst_set: dset,
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
            p_image_info: std::ptr::null(),
            p_buffer_info: std::ptr::null(),
            p_texel_buffer_view: std::ptr::null(),
            _marker: std::marker::PhantomData,
        };
        let buf_info = vk::DescriptorBufferInfo {
            buffer: hits_buf,
            offset: 0,
            range: (n * 4) as u64,
        };
        let write1 = vk::WriteDescriptorSet {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
            p_next: std::ptr::null(),
            dst_set: dset,
            dst_binding: 1,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
            p_image_info: std::ptr::null(),
            p_buffer_info: std::slice::from_ref(&buf_info).as_ptr(),
            p_texel_buffer_view: std::ptr::null(),
            _marker: std::marker::PhantomData,
        };
        unsafe { self.device.update_descriptor_sets(&[write0, write1], &[]) };
        // 5) 一次性构建命令（AS 构建）
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // 使用帧命令池外的一个独立分配
        let cb = unsafe { self.device.allocate_command_buffers(&alloc) }.map_err(|e| format!("pt cb: {e}"))?[0];
        unsafe {
            let begin_info = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device.begin_command_buffer(cb, &begin_info).map_err(|e| format!("pt cb begin: {e}"))?;
            self.record_pt_build(cb, &assets, boxes.len())?;
            self.device.end_command_buffer(cb).map_err(|e| format!("pt cb end: {e}"))?;
        }
        let cbs = [cb];
        let submit = vk::SubmitInfo::default().command_buffers(&cbs);
        unsafe { self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("pt submit: {e}"))?;
            self.device.queue_wait_idle(self.graphics_queue).map_err(|e| format!("pt wait: {e}"))?;
        }
        // 6) 计时迭代：dispatch × iterations（单独 cmd，等待后计时）
        let t0 = std::time::Instant::now();
        unsafe {
            self.device.reset_command_buffer(cb, vk::CommandBufferResetFlags::empty()).map_err(|e| format!("pt reset: {e}"))?;
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default()).map_err(|e| format!("pt bench begin: {e}"))?;
            self.device.cmd_bind_pipeline(cb, vk::PipelineBindPoint::COMPUTE, compute_pipeline);
            self.device.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::COMPUTE, pipe_layout, 0, &[dset], &[]);
            for _ in 0..iterations {
                self.device.cmd_dispatch(cb, (n as u32 + 63) / 64, 1, 1);
            }
            self.device.end_command_buffer(cb).map_err(|e| format!("pt bench end: {e}"))?;
            let cbs2 = [cb];
            let submit2 = vk::SubmitInfo::default().command_buffers(&cbs2);
            self.device.queue_submit(self.graphics_queue, &[submit2], vk::Fence::null()).map_err(|e| format!("pt bench submit: {e}"))?;
            self.device.queue_wait_idle(self.graphics_queue).map_err(|e| format!("pt bench wait: {e}"))?;
        }
        let elapsed = t0.elapsed().as_secs_f64();
        // 7) 回读命中
        let mut hits = 0u32;
        let hp = hits_mapped as *const u32;
        for i in 0..n {
            hits += unsafe { *hp.add(i) };
        }
        let total_rays = (rays as f64) * (iterations as f64);
        let mrays = total_rays / elapsed / 1_000_000.0;
        // 清理（基准一次性：简单释放）
        unsafe {
            self.device.unmap_memory(hits_mem);
            self.device.destroy_buffer(hits_buf, None);
            self.device.free_memory(hits_mem, None);
            self.device.destroy_pipeline(compute_pipeline, None);
            self.device.destroy_pipeline_layout(pipe_layout, None);
            self.device.destroy_descriptor_set_layout(set_layout_handle, None);
            self.device.destroy_descriptor_pool(dpool, None);
            self.device.destroy_shader_module(vs_module, None);
            self.device.free_command_buffers(self.command_pool, &[cb]);
            let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
            ext.destroy_acceleration_structure(assets.tlas, None);
            ext.destroy_acceleration_structure(assets.blas, None);
        }
        Ok((mrays, hits))
    }

    /// 记录 BLAS/TLAS 构建命令（一次性：命令缓冲执行）
    pub fn record_pt_build(
        &self,
        cmd: vk::CommandBuffer,
        assets: &crate::engine::ray_tracer::PtAssets,
        box_count: usize,
    ) -> Result<(), String> {
        let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
        // scratch 归 PtAssets 所有（旧实现每次 record 都新建 2MB 且从不释放 = 显存泄漏源）；
        // BLAS 用前段、TLAS 用后段，两次构建不再共享同一地址。
        let scratch_base = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.scratch_buf); self.device.get_buffer_device_address(&i) };
        // BLAS 构建（重建）
        let mut b_geom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        b_geom.ty = vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL;
        b_geom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        b_geom.geometry_count = 1;
        // 重建 geometry 引用（顶点/索引地址从缓冲重取）
        let vaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.verts_buf); self.device.get_buffer_device_address(&i) };
        let iaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.idx_buf); self.device.get_buffer_device_address(&i) };
        let mut tri = vk::AccelerationStructureGeometryTrianglesDataKHR::default();
        tri.vertex_format = vk::Format::R32G32B32_SFLOAT;
        tri.max_vertex = (crate::engine::ray_tracer::PT_MAX_BOXES * 24 - 1) as u32;
        tri.vertex_data = vk::DeviceOrHostAddressConstKHR { device_address: vaddr };
        tri.vertex_stride = 32;
        tri.index_type = vk::IndexType::UINT32;
        tri.index_data = vk::DeviceOrHostAddressConstKHR { device_address: iaddr };
        tri.transform_data = vk::DeviceOrHostAddressConstKHR { device_address: 0 };
        let mut b_geo = vk::AccelerationStructureGeometryKHR::default();
        b_geo.geometry_type = vk::GeometryTypeKHR::TRIANGLES;
        b_geo.geometry = vk::AccelerationStructureGeometryDataKHR { triangles: tri };
        b_geo.flags = vk::GeometryFlagsKHR::OPAQUE;
        // 🏢 道具几何与 build_pt_as 创建 BLAS 时同一套引用（pt_prop_key 保证句柄/三角数
        //   一致，见 pt_set_scene_markers 的整体重建分支）
        let mut geos = vec![b_geo];
        let mut ranges = vec![
            vk::AccelerationStructureBuildRangeInfoKHR {
                primitive_count: (box_count * 12) as u32,
                primitive_offset: 0,
                first_vertex: 0,
                transform_offset: 0,
            },
        ];
        if assets.prop_tris > 0 {
            let pvaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(self.prop_vertex_buffer); self.device.get_buffer_device_address(&i) };
            let piaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(self.prop_index_buffer); self.device.get_buffer_device_address(&i) };
            let mut ptri = vk::AccelerationStructureGeometryTrianglesDataKHR::default();
            ptri.vertex_format = vk::Format::R32G32B32_SFLOAT;
            ptri.vertex_data = vk::DeviceOrHostAddressConstKHR { device_address: pvaddr };
            ptri.vertex_stride = 32;
            ptri.max_vertex = self.prop_vertex_count.saturating_sub(1);
            ptri.index_type = vk::IndexType::UINT32;
            ptri.index_data = vk::DeviceOrHostAddressConstKHR { device_address: piaddr };
            ptri.transform_data = vk::DeviceOrHostAddressConstKHR { device_address: 0 };
            let mut pgeo = vk::AccelerationStructureGeometryKHR::default();
            pgeo.geometry_type = vk::GeometryTypeKHR::TRIANGLES;
            pgeo.geometry = vk::AccelerationStructureGeometryDataKHR { triangles: ptri };
            pgeo.flags = vk::GeometryFlagsKHR::OPAQUE;
            geos.push(pgeo);
            ranges.push(vk::AccelerationStructureBuildRangeInfoKHR {
                primitive_count: assets.prop_tris,
                primitive_offset: 0,
                first_vertex: 0,
                transform_offset: 0,
            });
        }
        b_geom.geometry_count = geos.len() as u32;
        b_geom.p_geometries = geos.as_ptr();
        b_geom.dst_acceleration_structure = assets.blas;
        b_geom.scratch_data = vk::DeviceOrHostAddressKHR { device_address: scratch_base };
        b_geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        // TLAS
        let mut t_geom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        t_geom.ty = vk::AccelerationStructureTypeKHR::TOP_LEVEL;
        t_geom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        t_geom.geometry_count = 1;
        let mut inst_geo_data = vk::AccelerationStructureGeometryInstancesDataKHR::default();
        inst_geo_data.array_of_pointers = vk::FALSE;
        let inst_addr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.inst_buf); self.device.get_buffer_device_address(&i) };
        inst_geo_data.data = vk::DeviceOrHostAddressConstKHR { device_address: inst_addr };
        let mut t_geo = vk::AccelerationStructureGeometryKHR::default();
        t_geo.geometry_type = vk::GeometryTypeKHR::INSTANCES;
        t_geo.geometry = vk::AccelerationStructureGeometryDataKHR { instances: inst_geo_data };
        t_geom.p_geometries = &t_geo;
        t_geom.dst_acceleration_structure = assets.tlas;
        t_geom.scratch_data = vk::DeviceOrHostAddressKHR { device_address: scratch_base + assets.scratch_blas };
        t_geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let range_t = vk::AccelerationStructureBuildRangeInfoKHR { primitive_count: 1, primitive_offset: 0, first_vertex: 0, transform_offset: 0 };
        unsafe {
            let rbs: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [ranges.as_slice()];
            ext.cmd_build_acceleration_structures(cmd, &[b_geom], &rbs);
            // BLAS 写完 -> TLAS 读几何/引用其结果，两次构建之间必须有执行依赖
            let bb = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR | vk::AccessFlags::SHADER_READ);
            self.device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::DependencyFlags::empty(),
                &[bb],
                &[],
                &[],
            );
            let rt: [vk::AccelerationStructureBuildRangeInfoKHR; 1] = [range_t];
            let rts: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [&rt];
            ext.cmd_build_acceleration_structures(cmd, &[t_geom], &rts);
        }
        Ok(())
    }
    /// 2026-08-28：第一人称枪的实例模型矩阵 per-frame（bob/后坐走矩阵，顶点静态）
    /// 枪槽 75841 的唯一写者：顶点缓冲 = 视空间静态（仅首次上传），矩阵每帧更新
    pub fn set_first_person_gun_model(&mut self, m: glam::Mat4) {
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return,
        };
        let stride = std::mem::size_of::<InstanceData>();
        unsafe {
            let p = slot.add(GUN_INSTANCE_INDEX as usize * stride);
            // InstanceData { model: [f32; 16], tint: [f32; 4] }
            let model = m.to_cols_array();
            std::ptr::copy_nonoverlapping(model.as_ptr(), p as *mut f32, 16);
        }
    }

    /// 🪖 **上传士兵 GLB 网格**（2026-09-13）。只调用一次（启动时）。
    ///
    /// **与枪模的两处关键差别**：
    /// 1. **按常量容量一次分配、永不重建**（照 `props` 那条纪律）。士兵网格不存在切枪那种
    ///    "容量忽大忽小"的场景，而重建会 destroy 在飞 buffer → NVIDIA device lost。
    /// 2. 顶点来源是 GLB 的 `[f32; 11]`（`pos(3) normal(3) uv(2) color(3)`，见 `assets.rs`），
    ///    这里按 `pos=[0..3] / uv=[6,7] / color=[8..11]` 取 —— **与 `upload_props` 完全同一套
    ///    映射**（本引擎顶点格式 `stride=32, pos/color/uv`，没有法线槽位，法线由屏幕空间
    ///    导数重建，所以 GLB 的法线直接丢弃）。
    pub fn set_soldier_mesh(&mut self, verts: &[[f32; 11]], indices: &[u32]) {
        // 🔴🔴 2026-09-13：**幂等守卫**。调用方在每帧的渲染准备段里调用本函数，
        // 而它每次都会 `create_host_buffer` 出一套新的 GPU 缓冲 ⇒ **每帧泄漏一份显存**，
        // 几分钟就 OOM / device lost。实测日志里同一个 "士兵 GLB 已上传" 一段内出现 3+ 次。
        // 士兵网格与武器不同：它**只在启动时上传一次**，之后永不改变，所以直接早退即可。
        if self.soldier_vertex_count > 0 {
            return;
        }
        if verts.is_empty() || indices.is_empty() {
            log::info!("soldier: 未提供网格，NPC 继续用 18 段箱体");
            return;
        }
        if verts.len() > SOLDIER_MESH_VERTS as usize || indices.len() > SOLDIER_MESH_INDICES as usize {
            log::error!(
                "soldier: 网格超出预留容量（{} > {} 顶点 / {} > {} 索引）—— 必须同步放大 \
                 SOLDIER_MESH_VERTS/SOLDIER_MESH_INDICES，否则写越界（host buffer 不报 VUID）",
                verts.len(), SOLDIER_MESH_VERTS, indices.len(), SOLDIER_MESH_INDICES
            );
            return;
        }
        let v_size = SOLDIER_MESH_VERTS as u64 * std::mem::size_of::<Vertex>() as u64;
        let i_size = SOLDIER_MESH_INDICES as u64 * 4;
        // 失败路径统一收尾：释放**已经建好、但还没存进 `self`** 的 buffer/memory。
        // 直接 `return` 等于永久泄漏这几份显存 —— 没有任何别的引用还找得到它们。
        fn free_pair(device: &ash::Device, b: vk::Buffer, m: vk::DeviceMemory) {
            unsafe {
                if b != vk::Buffer::null() {
                    device.destroy_buffer(b, None);
                }
                if m != vk::DeviceMemory::null() {
                    device.free_memory(m, None);
                }
            }
        }
        let (vb, vm) = match self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, v_size) {
            Ok(x) => x,
            Err(e) => {
                log::error!("soldier: 顶点缓冲创建失败，退回 18 段箱体: {e}");
                return;
            }
        };
        let (ib, im) = match self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, i_size) {
            Ok(x) => x,
            Err(e) => {
                log::error!("soldier: 索引缓冲创建失败，退回 18 段箱体: {e}");
                // 🔴 2026-09-22 复查补：顶点那一对已经建好了，必须先释放再退出。
                free_pair(&self.device, vb, vm);
                return;
            }
        };
        let mapped = match unsafe {
            self.device
                .map_memory(vm, 0, v_size, vk::MemoryMapFlags::empty())
        } {
            Ok(p) => p,
            Err(e) => {
                log::error!("soldier: 顶点缓冲映射失败，退回 18 段箱体: {e}");
                free_pair(&self.device, vb, vm);
                free_pair(&self.device, ib, im);
                return;
            }
        };
        let vptr = mapped as *mut Vertex;
        for (i, v) in verts.iter().enumerate() {
            unsafe {
                *vptr.add(i) = Vertex {
                    pos: [v[0], v[1], v[2]],
                    color: [v[8], v[9], v[10]],
                    uv: [v[6], v[7]],
                };
            }
        }
        // 索引也要 host-visible：单独 map 索引缓冲
        //
        // 🔴 2026-09-22 复查补：这里原来是把 `map_memory` 的结果用 `if let Ok` 吞掉 ——
        // **映射失败被静默吞掉**：索引一个都没写进去，函数却照常往下走，把
        // `soldier_index_count` 设成 `indices.len()` ⇒ draw call 按这个数去读
        // **未初始化的显存**（几何错乱，最坏是越界索引 = 设备消失），且不报任何错。
        // 现在与其它失败路径同款：报错 + 释放已建资源 + 退回 18 段箱体。
        let ip = match unsafe {
            self.device
                .map_memory(im, 0, i_size, vk::MemoryMapFlags::empty())
        } {
            Ok(p) => p,
            Err(e) => {
                log::error!("soldier: 索引缓冲映射失败，退回 18 段箱体: {e}");
                unsafe { self.device.unmap_memory(vm) };
                free_pair(&self.device, vb, vm);
                free_pair(&self.device, ib, im);
                return;
            }
        };
        let iptr = ip as *mut u32;
        for (i, idx) in indices.iter().enumerate() {
            unsafe { *iptr.add(i) = *idx };
        }
        unsafe { self.device.unmap_memory(im) };
        // 与枪模/道具同样的 unmap→remap：host-coherent 内存也可能被驱动延迟可见。
        // 顶点只上传这一次，所以重映射后**不留指针**（枪模留是因为它要反复重写）。
        unsafe {
            self.device.unmap_memory(vm);
            if let Err(e) = self
                .device
                .map_memory(vm, 0, v_size, vk::MemoryMapFlags::empty())
            {
                log::error!("soldier: 顶点缓冲重映射失败，退回 18 段箱体: {e}");
                // vm 刚刚已 unmap（不能重复 unmap），直接释放两对句柄即可。
                free_pair(&self.device, vb, vm);
                free_pair(&self.device, ib, im);
                return;
            }
            self.device.unmap_memory(vm);
        }
        self.soldier_vertex_buffer = vb;
        self.soldier_vertex_buffer_memory = vm;
        self.soldier_index_buffer = ib;
        self.soldier_index_buffer_memory = im;
        self.soldier_vertex_count = verts.len() as u32;
        self.soldier_index_count = indices.len() as u32;
        log::info!(
            "soldier: 士兵 GLB 已上传（{} 顶点 / {} 索引，实例区起点 {}，容量 {}）",
            self.soldier_vertex_count,
            self.soldier_index_count,
            SOLDIER_INSTANCE_BASE,
            MAX_SOLDIER_INSTANCES
        );
    }

    /// 每帧把 `soldier_parts` 写进实例缓冲的士兵区。返回实际写入数。
    ///
    /// **超出容量的部分直接不写**（并由调用方计数），**绝不越界** ——
    /// 越界写实例 storage buffer 的后果是驱动静默返回全零、几何塌成一点（铁律 B）。
    fn upload_soldiers(&mut self) -> u32 {
        self.soldier_drawn = 0;
        if self.soldier_vertex_count == 0 || self.soldier_parts.is_empty() {
            return 0;
        }
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return 0,
        };
        let stride = std::mem::size_of::<InstanceData>();
        let n = self.soldier_parts.len().min(MAX_SOLDIER_INSTANCES as usize);
        for (i, inst) in self.soldier_parts.iter().take(n).enumerate() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    inst as *const InstanceData as *const u8,
                    slot.add((SOLDIER_INSTANCE_BASE as usize + i) * stride),
                    stride,
                );
            }
        }
        self.soldier_drawn = n as u32;
        self.soldier_parts.clear();
        self.soldier_drawn
    }

    /// 每帧上传 NPC 士兵段到实例 buffer 的 NPC_SLOT_BASE 之后区域，
    /// 仿照 upload_markers 按距离分近/远档（不剔除，仅距离分档），返回 (近档, 远档) 计数。
    /// 上传 NPC 三几何区（盒/圆柱/球），每区按距离分近/远档。
    /// 返回 ((盒 near,far),(圆柱 near,far),(球 near,far))。
    fn upload_npcs(
        &mut self,
        cam_pos: glam::Vec3,
    ) -> ((u32, u32), (u32, u32), (u32, u32)) {
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return ((0, 0), (0, 0), (0, 0)),
        };
        let stride = std::mem::size_of::<InstanceData>();
        if self.mesh_enabled {
            // mesh 路径：全量上传（无分档），计数 = 各组长度
            for (i, inst) in self.npc_box_parts.iter().enumerate() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((NPC_SLOT_BASE + i as u32) as usize) * stride),
                        stride,
                    );
                }
            }
            for (i, inst) in self.npc_cyl_parts.iter().enumerate() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((NPC_CYL_SLOT_BASE + i as u32) as usize) * stride),
                        stride,
                    );
                }
            }
            for (i, inst) in self.npc_sph_parts.iter().enumerate() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((NPC_SPH_SLOT_BASE + i as u32) as usize) * stride),
                        stride,
                    );
                }
            }
            return (
                (self.npc_box_parts.len() as u32, 0),
                (self.npc_cyl_parts.len() as u32, 0),
                (self.npc_sph_parts.len() as u32, 0),
            );
        }
        // 近/远档分界距离随画质预设变化（与 marker/实例场同源）
        let near_sq = quality_params(self.quality).instance_lod_distance;
        let near_sq = near_sq * near_sq;
        let cam = glam::Vec3::new(cam_pos.x, cam_pos.y, cam_pos.z);
        let box_zone = Self::upload_zone_rel(&self.npc_box_parts, NPC_SLOT_BASE, near_sq, slot, stride, cam);
        let cyl_zone = Self::upload_zone_rel(&self.npc_cyl_parts, NPC_CYL_SLOT_BASE, near_sq, slot, stride, cam);
        let sph_zone = Self::upload_zone_rel(&self.npc_sph_parts, NPC_SPH_SLOT_BASE, near_sq, slot, stride, cam);
        (box_zone, cyl_zone, sph_zone)
    }

    /// 相对相机位置的分档上传（模型平移列 - 相机位置）
    fn upload_zone_rel(
        parts: &[InstanceData],
        base: u32,
        near_sq: f32,
        slot: *mut u8,
        stride: usize,
        cam: glam::Vec3,
    ) -> (u32, u32) {
        let mut near_count = 0u32;
        for inst in parts {
            let dx = inst.model[12] - cam.x;
            let dy = inst.model[13] - cam.y;
            let dz = inst.model[14] - cam.z;
            if dx * dx + dy * dy + dz * dz < near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((base + near_count) as usize) * stride),
                        stride,
                    );
                }
                near_count += 1;
            }
        }
        let mut far_count = 0u32;
        for inst in parts {
            let dx = inst.model[12] - cam.x;
            let dy = inst.model[13] - cam.y;
            let dz = inst.model[14] - cam.z;
            if dx * dx + dy * dy + dz * dz >= near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((base + near_count + far_count) as usize) * stride),
                        stride,
                    );
                }
                far_count += 1;
            }
        }
        (near_count, far_count)
    }

    /// 每帧视锥剔除 + 距离 LOD 分档（多核并行）：
    /// 可见实例按 [近档][远档] 连续压缩上传到当前帧 slot，返回 (near, far)。
    ///
    /// 并行结构（`cpu::scene_pool()`：AMD 绑首簇 CCD0、Intel 仅 P-core，与渲染主线程
    /// 同簇，杜绝跨 CCD 访问与 E-core 调度；渲染线程不固定 1-2 核——池与主线程同绑整簇
    /// 集合，由 OS 调度器把渲染侧工作分给集合内空闲率最高的核）：
    /// - 阶段 A：按段并行剔除（每段走 SIMD 选路，见 cull_spheres_dispatch），
    ///   可见全局索引写入 `culled_scratch` 对应段，同时统计各段近/远档计数；
    /// - 前缀和（串行，段数 ≤ 9，微秒级）：算出每段近/远档在 slot 中的写入起点；
    /// - 阶段 B：按段并行把 80B 实例拷贝到 slot 压缩偏移（段内近档在前、远档随后）。
    fn cull_and_upload(
        &mut self,
        view: glam::Mat4,
        proj: glam::Mat4,
        cam_pos: glam::Vec3,
    ) -> (u32, u32) {
        let planes = Self::extract_frustum_planes(view, proj);
        let near_sq = quality_params(self.quality).instance_lod_distance;
        let near_sq = near_sq * near_sq;
        let stride = std::mem::size_of::<InstanceData>();
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return (0, 0),
        };

        // 池大小（调用线程参与首段 → 并发 = workers+1）；段计数数组按需建一次
        let pool = crate::engine::cpu::scene_pool();
        // 段数硬夹 `CULL_MAX_SEGMENTS`（下面两张前缀和表是栈上定长数组，见该常量的注释）
        let nw = cull_segment_count(pool.workers());
        if self.seg_near_counts.len() != nw {
            self.seg_near_counts = (0..nw)
                .map(|_| std::sync::atomic::AtomicU32::new(0))
                .collect();
            self.seg_far_counts = (0..nw)
                .map(|_| std::sync::atomic::AtomicU32::new(0))
                .collect();
        }

        // 拆借引用（同一 self 的多字段并行借用；闭包 move 捕获各自引用）
        let cx = &self.instance_center_x;
        let cy = &self.instance_center_y;
        let cz = &self.instance_center_z;
        let radii = &self.instance_radii;
        let instances = &self.instances;
        let seg_near = &self.seg_near_counts;
        let seg_far = &self.seg_far_counts;
        let scratch = &mut self.culled_scratch;

        // ---- 阶段 A：并行剔除 + 近/远分档计数（段内 SIMD 选路，见 cull_spheres_dispatch）----
        pool.par_for_each_mut(scratch, move |seg, start, seg_slice| {
            let end = start + seg_slice.len();
            // 每段局部剔除结果（段实例数为容量上限，可见数通常远小于此）
            let mut local: Vec<u32> = Vec::with_capacity(seg_slice.len());
            Self::cull_spheres_dispatch(
                &cx[start..end],
                &cy[start..end],
                &cz[start..end],
                &radii[start..end],
                &planes,
                &mut local,
            );
            // 段暂存写入全局索引（段内偏移 + 段起点）
            for (k, &li) in local.iter().enumerate() {
                seg_slice[k] = (start + li as usize) as u32;
            }
            // 近/远档计数（与串行版同一距离² 判定，结果一致）
            let mut near = 0u32;
            let mut far = 0u32;
            for &gi in &seg_slice[..local.len()] {
                let inst = &instances[gi as usize];
                let dx = inst.model[12] - cam_pos.x;
                let dy = inst.model[13] - cam_pos.y;
                let dz = inst.model[14] - cam_pos.z;
                if dx * dx + dy * dy + dz * dz < near_sq {
                    near += 1;
                } else {
                    far += 1;
                }
            }
            seg_near[seg].store(near, std::sync::atomic::Ordering::Relaxed);
            seg_far[seg].store(far, std::sync::atomic::Ordering::Relaxed);
        });

        // ---- 前缀和（串行，段数 ≤ 9，微秒级）：每段近/远档写入起点 ----
        // 表长 = `CULL_MAX_SEGMENTS`，而 `nw` 已由 `cull_segment_count` 夹到同一上限
        // （原来这里是一句 `debug_assert!(nw <= 64)` —— release 里不存在，等于没写）
        let mut near_prefix = [0u32; CULL_MAX_SEGMENTS];
        let mut far_prefix = [0u32; CULL_MAX_SEGMENTS];
        let mut near_total = 0u32;
        let mut far_total = 0u32;
        for w in 0..nw {
            near_prefix[w] = near_total;
            near_total += seg_near[w].load(std::sync::atomic::Ordering::Relaxed);
            far_prefix[w] = far_total;
            far_total += seg_far[w].load(std::sync::atomic::Ordering::Relaxed);
        }

        // ---- 阶段 B：按段并行压缩上传（近档在前、远档随后；段间偏移由前缀和保证互不相交）----
        let slot_ptr = crate::engine::cpu::SendPtr(slot);
        pool.par_for_each_mut(scratch, move |seg, _start, seg_slice| {
            let count = (seg_near[seg].load(std::sync::atomic::Ordering::Relaxed)
                + seg_far[seg].load(std::sync::atomic::Ordering::Relaxed))
                as usize;
            let near_off = near_prefix[seg] as usize;
            let far_off = (near_total + far_prefix[seg]) as usize;
            let base = slot_ptr.get();
            let mut near_k = 0usize;
            let mut far_k = 0usize;
            for &gi in &seg_slice[..count] {
                let inst = &instances[gi as usize];
                let dx = inst.model[12] - cam_pos.x;
                let dy = inst.model[13] - cam_pos.y;
                let dz = inst.model[14] - cam_pos.z;
                // SAFETY: base 指向当前帧实例槽（映射内存），偏移由前缀和保证落在槽内
                let dst = unsafe {
                    if dx * dx + dy * dy + dz * dz < near_sq {
                        let d = base.add((near_off + near_k) * stride);
                        near_k += 1;
                        d
                    } else {
                        let d = base.add((far_off + far_k) * stride);
                        far_k += 1;
                        d
                    }
                };
                // SAFETY: 段内近/远档写入游标互不相交；段间偏移由前缀和保证互不相交；
                // par_for_each_mut join 后才返回，slot 在本次调用内不会再被触碰。
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        dst,
                        stride,
                    );
                }
            }
        });
        (near_total, far_total)
    }

    /// 按运行时指令集选路执行视锥剔除（各档路径与标量逐位一致，非 FMA）：
    /// x86_64：AVX-512（16 实例/批）> AVX2（8）> AVX（8）> SSE4.2（4）> 标量；
    /// aarch64：NEON（4）> 标量；其余平台：标量。
    /// ★ AVX-512 说明：本机 Zen4（8940HX，双 256 单元合并执行 512 位）实测走
    ///   16 实例/批路径；选路统一走 cpu::avx512_enabled()——硬件不支持、
    ///   RV3D_DISABLE_AVX512=1、Intel 11 代（能效差）与 12 代起（大小核）自动禁用回退。
    fn cull_spheres_dispatch(
        cx: &[f32],
        cy: &[f32],
        cz: &[f32],
        radii: &[f32],
        planes: &[[f32; 4]; 6],
        out: &mut Vec<u32>,
    ) {
        #[cfg(target_arch = "x86_64")]
        {
            // 基准用强制选路（RV3D_FORCE_SIMD，见 cpu::forced_simd_path）；仍要求硬件支持
            if let Some(forced) = crate::engine::cpu::forced_simd_path() {
                let supported = match forced {
                    "avx512" => std::is_x86_feature_detected!("avx512f"),
                    "avx2" => std::is_x86_feature_detected!("avx2"),
                    "avx" => std::is_x86_feature_detected!("avx"),
                    "sse4.2" => std::is_x86_feature_detected!("sse4.2"),
                    "scalar" => true,
                    _ => false,
                };
                if supported {
                    match forced {
                        "avx512" => {
                            // safety: 上面已确认 avx512f 硬件支持
                            unsafe {
                                Self::cull_spheres_avx512(cx, cy, cz, radii, planes, out);
                            }
                        }
                        "avx2" => {
                            // safety: 上面已确认 avx2 硬件支持
                            unsafe {
                                Self::cull_spheres_avx2(cx, cy, cz, radii, planes, out);
                            }
                        }
                        "avx" => {
                            // safety: 上面已确认 avx 硬件支持
                            unsafe {
                                Self::cull_spheres_avx(cx, cy, cz, radii, planes, out);
                            }
                        }
                        "sse4.2" => {
                            // safety: 上面已确认 sse4.2 硬件支持
                            unsafe {
                                Self::cull_spheres_sse(cx, cy, cz, radii, planes, out);
                            }
                        }
                        _ => Self::cull_spheres_scalar(cx, cy, cz, radii, planes, out),
                    }
                    return;
                }
                // 每帧调用（morph 每级 / 剔除每段）⇒ 走一次性告警，见 `simd::warn_forced_simd_unsupported`
                crate::engine::simd::warn_forced_simd_unsupported(forced);
            }
            if crate::engine::cpu::avx512_enabled() {
                // safety: 上面已运行时检测 AVX-512，CPU 支持才进入该分支
                unsafe {
                    Self::cull_spheres_avx512(cx, cy, cz, radii, planes, out);
                }
            } else if std::is_x86_feature_detected!("avx2") {
                // safety: 上面已运行时检测 AVX2
                unsafe {
                    Self::cull_spheres_avx2(cx, cy, cz, radii, planes, out);
                }
            } else if std::is_x86_feature_detected!("avx") {
                // safety: 上面已运行时检测 AVX
                unsafe {
                    Self::cull_spheres_avx(cx, cy, cz, radii, planes, out);
                }
            } else if std::is_x86_feature_detected!("sse4.2") {
                // safety: 上面已运行时检测 SSE4.2
                unsafe {
                    Self::cull_spheres_sse(cx, cy, cz, radii, planes, out);
                }
            } else {
                Self::cull_spheres_scalar(cx, cy, cz, radii, planes, out);
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            if std::arch::is_aarch64_feature_detected!("neon") {
                // safety: NEON 在 AArch64 是基线特性（此处仍运行时确认）
                unsafe {
                    Self::cull_spheres_neon(cx, cy, cz, radii, planes, out);
                }
            } else {
                Self::cull_spheres_scalar(cx, cy, cz, radii, planes, out);
            }
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self::cull_spheres_scalar(cx, cy, cz, radii, planes, out);
        }
    }

    /// 提交一次性命令（用于纹理布局转换、数据拷贝等）
    fn run_single_time_commands(&self, f: impl FnOnce(vk::CommandBuffer)) -> Result<(), String> {
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd_buffer = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("分配一次性命令缓冲失败: {}", e))?
        }[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            self.device
                .begin_command_buffer(cmd_buffer, &begin_info)
                .map_err(|e| format!("开始一次性命令缓冲失败: {}", e))?;
        }

        f(cmd_buffer);

        unsafe {
            self.device
                .end_command_buffer(cmd_buffer)
                .map_err(|e| format!("结束一次性命令缓冲失败: {}", e))?;
        }

        let cmd_buffers = [cmd_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_buffers);
        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], vk::Fence::null())
                .map_err(|e| format!("提交一次性命令失败: {}", e))?;
            self.device
                .queue_wait_idle(self.graphics_queue)
                .map_err(|e| format!("等待一次性命令失败: {}", e))?;
            self.device.free_command_buffers(self.command_pool, &[cmd_buffer]);
        }
        Ok(())
    }

    /// 惰性创建截图读回资源：按 max_frames_in_flight 双缓冲 HOST_VISIBLE staging buffer + fence，
    /// 避免与 in-flight 帧竞态（capture_screenshot 首次调用时创建；交换链重建后作废重建）。
    fn init_screenshot_resources(&mut self) -> Result<(), String> {
        let size = (self.swapchain_extent.width as u64) * (self.swapchain_extent.height as u64) * 4;
        if size == 0 {
            return Err("交换链尺寸为 0，无法创建截图缓冲".to_string());
        }
        let mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        for _ in 0..self.max_frames_in_flight {
            let buffer_info = vk::BufferCreateInfo::default()
                .size(size)
                .usage(vk::BufferUsageFlags::TRANSFER_DST)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = match unsafe { self.device.create_buffer(&buffer_info, None) } {
                Ok(b) => b,
                Err(e) => {
                    self.destroy_screenshot_resources();
                    return Err(format!("创建截图 staging buffer 失败: {}", e));
                }
            };
            self.screenshot_buffers.push(buffer);

            let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(buffer) };
            let memory_type = mem_props
                .memory_types
                .iter()
                .enumerate()
                .find(|(i, mem_type)| {
                    let type_mask = 1 << i;
                    (mem_reqs.memory_type_bits & type_mask) != 0
                        && mem_type
                            .property_flags
                            .contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                        && mem_type
                            .property_flags
                            .contains(vk::MemoryPropertyFlags::HOST_COHERENT)
                })
                .map(|(i, _)| i as u32)
                .ok_or_else(|| "没有找到合适的内存类型（截图 staging buffer）".to_string())
                .map_err(|e| {
                    self.destroy_screenshot_resources();
                    e
                })?;
            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_reqs.size)
                .memory_type_index(memory_type);
            let memory = match unsafe { self.device.allocate_memory(&alloc_info, None) } {
                Ok(m) => m,
                Err(e) => {
                    self.destroy_screenshot_resources();
                    return Err(format!("分配截图 staging buffer 内存失败: {}", e));
                }
            };
            self.screenshot_buffers_memory.push(memory);

            if let Err(e) = unsafe { self.device.bind_buffer_memory(buffer, memory, 0) } {
                self.destroy_screenshot_resources();
                return Err(format!("绑定截图 staging buffer 内存失败: {}", e));
            }

            let fence = match unsafe {
                self.device
                    .create_fence(&vk::FenceCreateInfo::default(), None)
            } {
                Ok(f) => f,
                Err(e) => {
                    self.destroy_screenshot_resources();
                    return Err(format!("创建截图围栏失败: {}", e));
                }
            };
            self.screenshot_fences.push(fence);
        }
        Ok(())
    }

    /// 销毁截图读回资源（交换链重建 / Drop 时调用；字段归零，下次截图惰性重建）
    fn destroy_screenshot_resources(&mut self) {
        for (&buffer, &memory) in self
            .screenshot_buffers
            .iter()
            .zip(self.screenshot_buffers_memory.iter())
        {
            if buffer != vk::Buffer::null() {
                unsafe { self.device.destroy_buffer(buffer, None) };
            }
            if memory != vk::DeviceMemory::null() {
                unsafe { self.device.free_memory(memory, None) };
            }
        }
        self.screenshot_buffers.clear();
        self.screenshot_buffers_memory.clear();
        for &fence in &self.screenshot_fences {
            if fence != vk::Fence::null() {
                unsafe { self.device.destroy_fence(fence, None) };
            }
        }
        self.screenshot_fences.clear();
    }

    /// 读回当前帧 swapchain 图像并保存 PNG。
    /// 在 render() 提交渲染之后、present 之前调用：此时图像内容已确定，
    /// 且 render_finished 信号量尚未被 present 消费，主机侧等待不会死锁。
    /// 流程：等待信号量 → 一次性命令（布局转换 + 拷贝）→ wait fence → map 读取 → 保存。
    fn do_screenshot_readback(&mut self, image: vk::Image) -> Result<(), String> {
        let path = match self.screenshot_request.take() {
            Some(p) => p,
            None => return Ok(()),
        };
        let width = self.swapchain_extent.width;
        let height = self.swapchain_extent.height;
        let format = self.swapchain_format;
        let slot = self.current_frame;
        let buffer = *self
            .screenshot_buffers
            .get(slot)
            .ok_or_else(|| "截图缓冲未初始化".to_string())?;
        let memory = *self
            .screenshot_buffers_memory
            .get(slot)
            .ok_or_else(|| "截图缓冲内存未初始化".to_string())?;
        let fence = *self
            .screenshot_fences
            .get(slot)
            .ok_or_else(|| "截图围栏未初始化".to_string())?;
        let buffer_size = (width as u64) * (height as u64) * 4;

        // 1. 主机侧等待本帧渲染完成：vkWaitSemaphores 只接受 timeline 信号量，
        //    这里复用 in_flight_fence（本帧 queue_submit 已提交，等待不会死锁）。
        unsafe {
            self.device
                .wait_for_fences(
                    &[self.in_flight_fences[slot]],
                    true,
                    SCREENSHOT_WAIT_TIMEOUT_NS,
                )
                .map_err(|e| format!("等待渲染完成围栏失败: {}", e))?;
        }

        // 2. 一次性命令缓冲：PRESENT_SRC_KHR → TRANSFER_SRC_OPTIMAL → 拷贝 → 回 PRESENT_SRC_KHR
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd_buffer = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("分配截图命令缓冲失败: {}", e))?
        }[0];
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        let subresource_range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let barrier_to_transfer = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource_range)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
        let barrier_to_present = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource_range)
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::empty());
        let copy_region = vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1),
            )
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D { width, height, depth: 1 });
        unsafe {
            self.device
                .begin_command_buffer(cmd_buffer, &begin_info)
                .map_err(|e| format!("开始截图命令缓冲失败: {}", e))?;
            self.device.cmd_pipeline_barrier(
                cmd_buffer,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_to_transfer],
            );
            self.device.cmd_copy_image_to_buffer(
                cmd_buffer,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer,
                &[copy_region],
            );
            self.device.cmd_pipeline_barrier(
                cmd_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_to_present],
            );
            self.device
                .end_command_buffer(cmd_buffer)
                .map_err(|e| format!("结束截图命令缓冲失败: {}", e))?;
        }

        // 3. 提交拷贝命令（独立 fence），等待完成后再释放命令缓冲
        unsafe {
            self.device
                .reset_fences(&[fence])
                .map_err(|e| format!("重置截图围栏失败: {}", e))?;
        }
        let cmd_buffers = [cmd_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_buffers);
        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence)
                .map_err(|e| format!("提交截图命令失败: {}", e))?;
            self.device
                .wait_for_fences(&[fence], true, u64::MAX)
                .map_err(|e| format!("等待截图围栏失败: {}", e))?;
            self.device.free_command_buffers(self.command_pool, &[cmd_buffer]);
        }

        // 4. map 读取像素 → 格式转换 → 保存 PNG
        let data_ptr = unsafe {
            self.device
                .map_memory(memory, 0, buffer_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射截图缓冲失败: {}", e))?
        };
        let mut raw = vec![0u8; (width * height * 4) as usize];
        unsafe {
            std::ptr::copy_nonoverlapping(data_ptr as *const u8, raw.as_mut_ptr(), raw.len());
        }
        unsafe {
            self.device.unmap_memory(memory);
        }
        let mut rgba = vec![0u8; raw.len()];
        convert_pixels_to_rgba(format, &raw, &mut rgba)?;
        let img = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| "创建 RGBA 图像失败".to_string())?;
        img.save_with_format(&path, image::ImageFormat::Png)
            .map_err(|e| format!("保存截图失败 '{}': {}", path.display(), e))?;
        log::info!("截图已保存: {}", path.display());
        Ok(())
    }

    /// 创建一张带完整 mip 链的采样纹理：
    /// staging buffer → Image（SAMPLED|TRANSFER_DST|TRANSFER_SRC）→ 逐级 blit 生成 mip →
    /// ImageView。地面纹理之外的附加贴图（marker/NPC 程序化皮肤纹理、地面微细节 tile）
    /// 共用此路径，采样器复用主纹理的 texture_sampler（尺寸同规格，mip 级数一致）。
    ///
    /// `format` 决定 view 的色彩空间：皮肤图传 `R8G8B8A8_SRGB`（存的是显示编码色），
    /// 地面细节层必须传 `R8G8B8A8_UNORM`——它存的是**线性亮度调制**（纹素 = 调制/2），
    /// 用 SRGB view 会把 128 解码成 0.214，乘 2 后得 0.43 → 全场地面暗一半。
    fn create_sampled_image(
        &self,
        pixels: &[u8],
        width: u32,
        height: u32,
        format: vk::Format,
    ) -> Result<(vk::Image, vk::DeviceMemory, vk::ImageView), String> {
        let image_size = (width * height * 4) as u64;
        // mip 链级别数：按长边逐次减半直至 1
        let mut mip_levels = 1u32;
        let mut largest = width.max(height);
        while largest > 1 {
            largest >>= 1;
            mip_levels += 1;
        }

        // ---- 1. staging buffer：CPU 写入像素数据 ----
        let buffer_info = vk::BufferCreateInfo::default()
            .size(image_size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let staging_buffer = unsafe {
            self.device
                .create_buffer(&buffer_info, None)
                .map_err(|e| format!("创建纹理 staging buffer 失败: {e}"))?
        };

        let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(staging_buffer) };
        let mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let memory_type = mem_props
            .memory_types
            .iter()
            .enumerate()
            .find(|(i, mem_type)| {
                let type_mask = 1 << i;
                (mem_reqs.memory_type_bits & type_mask) != 0
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT)
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到合适的内存类型（纹理 staging buffer）".to_string())?;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(memory_type);
        let staging_memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配纹理 staging buffer 内存失败: {e}"))?
        };
        unsafe {
            self.device
                .bind_buffer_memory(staging_buffer, staging_memory, 0)
                .map_err(|e| format!("绑定纹理 staging buffer 内存失败: {e}"))?;
            let data_ptr = self
                .device
                .map_memory(staging_memory, 0, image_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射纹理 staging buffer 失败: {e}"))?;
            std::ptr::copy_nonoverlapping(
                pixels.as_ptr() as *const u8,
                data_ptr as *mut u8,
                pixels.len(),
            );
            self.device.unmap_memory(staging_memory);
        }

        // ---- 2. Vulkan Image（SAMPLED | TRANSFER_DST | TRANSFER_SRC）----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(mip_levels)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::TRANSFER_SRC,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe {
            self.device
                .create_image(&image_info, None)
                .map_err(|e| format!("创建纹理 Image 失败: {e}"))?
        };

        let img_reqs = unsafe { self.device.get_image_memory_requirements(image) };
        let img_mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let img_memory_type = img_mem_props
            .memory_types
            .iter()
            .enumerate()
            .find(|(i, mem_type)| {
                let type_mask = 1 << i;
                (img_reqs.memory_type_bits & type_mask) != 0
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .or_else(|| {
                img_mem_props.memory_types.iter().enumerate().find(|(i, _)| {
                    let type_mask = 1 << i;
                    (img_reqs.memory_type_bits & type_mask) != 0
                })
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到合适的内存类型（纹理 Image）".to_string())?;

        let img_alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(img_reqs.size)
            .memory_type_index(img_memory_type);
        let image_memory = unsafe {
            self.device
                .allocate_memory(&img_alloc_info, None)
                .map_err(|e| format!("分配纹理 Image 内存失败: {e}"))?
        };
        unsafe {
            self.device
                .bind_image_memory(image, image_memory, 0)
                .map_err(|e| format!("绑定纹理 Image 内存失败: {e}"))?;
        }

        // ---- 3. 拷贝 staging buffer → Image，生成 mip 链，转 SHADER_READ_ONLY_OPTIMAL ----
        self.run_single_time_commands(|cmd| {
            let subresource_range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(mip_levels)
                .base_array_layer(0)
                .layer_count(1);

            // UNDEFINED → TRANSFER_DST_OPTIMAL
            let barrier_to_transfer = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(subresource_range)
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier_to_transfer],
                );
            }

            // 拷贝像素数据
            let region = vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(0)
                        .base_array_layer(0)
                        .layer_count(1),
                )
                .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                .image_extent(vk::Extent3D { width, height, depth: 1 });
            unsafe {
                self.device.cmd_copy_buffer_to_image(
                    cmd,
                    staging_buffer,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
            }

            // 逐级生成 mip：上一级 TRANSFER_DST → TRANSFER_SRC，再 blit 缩小到本级
            for mip in 1..mip_levels {
                let src_w = (width >> (mip - 1)).max(1);
                let src_h = (height >> (mip - 1)).max(1);
                let dst_w = (width >> mip).max(1);
                let dst_h = (height >> mip).max(1);

                let level_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(mip - 1)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1);

                // TRANSFER_DST_OPTIMAL → TRANSFER_SRC_OPTIMAL
                let barrier_to_blit_src = vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(level_range)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
                unsafe {
                    self.device.cmd_pipeline_barrier(
                        cmd,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[barrier_to_blit_src],
                    );
                }

                let blit_region = vk::ImageBlit::default()
                    .src_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(mip - 1)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .src_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D { x: src_w as i32, y: src_h as i32, z: 1 },
                    ])
                    .dst_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(mip)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .dst_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D { x: dst_w as i32, y: dst_h as i32, z: 1 },
                    ]);
                unsafe {
                    self.device.cmd_blit_image(
                        cmd,
                        image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[blit_region],
                        vk::Filter::LINEAR,
                    );
                }
            }

            // 全部 mip → SHADER_READ_ONLY_OPTIMAL（基级们 TRANSFER_SRC、末级 TRANSFER_DST）
            let mut read_barriers = Vec::new();
            if mip_levels > 1 {
                let read_src_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(mip_levels - 1)
                    .base_array_layer(0)
                    .layer_count(1);
                read_barriers.push(
                    vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(image)
                        .subresource_range(read_src_range)
                        .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ),
                );
            }
            let read_last_range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(mip_levels - 1)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1);
            read_barriers.push(
                vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(read_last_range)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ),
            );
            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &read_barriers,
                );
            }
        })?;

        // 释放 staging buffer
        unsafe {
            self.device.free_memory(staging_memory, None);
            self.device.destroy_buffer(staging_buffer, None);
        }

        // ---- 4. Image View（2D 类型）----
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(mip_levels)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        let view = unsafe {
            self.device
                .create_image_view(&view_info, None)
                .map_err(|e| format!("创建纹理 Image View 失败: {e}"))?
        };

        Ok((image, image_memory, view))
    }

    /// 加载 assets/textures/test.png 并创建纹理资源
    fn init_texture(&mut self) -> Result<(), String> {
        // 程序化地面纹理（CPU 画像素 + 烘焙高度场 AO/静态天光，零第三方依赖）。
        // 世界空间对齐：与 build.rs 片元着色器 world-space UV 一致（见 procedural.rs）。
        // RV3D_PROC_TEX=0 回退到 assets/textures/test.png（A/B 验证程序化材质效果）。
        let (width, height, pixels) = if std::env::var("RV3D_PROC_TEX").as_deref() != Ok("0") {
            let size = super::procedural::GROUND_TEXTURE_SIZE;
            let height_at = |x: f32, z: f32| terrain_height(x, z);
            (
                size,
                size,
                super::procedural::generate_city_ground_texture(size, &height_at),
            )
        } else {
            let texture_path = "assets/textures/test.png";
            let img = image::open(texture_path)
                .map_err(|e| format!("加载纹理图片失败 '{}': {}", texture_path, e))?
                .to_rgba8();
            (img.width(), img.height(), img.as_raw().clone())
        };
        let image_size = (width * height * 4) as u64;
        // mip 链级别数：按长边逐次减半直至 1
        let mut mip_levels = 1u32;
        let mut largest = width.max(height);
        while largest > 1 {
            largest >>= 1;
            mip_levels += 1;
        }

        // ---- 1. staging buffer：CPU 写入像素数据 ----
        let buffer_info = vk::BufferCreateInfo::default()
            .size(image_size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let staging_buffer = unsafe {
            self.device
                .create_buffer(&buffer_info, None)
                .map_err(|e| format!("创建纹理 staging buffer 失败: {}", e))?
        };

        let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(staging_buffer) };
        let mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let memory_type = mem_props
            .memory_types
            .iter()
            .enumerate()
            .find(|(i, mem_type)| {
                let type_mask = 1 << i;
                (mem_reqs.memory_type_bits & type_mask) != 0
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT)
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到合适的内存类型（纹理 staging buffer）".to_string())?;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(memory_type);
        let staging_memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配纹理 staging buffer 内存失败: {}", e))?
        };
        unsafe {
            self.device
                .bind_buffer_memory(staging_buffer, staging_memory, 0)
                .map_err(|e| format!("绑定纹理 staging buffer 内存失败: {}", e))?;
            let data_ptr = self
                .device
                .map_memory(staging_memory, 0, image_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射纹理 staging buffer 失败: {}", e))?;
            std::ptr::copy_nonoverlapping(
                pixels.as_ptr() as *const u8,
                data_ptr as *mut u8,
                pixels.len(),
            );
            self.device.unmap_memory(staging_memory);
        }

        // ---- 2. Vulkan Image（SAMPLED | TRANSFER_DST）----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_SRGB)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(mip_levels)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::TRANSFER_SRC,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        self.texture_image = unsafe {
            self.device
                .create_image(&image_info, None)
                .map_err(|e| format!("创建纹理 Image 失败: {}", e))?
        };

        let img_reqs = unsafe { self.device.get_image_memory_requirements(self.texture_image) };
        let img_mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let img_memory_type = img_mem_props
            .memory_types
            .iter()
            .enumerate()
            .find(|(i, mem_type)| {
                let type_mask = 1 << i;
                (img_reqs.memory_type_bits & type_mask) != 0
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .or_else(|| {
                img_mem_props.memory_types.iter().enumerate().find(|(i, _)| {
                    let type_mask = 1 << i;
                    (img_reqs.memory_type_bits & type_mask) != 0
                })
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到合适的内存类型（纹理 Image）".to_string())?;

        let img_alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(img_reqs.size)
            .memory_type_index(img_memory_type);
        self.texture_image_memory = unsafe {
            self.device
                .allocate_memory(&img_alloc_info, None)
                .map_err(|e| format!("分配纹理 Image 内存失败: {}", e))?
        };
        unsafe {
            self.device
                .bind_image_memory(self.texture_image, self.texture_image_memory, 0)
                .map_err(|e| format!("绑定纹理 Image 内存失败: {}", e))?;
        }

        // ---- 3. 拷贝 staging buffer → Image，并转换布局 ----
        let image = self.texture_image;
        self.run_single_time_commands(|cmd| {
            let subresource_range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(mip_levels)
                .base_array_layer(0)
                .layer_count(1);

            // UNDEFINED → TRANSFER_DST_OPTIMAL
            let barrier_to_transfer = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(subresource_range)
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier_to_transfer],
                );
            }

            // 拷贝像素数据
            let region = vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(0)
                        .base_array_layer(0)
                        .layer_count(1),
                )
                .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                .image_extent(vk::Extent3D { width, height, depth: 1 });
            unsafe {
                self.device.cmd_copy_buffer_to_image(
                    cmd,
                    staging_buffer,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
            }

            // 逐级生成 mip：上一级 TRANSFER_DST → TRANSFER_SRC，再 blit 缩小到本级
            for mip in 1..mip_levels {
                let src_w = (width >> (mip - 1)).max(1);
                let src_h = (height >> (mip - 1)).max(1);
                let dst_w = (width >> mip).max(1);
                let dst_h = (height >> mip).max(1);

                let level_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(mip - 1)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1);

                // TRANSFER_DST_OPTIMAL → TRANSFER_SRC_OPTIMAL
                let barrier_to_blit_src = vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(level_range)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
                unsafe {
                    self.device.cmd_pipeline_barrier(
                        cmd,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[barrier_to_blit_src],
                    );
                }

                let blit_region = vk::ImageBlit::default()
                    .src_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(mip - 1)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .src_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D { x: src_w as i32, y: src_h as i32, z: 1 },
                    ])
                    .dst_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(mip)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .dst_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D { x: dst_w as i32, y: dst_h as i32, z: 1 },
                    ]);
                unsafe {
                    self.device.cmd_blit_image(
                        cmd,
                        image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[blit_region],
                        vk::Filter::LINEAR,
                    );
                }
            }

            // 全部 mip → SHADER_READ_ONLY_OPTIMAL（基级们 TRANSFER_SRC、末级 TRANSFER_DST）
            let mut read_barriers = Vec::new();
            if mip_levels > 1 {
                let read_src_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(mip_levels - 1)
                    .base_array_layer(0)
                    .layer_count(1);
                read_barriers.push(
                    vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(image)
                        .subresource_range(read_src_range)
                        .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ),
                );
            }
            let read_last_range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(mip_levels - 1)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1);
            read_barriers.push(
                vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(read_last_range)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ),
            );
            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &read_barriers,
                );
            }
        })?;

        // 释放 staging buffer
        unsafe {
            self.device.free_memory(staging_memory, None);
            self.device.destroy_buffer(staging_buffer, None);
        }

        // ---- 4. Image View（2D 类型）----
        let view_info = vk::ImageViewCreateInfo::default()
            .image(self.texture_image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_SRGB)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(mip_levels)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        self.texture_image_view = unsafe {
            self.device
                .create_image_view(&view_info, None)
                .map_err(|e| format!("创建纹理 Image View 失败: {}", e))?
        };

        // ---- 5. Sampler（线性过滤）----
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .anisotropy_enable(self.texture_anisotropy_enabled)
            .max_anisotropy(if self.texture_anisotropy_enabled {
                self.physical_device_properties
                    .limits
                    .max_sampler_anisotropy
                    .min(16.0)
            } else {
                1.0
            })
            .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
            .unnormalized_coordinates(false)
            .min_lod(0.0)
            .max_lod((mip_levels - 1) as f32);
        self.texture_sampler = unsafe {
            self.device
                .create_sampler(&sampler_info, None)
                .map_err(|e| format!("创建纹理 Sampler 失败: {}", e))?
        };

        let tex_src = if std::env::var("RV3D_PROC_TEX").as_deref() == Ok("0") {
            "assets/textures/test.png"
        } else {
            "程序化地面材质"
        };
        log::info!(
            "纹理初始化完成: {}x{}（{} mip，来源={}）",
            width,
            height,
            mip_levels,
            tex_src
        );

        // ---- marker/NPC 程序化皮肤纹理（CPU 画像素，零依赖）----
        // RV3D_SKIN_TEX=1 时片元着色器采样（light_data.flags.z 通知）；缺省 0 纯色回退。
        // 纹理恒创建（shader 静态引用 binding 7/8，descriptor 必须有效），仅门控采样路径。
        let skin_size = super::procedural::SKIN_TEXTURE_SIZE;
        let (img, mem, view) = self.create_sampled_image(
            &super::procedural::generate_default_marker_skin_texture(),
            skin_size,
            skin_size,
            vk::Format::R8G8B8A8_SRGB,
        )?;
        self.skin_marker_image = img;
        self.skin_marker_memory = mem;
        self.skin_marker_image_view = view;
        let (img, mem, view) = self.create_sampled_image(
            &super::procedural::generate_default_npc_skin_texture(),
            skin_size,
            skin_size,
            vk::Format::R8G8B8A8_SRGB,
        )?;
        self.skin_npc_image = img;
        self.skin_npc_memory = mem;
        self.skin_npc_image_view = view;
        log::info!(
            "程序化皮肤纹理初始化完成: {}x{}（marker=木板墙, npc=迷彩军服, RV3D_SKIN_TEX={}）",
            skin_size,
            skin_size,
            if self.skin_tex_enabled { "on" } else { "off（纯色回退）" }
        );

        // ---- 地面微细节层（binding 9；build.rs 片元 `ground_detail_tex`）----
        // ⚠ 恒创建、恒绑定，**不受任何环境变量门控**：片元是无条件采样它的，缺这个
        // 描述符不会报错、只会让驱动回吐 0，于是 `mixed *= mix(1.0, 0*2, gdetail)`
        // 把相机周边整圈地面乘成纯黑（2026-09-03 大面积黑地根因）。
        // 格式必须是 UNORM（线性）：纹素存的是「亮度调制 / 2」而不是显示编码颜色。
        // 采样器复用 texture_sampler（binding 3）：REPEAT + LINEAR/LINEAR-mip，
        // 正是平铺细节层要的（build.rs 用 textureSampleLevel 显式选 mip）。
        let detail_size = super::procedural::GROUND_DETAIL_SIZE;
        let (img, mem, view) = self.create_sampled_image(
            &super::procedural::generate_default_ground_detail_texture(),
            detail_size,
            detail_size,
            vk::Format::R8G8B8A8_UNORM,
        )?;
        self.ground_detail_image = img;
        self.ground_detail_memory = mem;
        self.ground_detail_image_view = view;
        log::info!(
            "地面微细节层初始化完成: {}x{} 覆盖 {}m（{} 纹素/米，UNORM 线性，绑定 binding {}）",
            detail_size,
            detail_size,
            super::procedural::GROUND_DETAIL_METRES,
            detail_size as f32 / super::procedural::GROUND_DETAIL_METRES,
            GROUND_DETAIL_BINDING
        );
        Ok(())
    }

    // ============================================================
    // 阴影贴图（2026-08-11）：depth-only pass 渲光空间深度，主 pass 3x3 PCF 采样
    // ============================================================
    /// 创建阴影贴图资源：2048x2048 D32_SFLOAT（DEPTH_STENCIL_ATTACHMENT | SAMPLED）、
    /// depth-compare 采样器、depth-only render pass、framebuffer、每帧 shadow UBO、
    /// shadow descriptor set layout + sets（binding 0 = shadow UBO，binding 2 = 实例 storage）。
    fn init_shadow_resources(&mut self) -> Result<(), String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;

        // ---- 1. 阴影图 Image + 内存 + View ----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .extent(vk::Extent3D {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT
                    | vk::ImageUsageFlags::SAMPLED,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let shadow_image = unsafe {
            self.device
                .create_image(&image_info, None)
                .map_err(|e| format!("创建阴影图 Image 失败: {}", e))?
        };
        let mem_reqs = unsafe { self.device.get_image_memory_requirements(shadow_image) };
        let memory_type = self.pick_memory_type(mem_reqs, true)?;
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(memory_type);
        let shadow_image_memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配阴影图 Image 内存失败: {}", e))?
        };
        unsafe {
            self.device
                .bind_image_memory(shadow_image, shadow_image_memory, 0)
                .map_err(|e| format!("绑定阴影图 Image 内存失败: {}", e))?;
        }
        let view_info = vk::ImageViewCreateInfo::default()
            .image(shadow_image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::DEPTH)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        let shadow_image_view = unsafe {
            self.device
                .create_image_view(&view_info, None)
                .map_err(|e| format!("创建阴影图 Image View 失败: {}", e))?
        };
        self.shadow_image = shadow_image;
        self.shadow_image_memory = shadow_image_memory;
        self.shadow_image_view = shadow_image_view;

        // ---- 2. 阴影采样器（PCF：NEAREST + CLAMP_TO_EDGE）----
        // 手动 PCF 用 textureSample 读原始深度再比较，必须是普通采样器：
        // comparison sampler + 非 Dref 采样在严格 Vulkan 验证下会报 VUID。
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::NEAREST)
            .min_filter(vk::Filter::NEAREST)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false)
            .compare_enable(false)
            .compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_lod(0.0)
            .max_lod(1.0)
            .border_color(vk::BorderColor::FLOAT_OPAQUE_WHITE)
            .unnormalized_coordinates(false);
        self.shadow_sampler = unsafe {
            self.device
                .create_sampler(&sampler_info, None)
                .map_err(|e| format!("创建阴影采样器失败: {}", e))?
        };

        // ---- 3. depth-only render pass（无颜色附件，clear 1.0，store 供主 pass 采样）----
        let depth_attachment = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let depth_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .depth_stencil_attachment(&depth_attachment_ref);
        let subpasses = [subpass];
        let attachments = [depth_attachment];
        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses);
        self.shadow_render_pass = unsafe {
            self.device
                .create_render_pass(&render_pass_info, None)
                .map_err(|e| format!("创建阴影渲染流程失败: {}", e))?
        };

        // ---- 4. framebuffer（单附件：阴影图 view）----
        let framebuffer_attachments = [self.shadow_image_view];
        let framebuffer_info = vk::FramebufferCreateInfo::default()
            .render_pass(self.shadow_render_pass)
            .attachments(&framebuffer_attachments)
            .width(SHADOW_MAP_SIZE)
            .height(SHADOW_MAP_SIZE)
            .layers(1);
        self.shadow_framebuffer = unsafe {
            self.device
                .create_framebuffer(&framebuffer_info, None)
                .map_err(|e| format!("创建阴影帧缓冲失败: {}", e))?
        };

        // ---- 5. shadow descriptor layout（binding 0 = UBO，binding 2 = 实例 storage）----
        let ubo_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
        let storage_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
        let shadow_bindings = [ubo_binding, storage_binding];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&shadow_bindings);
        self.shadow_descriptor_set_layout = unsafe {
            self.device
                .create_descriptor_set_layout(&layout_info, None)
                .map_err(|e| format!("创建阴影 Descriptor Set Layout 失败: {}", e))?
        };

        // ---- 6. shadow UBO（每帧 slot 一份 64B mat4）+ descriptor sets（从主 pool 分配）----
        let max_frames = self.max_frames_in_flight;
        for _ in 0..max_frames {
            let (buffer, memory, mapped) = self.create_uniform_buffer(
                std::mem::size_of::<glam::Mat4>() as u64,
            )?;
            self.shadow_ubo_buffers.push(buffer);
            self.shadow_ubo_memory.push(memory);
            self.shadow_ubo_mapped.push(mapped);
        }

        let layouts: Vec<vk::DescriptorSetLayout> = (0..max_frames)
            .map(|_| self.shadow_descriptor_set_layout)
            .collect();
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);
        self.shadow_descriptor_sets = unsafe {
            self.device
                .allocate_descriptor_sets(&alloc_info)
                .map_err(|e| format!("分配阴影 Descriptor Sets 失败: {}", e))?
        };

        // 阴影 pass 的实例范围同样用唯一定义。注意：这里此前连枪模槽（+1）都没覆盖，
        // 只是从没有 shader 在阴影里读那些槽所以没暴露；统一后一并修正。
        let instance_range =
            std::mem::size_of::<InstanceData>() as u64 * INSTANCE_BUFFER_ELEMS;
        for i in 0..max_frames {
            let ubo_info = vk::DescriptorBufferInfo::default()
                .buffer(self.shadow_ubo_buffers[i])
                .offset(0)
                .range(std::mem::size_of::<glam::Mat4>() as u64);
            let ubo_infos = [ubo_info];
            let ubo_write = vk::WriteDescriptorSet::default()
                .dst_set(self.shadow_descriptor_sets[i])
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&ubo_infos);
            let instance_info = vk::DescriptorBufferInfo::default()
                .buffer(self.instance_buffers[i])
                .offset(0)
                .range(instance_range);
            let instance_infos = [instance_info];
            let instance_write = vk::WriteDescriptorSet::default()
                .dst_set(self.shadow_descriptor_sets[i])
                .dst_binding(2)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&instance_infos);
            let writes = [ubo_write, instance_write];
            unsafe {
                self.device.update_descriptor_sets(&writes, &[]);
            }
        }

        log::info!("阴影贴图资源创建完成: {}x{} D32_SFLOAT", SHADOW_MAP_SIZE, SHADOW_MAP_SIZE);
        Ok(())
    }

    /// 阴影 depth-only 管线：与主几何共享顶点/实例布局，无颜色附件；
    /// depth bias（constant 1.25 / slope 1.75）缓解斜面 shadow acne。
    fn init_shadow_pipeline(&mut self) -> Result<(), String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;

        let vs_spirv = load_spirv("assets/shadow.vert.spv")?;
        let fs_spirv = load_spirv("assets/shadow.frag.spv")?;
        let vs_module = self.create_shader_module(&vs_spirv)?;
        let fs_module = self.create_shader_module(&fs_spirv)?;

        let vs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs_module)
            .name(c"shadow_main");
        let fs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_module)
            .name(c"fs_main");
        let shader_stages = [vs_stage, fs_stage];

        let vertex_binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<Vertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);
        // shadow VS 只读 position（location 0）；实例变换走 storage buffer（binding 2）
        let vertex_attributes = [vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(0)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(0)];
        let vertex_bindings = [vertex_binding];
        let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vertex_bindings)
            .vertex_attribute_descriptions(&vertex_attributes);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        let viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(SHADOW_MAP_SIZE as f32)
            .height(SHADOW_MAP_SIZE as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(vk::Extent2D {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
            });
        let viewports = [viewport];
        let scissors = [scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);

        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_depth_bounds(0.0)
            .max_depth_bounds(1.0);

        // 无颜色附件：color blend state 留空（Vulkan 对该场景忽略此状态）
        let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&[]);

        let set_layouts = [self.shadow_descriptor_set_layout];
        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts);
        self.shadow_pipeline_layout = unsafe {
            self.device
                .create_pipeline_layout(&layout_info, None)
                .map_err(|e| format!("创建阴影管线布局失败: {}", e))?
        };

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend_state)
            .layout(self.shadow_pipeline_layout)
            .render_pass(self.shadow_render_pass)
            .subpass(0);

        self.shadow_pipeline = unsafe {
            self.device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    &[pipeline_info],
                    None,
                )
                .map_err(|(_, e)| format!("创建阴影管线失败: {}", e))?
                .remove(0)
        };

        unsafe {
            self.device.destroy_shader_module(vs_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }
        log::info!("阴影 depth-only 管线创建完成");
        Ok(())
    }

    /// 把纹理 Image View 和 Sampler 写入每个 DescriptorSet（binding 1 / 3），
    /// 并把阴影贴图 View + depth-compare Sampler 写入 binding 5 / 6。
    fn update_texture_descriptor_sets(&mut self) -> Result<(), String> {
        for i in 0..self.descriptor_sets.len() {
            let image_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.texture_image_view)
                .sampler(self.texture_sampler);
            let image_infos = [image_info];

            let sampled_image_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(1)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image_infos);

            let sampler_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(3)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&image_infos);

            // 阴影贴图（binding 5 = SAMPLED_IMAGE，binding 6 = SAMPLER；depth-compare 采样）
            let shadow_image_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.shadow_image_view)
                .sampler(self.shadow_sampler);
            let shadow_image_infos = [shadow_image_info];
            let shadow_map_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(5)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&shadow_image_infos);
            let shadow_sampler_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(6)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&shadow_image_infos);

            // marker/NPC 程序化皮肤纹理（binding 7/8；RV3D_SKIN_TEX=1 时片元采样，缺省纯色回退）
            let marker_skin_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.skin_marker_image_view)
                .sampler(self.texture_sampler);
            let marker_skin_infos = [marker_skin_info];
            let marker_skin_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(7)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&marker_skin_infos);
            let npc_skin_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.skin_npc_image_view)
                .sampler(self.texture_sampler);
            let npc_skin_infos = [npc_skin_info];
            let npc_skin_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(8)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&npc_skin_infos);
            // 地面微细节层（binding 9）：**必须写**，片元无条件采样它。
            // 采样器字段对 SAMPLED_IMAGE 写入无意义（真正的采样器走 binding 3 那条
            // SAMPLER 写入），与其它贴图保持一致填 texture_sampler。
            let ground_detail_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.ground_detail_image_view)
                .sampler(self.texture_sampler);
            let ground_detail_infos = [ground_detail_info];
            let ground_detail_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(GROUND_DETAIL_BINDING)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&ground_detail_infos);

            let writes = [
                sampled_image_write,
                sampler_write,
                shadow_map_write,
                shadow_sampler_write,
                marker_skin_write,
                npc_skin_write,
                ground_detail_write,
            ];
            unsafe {
                self.device.update_descriptor_sets(&writes, &[]);
            }
        }
        Ok(())
    }

    /// PT 覆盖后重绘 HUD 的 overlay pass（load=LOAD 保留 PT 画面！2026-09-01）
    ///
    /// 🔴 **2026-09-15 修了三处**（都由验证层在 PT 打开时抓出，见未结案 #2 的后续）：
    /// ① `initialLayout` 曾是 `PRESENT_SRC_KHR`，而调用方在 `cmd_begin_render_pass` **之前**
    ///    已经把图手动转成了 `COLOR_ATTACHMENT_OPTIMAL` ⇒
    ///    `VUID-vkCmdBeginRenderPass-initialLayout-00900`（"初始布局必须等于当前布局"）。
    ///    现在声明成 `COLOR_ATTACHMENT_OPTIMAL`，与那次手动 barrier 对齐；
    ///    `finalLayout` 仍是 `PRESENT_SRC_KHR`，由 render pass 自己做最后那次转换。
    /// ② HUD 覆盖层以前**复用主 pass 的 `hud_pipeline`**（那是 MSAA 4x + 带深度附件的管线），
    ///    而 overlay pass 是 1 采样、无深度 ⇒ `VUID-vkCmdDraw-renderPass-02684`（管线与当前
    ///    render pass 不兼容 = UB）。现在单独建一条 `hud_overlay_pipeline`。
    /// ③ 收尾那次 `COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR` 的 barrier 是多余的，
    ///    而且与 render pass 的 `finalLayout` 撞车 ⇒ `VUID-VkImageMemoryBarrier-oldLayout-01197`。
    ///    已删除（转换由 render pass 负责）。
    pub fn init_hud_overlay(&mut self) -> Result<(), String> {
        unsafe {
            let color_attachment = vk::AttachmentDescription::default()
                .format(self.swapchain_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
            let color_refs = [vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
            let subpass = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color_refs);
            let attachments = [color_attachment];
            let subpasses = [subpass];
            let rp_info = vk::RenderPassCreateInfo::default()
                .attachments(&attachments)
                .subpasses(&subpasses);
            self.hud_render_pass = self.device.create_render_pass(&rp_info, None)
                .map_err(|e| format!("hud rp: {e}"))?;
        }
        self.create_hud_overlay_pipeline()?;
        self.recreate_hud_framebuffers()
    }

    /// 建 **HUD overlay 专用**图形管线：与 `hud_pipeline` 同着色器/同顶点格式/同混合，
    /// 但 `render_pass = hud_render_pass`、**1 采样、无深度附件** —— 渲染状态必须与 render pass
    /// 逐项兼容，复用主 pass 的管线就是 `VUID-vkCmdDraw-renderPass-02684`（UB）。
    /// 只借用 `hud_pipeline_layout`（同一套着色器 ⇒ 同一套布局，空描述符集 + 空 push constant）。
    fn create_hud_overlay_pipeline(&mut self) -> Result<(), String> {
        let vs_spirv = load_spirv("assets/hud.vert.spv")?;
        let fs_spirv = load_spirv("assets/hud.frag.spv")?;
        let vs_module = self.create_shader_module(&vs_spirv)?;
        let fs_module = self.create_shader_module(&fs_spirv)?;
        let vs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs_module)
            .name(c"vs_main");
        let fs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_module)
            .name(c"fs_main");
        let shader_stages = [vs_stage, fs_stage];

        let hud_binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<HudVertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);
        let hud_attributes = [
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(std::mem::size_of::<[f32; 2]>() as u32),
        ];
        let hud_bindings = [hud_binding];
        let hud_vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&hud_bindings)
            .vertex_attribute_descriptions(&hud_attributes);
        let hud_input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);
        let hud_viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.swapchain_extent.width as f32)
            .height(self.swapchain_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let hud_scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(self.swapchain_extent);
        let hud_viewports = [hud_viewport];
        let hud_scissors = [hud_scissor];
        let hud_viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&hud_viewports)
            .scissors(&hud_scissors);
        let hud_rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);
        // 🔴 这一行是本次修法的关键：overlay pass 只有 1 个采样，不是主 pass 的 MSAA 数
        let hud_multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let hud_depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::ALWAYS);
        let hud_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            )
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD);
        let hud_blend_attachments = [hud_blend_attachment];
        let hud_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&hud_blend_attachments);
        let hud_dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let hud_dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&hud_dynamic_states);
        let create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&hud_vertex_input)
            .input_assembly_state(&hud_input_assembly)
            .viewport_state(&hud_viewport_state)
            .rasterization_state(&hud_rasterizer)
            .multisample_state(&hud_multisampling)
            .depth_stencil_state(&hud_depth_stencil)
            .color_blend_state(&hud_blend_state)
            .dynamic_state(&hud_dynamic_state)
            .layout(self.hud_pipeline_layout)
            .render_pass(self.hud_render_pass)
            .subpass(0);
        let result = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[create_info], None)
        };
        unsafe {
            self.device.destroy_shader_module(vs_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }
        self.hud_overlay_pipeline =
            result.map_err(|(_, e)| format!("创建 HUD overlay 管线失败: {}", e))?.remove(0);
        Ok(())
    }

    /// 按**当前** `swapchain_image_views` 重建 HUD overlay 的 framebuffer。
    ///
    /// 🔴 **这就是未结案 #2「PT 一启动即 0xC0000005」的根因（2026-09-15 由验证层抓出）**：
    /// `hud_framebuffers` 原本只在 `init_hud_overlay`（启动时一次）里创建，而
    /// `destroy_swapchain` 会**销毁它依赖的 `swapchain_image_views`** 却不重建它们。
    /// 启动阶段就有 **5 次** swapchain 重建（resize 事件），所以这组 framebuffer 从很早就
    /// 指向**已销毁的 ImageView**；而它唯一的消费者是 **PT 通路**（PT 画完再叠 HUD），
    /// 光栅路径走 `self.framebuffers`（那次是重建过的）——
    /// ⇒ 症状正好是"**光栅一切正常、一开 PT 就崩**"，而且崩因与 PT 本身毫无关系。
    ///
    /// 验证层原话：
    /// ```text
    /// vkCmdBeginRenderPass(): pCreateInfo->pAttachments[0] VkImageView 0x70000000007 is invalid.
    /// VUID-VkRenderPassBeginInfo-framebuffer-parameter
    /// ```
    fn recreate_hud_framebuffers(&mut self) -> Result<(), String> {
        if self.hud_render_pass == vk::RenderPass::null() {
            return Ok(()); // HUD overlay 未启用（无 HUD 管线），无需 framebuffer
        }
        for &framebuffer in &self.hud_framebuffers {
            unsafe { self.device.destroy_framebuffer(framebuffer, None) };
        }
        self.hud_framebuffers = self
            .swapchain_image_views
            .iter()
            .map(|&iv| {
                let fbi = vk::FramebufferCreateInfo::default()
                    .render_pass(self.hud_render_pass)
                    .attachments(std::slice::from_ref(&iv))
                    .width(self.swapchain_extent.width)
                    .height(self.swapchain_extent.height)
                    .layers(1);
                unsafe { self.device.create_framebuffer(&fbi, None) }
                    .map_err(|e| format!("hud fb: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(())
    }

    fn init_framebuffers(&mut self) -> Result<(), String> {
        self.framebuffers = self
            .swapchain_image_views
            .iter()
            .enumerate()
            .map(|(i, &image_view)| {
                // MSAA：attachments = [msaa 颜色, 交换链（resolve 目标）, 深度]；
                // 关闭时 msaa view 即交换链本身（TYPE_1，无独立附件）
                let msaa_view = if self.msaa_samples == vk::SampleCountFlags::TYPE_1 {
                    image_view
                } else {
                    self.msaa_image_views[i]
                };
                let attachments = [msaa_view, image_view, self.depth_image_views[i]];
                let framebuffer_create_info = vk::FramebufferCreateInfo::default()
                    .render_pass(self.render_pass)
                    .attachments(&attachments)
                    .width(self.swapchain_extent.width)
                    .height(self.swapchain_extent.height)
                    .layers(1);
                unsafe {
                    self.device
                        .create_framebuffer(&framebuffer_create_info, None)
                        .map_err(|e| format!("创建帧缓冲失败: {e}"))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(())
    }

    fn init_command_pool(&mut self) -> Result<(), String> {
        let pool_create_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.graphics_queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);

        self.command_pool = unsafe {
            self.device
                .create_command_pool(&pool_create_info, None)
                .map_err(|e| format!("创建命令池失败: {}", e))?
        };
        Ok(())
    }

    fn init_command_buffers(&mut self) -> Result<(), String> {

        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(self.framebuffers.len() as u32);

        self.command_buffers = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("分配命令缓冲失败: {}", e))?
        };

        for (i, &command_buffer) in self.command_buffers.iter().enumerate() {
            self.record_command_buffer(command_buffer, i, INSTANCE_COUNT, 0, TerrainLod::High as usize)?;
        }
        Ok(())
    }

    fn record_command_buffer(
        &self,
        command_buffer: vk::CommandBuffer,
        image_index: usize,
        near_count: u32,
        far_count: u32,
        terrain_lod: usize,
    ) -> Result<(), String> {
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::empty());

        unsafe {
            self.device
                .begin_command_buffer(command_buffer, &begin_info)
                .map_err(|e| format!("开始命令缓冲失败: {}", e))?;
        }

        // ---- 阴影 pass：depth-only 渲光空间深度，供主 pass 3x3 PCF 采样 ----
        // （mesh 路径已冻结，shadow 只服务传统 VERTEX 几何；mesh 模式 near=INSTANCE_COUNT
        //   地面实例静态上传，marker/NPC/自发光照常上传，同一槽位布局可复用）
        if !self.void_mode {
            self.record_shadow_pass(command_buffer, near_count, far_count, terrain_lod)?;
        }

        // clear values 按 attachment 索引寻址：0=MSAA 颜色(CLEAR)、1=resolve(DONT_CARE，
        // 值被忽略但占位保证索引正确)、2=深度(CLEAR)。旧实现只有 2 个元素 → 深度清除值
        // 越界读取 → 深度缓冲未清除（垃圾）→ 深度测试随机失败：地面/障碍大面积消失。
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: if self.void_mode {
                        [1.0, 1.0, 1.0, 1.0] // 检视模式：白色背景，便于对比透视
                    } else {
                        // 白天天空（线性 RGB → sRGB 约浅蓝）；城市地图配套（2026-08-21）
                        [0.24, 0.36, 0.60, 1.0]
                    },
                },
            },
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        ];

        let render_pass_begin_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.render_pass)
            .framebuffer(self.framebuffers[image_index])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.swapchain_extent,
            })
            .clear_values(&clear_values);

        unsafe {
            self.device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_begin_info,
                vk::SubpassContents::INLINE,
            );
        }

        // 动态 viewport/scissor：每帧按当前 swapchain_extent 重设（resize 后自动适配，
        // 2026-08-15 修复全屏/窗口变化后画面卡左上角）
        let vp = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.swapchain_extent.width as f32)
            .height(self.swapchain_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let sc = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(self.swapchain_extent);
        unsafe {
            self.device.cmd_set_viewport(command_buffer, 0, &[vp]);
            self.device.cmd_set_scissor(command_buffer, 0, &[sc]);
        }

        unsafe {
            self.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );
        }

        // ---- 绑定 Descriptor Set ----
        // 本帧写入的 UBO 与 instance buffer 都是 current_frame 对应的 slot，
        // 因此必须绑定 descriptor_sets[current_frame]（image_index 与帧 slot 无关）。
        let descriptor_sets = [self.descriptor_sets[self.current_frame]];
        unsafe {
            self.device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                0,
                &descriptor_sets,
                &[],
            );
        }

        // 地形 draw call（非实例，instance_index = 65536 读保留 identity 实例；
        // 每帧按 LOD 选择绘制 3 级网格之一，mesh.index_count 随密度变化）
        // 虚空检视模式：不绘制地形（仅枪模）
        if !self.void_mode {
        if let Some(mesh) = self.terrain_lods.get(terrain_lod) {
            let terrain_vertex_buffers = [mesh.vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &terrain_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    mesh.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    mesh.index_count,
                    1,
                    0,
                    0,
                    INSTANCE_COUNT,
                );
            }
        }
        }

        if self.mesh_enabled {
            // ---- 网格着色器路径（VK_EXT_mesh_shader）：逐实例 GPU 视锥剔除 + 顶点变换 ----
            // 地面实例场静态一次性上传（槽位 0..INSTANCE_COUNT）；marker/NPC/自发光每帧
            // 顺序上传到各自 BASE 槽位（shader 按距离自选立方体 / 远档十字 quad 几何）。
            let mesh = self
                .mesh_shader
                .as_ref()
                .expect("mesh_enabled=true 但 vkCmdDrawMeshTasksEXT 加载器缺失");
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.mesh_pipeline,
                );
                self.device.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.mesh_pipeline_layout,
                    0,
                    &descriptor_sets,
                    &[],
                );
            }
            if !self.void_mode {
                self.draw_mesh_range(command_buffer, mesh, 0, INSTANCE_COUNT);
            }
            self.draw_mesh_range(
                command_buffer,
                mesh,
                MARKER_SLOT_BASE,
                self.last_marker_near + self.last_marker_far,
            );
            self.draw_mesh_range(
                command_buffer,
                mesh,
                NPC_SLOT_BASE,
                self.last_npc_box_near + self.last_npc_box_far,
            );
            self.draw_mesh_range(
                command_buffer,
                mesh,
                NPC_CYL_SLOT_BASE,
                self.last_npc_cyl_near + self.last_npc_cyl_far,
            );
            self.draw_mesh_range(
                command_buffer,
                mesh,
                NPC_SPH_SLOT_BASE,
                self.last_npc_sph_near + self.last_npc_sph_far,
            );
            self.draw_mesh_range(
                command_buffer,
                mesh,
                EMISSIVE_SLOT_BASE,
                self.last_emissive_near + self.last_emissive_far,
            );
        } else {
        // 近档地面 draw call：平铺 quad 几何（无侧壁），实例区从 0 开始
        if near_count > 0 {
            let vertex_buffers = [self.ground_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.ground_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    GROUND_INDICES.len() as u32,
                    near_count,
                    0,
                    0,
                    0,
                );
            }
        }

        // 远档地面 draw call：同样平铺 quad 几何，实例区偏移 = near_count（[近档][远档] 连续排布）
        if far_count > 0 {
            let far_vertex_buffers = [self.ground_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &far_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.ground_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    GROUND_INDICES.len() as u32,
                    far_count,
                    0,
                    0,
                    near_count,
                );
            }
        }

        // ---- 世界障碍 marker draw（复用同一 pipeline 与几何，实例槽从 MARKER_SLOT_BASE 起）----
        if self.last_marker_near > 0 {
            let marker_vertex_buffers = [self.vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &marker_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    INDICES.len() as u32,
                    self.last_marker_near,
                    0,
                    0,
                    MARKER_SLOT_BASE,
                );
            }
        }
        if self.last_marker_far > 0 {
            let far_vertex_buffers = [self.far_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &far_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.far_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    FAR_INDICES.len() as u32,
                    self.last_marker_far,
                    0,
                    0,
                    MARKER_SLOT_BASE + self.last_marker_near,
                );
            }
        }

        // ---- NPC 士兵段 draw（人体三几何：盒体躯干/圆柱四肢/球体头，各自独立
        //      几何与实例槽区；每区按距离分近档（对应几何）+ 远档（十字 quad））----
        // 盒体区（躯干/脚/枪）
        if self.last_npc_box_near > 0 {
            let npc_vertex_buffers = [self.vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    INDICES.len() as u32,
                    self.last_npc_box_near,
                    0,
                    0,
                    NPC_SLOT_BASE,
                );
            }
        }
        if self.last_npc_box_far > 0 {
            let npc_vertex_buffers = [self.far_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.far_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    FAR_INDICES.len() as u32,
                    self.last_npc_box_far,
                    0,
                    0,
                    NPC_SLOT_BASE + self.last_npc_box_near,
                );
            }
        }
        // 圆柱区（四肢）
        if self.last_npc_cyl_near > 0 {
            let npc_vertex_buffers = [self.cylinder_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.cylinder_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    self.cylinder_index_count,
                    self.last_npc_cyl_near,
                    0,
                    0,
                    NPC_CYL_SLOT_BASE,
                );
            }
        }
        if self.last_npc_cyl_far > 0 {
            let npc_vertex_buffers = [self.far_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.far_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    FAR_INDICES.len() as u32,
                    self.last_npc_cyl_far,
                    0,
                    0,
                    NPC_CYL_SLOT_BASE + self.last_npc_cyl_near,
                );
            }
        }
        // 球体区（头）
        if self.last_npc_sph_near > 0 {
            let npc_vertex_buffers = [self.sphere_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.sphere_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    self.sphere_index_count,
                    self.last_npc_sph_near,
                    0,
                    0,
                    NPC_SPH_SLOT_BASE,
                );
            }
        }
        if self.last_npc_sph_far > 0 {
            let npc_vertex_buffers = [self.far_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.far_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    FAR_INDICES.len() as u32,
                    self.last_npc_sph_far,
                    0,
                    0,
                    NPC_SPH_SLOT_BASE + self.last_npc_sph_near,
                );
            }
        }

        // ---- 自发光实体 draw（爆炸闪光等；复用同一 pipeline，实例槽从 EMISSIVE_SLOT_BASE 起，
        //      shader 对槽位 >= EMISSIVE_INSTANCE_BASE 的实例走自发光直出）----
        // 2026-08-15：改用 UV 球体几何（爆炸球形扩散，不再是一整块立方体）
        if self.last_emissive_near > 0 {
            let emissive_vertex_buffers = [self.sphere_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &emissive_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.sphere_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    self.sphere_index_count,
                    self.last_emissive_near,
                    0,
                    0,
                    EMISSIVE_SLOT_BASE,
                );
            }
        }
        if self.last_emissive_far > 0 {
            let emissive_vertex_buffers = [self.far_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &emissive_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.far_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    FAR_INDICES.len() as u32,
                    self.last_emissive_far,
                    0,
                    0,
                    EMISSIVE_SLOT_BASE + self.last_emissive_near,
                );
            }
        }
        }

        // ---- GLB 道具：按空间分桶逐桶视锥剔除后绘制。
        //      走主管线（**已开深度测试**），所以道具之间、道具与地形之间遮挡正确。
        //      identity 实例取 PROP_INSTANCE_INDEX：位姿已在 CPU 烘进顶点，GPU 侧不需要
        //      逐实例矩阵；该槽 tint.w=Shape::Authored.tag() 让片元跳过程序化立面加工。
        //      分桶动机与实测收益见 `engine::props::merge_binned`（道具曾占整帧约 40%）。
        if self.prop_index_count > 0 && self.prop_vertex_count > 0 && !self.prop_bins.is_empty() {
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline,
                );
                self.device.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline_layout,
                    0,
                    &descriptor_sets,
                    &[],
                );
                let pvb = [self.prop_vertex_buffer];
                let poff = [0u64];
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &pvb, &poff);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.prop_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                // 逐桶球-视锥测试，只发可见桶的那段索引。所有桶共用同一份 VBO/IBO 与
                // 同一个 identity 实例，所以这里只换 firstIndex/indexCount，
                // 不需要任何重新绑定或重传。
                // margin 2m：桶边界上的建筑不该在转视角时逐帧抖动进出。
                // RV3D_ONE_PROP_DRAW=1：A/B（第 43 轮）—— 不分桶，整份索引一次画完。
                // 用来二选一：**1 个 draw 也慢 ⇒ 顶点/片元着色本身贵**；
                // **1 个 draw 快很多 ⇒ 逐 draw 的 state 开销贵**（28 次切换）。
                // 前提：顶点数/三角形数/draw call 数/填充率四个维度都已排除对不上这 3.7ms。
                let one_draw = std::env::var("RV3D_ONE_PROP_DRAW").is_ok();
                let mut drawn_bins = 0u32;
                let mut drawn_tris = 0u32;
                let mut drawn_vert_span = 0u64;
                if one_draw {
                    self.device.cmd_draw_indexed(
                        command_buffer,
                        self.prop_index_count,
                        1,
                        0,
                        0,
                        PROP_INSTANCE_INDEX,
                    );
                    drawn_bins = 1;
                    drawn_tris = self.prop_index_count / 3;
                    drawn_vert_span = self.prop_vertex_count as u64;
                } else {
                    for bin in &self.prop_bins {
                        if !crate::engine::props::bin_visible(bin, &self.frame_frustum, 2.0) {
                            continue;
                        }
                        drawn_bins += 1;
                        drawn_tris += bin.index_count / 3;
                        drawn_vert_span += (bin.max_vertex - bin.min_vertex + 1) as u64;
                        self.device.cmd_draw_indexed(
                            command_buffer,
                            bin.index_count,
                            1,
                            bin.first_index,
                            0,
                            PROP_INSTANCE_INDEX,
                        );
                    }
                }
                // 🪖 士兵 GLB（2026-09-13）：**一次 draw 画完所有实例**。
                //
                // 与道具/箱体的关键差别：这里是**真正的实例化** —— 网格上传一次，
                // 每个 NPC 只占一个实例矩阵。`cmd_draw_indexed` 的实例数（第 2 个参数）
                // 就是为这个准备的，枪模已经在用同一条路（只是它固定传 1）。
                //
                // 管线用 `self.pipeline`：它就是传统 VERTEX 管线（`vs_main`/`fs_main`），
                // `depth_test` 是开的 —— 世界里的士兵必须被墙挡住（枪那条是 OFF，
                // 因为它要恒在 HUD 之上）。这个管线在 mesh 可用时同样被无条件创建，
                // 道具/地面也一直在用它。
                //
                // ⚠️ `soldier_drawn` 由 `upload_soldiers` 写；为 0 时整段跳过，
                // 于是"没上传网格"时行为与改动前**逐字节一致**（NPC 仍只有 18 段箱体）。
                if self.soldier_drawn > 0
                    && self.soldier_index_count > 0
                    && self.soldier_vertex_buffer != vk::Buffer::null()
                {
                    self.device.cmd_bind_pipeline(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.pipeline,
                    );
                    let svb = [self.soldier_vertex_buffer];
                    let soff = [0u64];
                    self.device
                        .cmd_bind_vertex_buffers(command_buffer, 0, &svb, &soff);
                    self.device.cmd_bind_index_buffer(
                        command_buffer,
                        self.soldier_index_buffer,
                        0,
                        vk::IndexType::UINT32,
                    );
                    self.device.cmd_draw_indexed(
                        command_buffer,
                        self.soldier_index_count,
                        self.soldier_drawn,
                        0,
                        0,
                        SOLDIER_INSTANCE_BASE,
                    );
                }
                // 第②条判据（RV3D_PROP_STATS=1）：每帧实际提交了多少桶/三角形。
                // 存在的理由：道具是已知最大单项（关掉道具 fps 68.8→184.7），
                // 但机制一直靠猜。先回答"到底提交了多少"，再看是**剔除粒度**问题
                // （提交量远超屏幕能分辨的量）还是**逐 draw call 开销**（量不大但时间高）。
                {
                    use std::sync::atomic::{AtomicU32, Ordering};
                    static TICK: AtomicU32 = AtomicU32::new(0);
                    if std::env::var("RV3D_PROP_STATS").is_ok()
                        && TICK.fetch_add(1, Ordering::Relaxed) % 120 == 0
                    {
                        let max_bin = self
                            .prop_bins
                            .iter()
                            .map(|b| b.index_count / 3)
                            .max()
                            .unwrap_or(0);
                        log::info!(
                            "propdraw: 桶 {drawn_bins}/{} 可见；提交三角形 {drawn_tris}；单桶最大 {max_bin}；顶点区间合计 {drawn_vert_span}（顶点总数 {}）",
                            self.prop_bins.len(),
                            self.prop_vertex_count
                        );
                    }
                }
            }
        }

        // ---- 第一人称枪模（程序化高模，2026-08-16）：identity 实例（GUN_INSTANCE_INDEX
        //      → inst.model = 单位阵，顶点即世界空间，main.rs 已烘焙 view⁻¹×锚点）。
        //      走 `gun_pipeline`（depth_test=OFF 且不写深度）→ 枪模恒可见、也不会挡住 HUD。
        //      2026-09-04：主管线开了深度测试，枪模若继续共用会被它前面的墙裁掉，
        //      所以这里必须切到独立管线，而不是继续靠主管线的宽松 depth 状态。
        if self.gun_index_count > 0 && self.gun_vertex_count > 0 {
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.gun_pipeline,
                );
                self.device.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline_layout,
                    0,
                    &descriptor_sets,
                    &[],
                );
                let gun_vb = [self.gun_vertex_buffer];
                let gun_off = [0u64];
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &gun_vb, &gun_off);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.gun_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    self.gun_index_count,
                    1,
                    0,
                    0,
                    GUN_INSTANCE_INDEX,
                );
            }
        }

        // ---- HUD 覆盖层：自包含 pipeline 与顶点缓冲，追加在主 pass 末尾 ----
        if self.hud_vertex_count > 0 && self.hud_pipeline != vk::Pipeline::null() {
            let hud_vertex_buffers = [self.hud_vertex_buffer];
            let hud_offsets = [0u64];
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.hud_pipeline,
                );
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &hud_vertex_buffers,
                    &hud_offsets,
                );
                self.device.cmd_draw(command_buffer, self.hud_vertex_count, 1, 0, 0);
            }
        }

        unsafe {
            self.device.cmd_end_render_pass(command_buffer);
        }

        // ===== 2026-08-29 路径追踪全景：全部记录进主命令缓冲（零第二提交/围栏冲突；常驻零分配）=====
        if self.pt_live_enabled && self.pt_resident.is_some() {
            let sw_img = self.swapchain_images[image_index as usize];
            // 与 init_pt_resident 创建的图像同尺寸（硬编码会与新分辨率错配）
            let (pw, ph) = self.pt_size;
            unsafe {
                // 取景/光照变化 => 清空重开累积（不同视角样本混在一起会拖影）
                let sig = self.pt_params.signature();
                // 2026-09-01：sig 已量化 0.5m 位移——只有大于该步幅才重置（指数平均吸收细微移动）
                if sig != self.pt_view_sig.get() {
                    self.pt_view_sig.set(sig);
                    self.pt_reset.set(true);
                    self.pt_frame.set(0);
                }
                let accumulating = self.pt_frame.get() < self.pt_spp_target;
                if accumulating {
                    // 主图像每帧整体重写 => 允许 UNDEFINED 丢弃；累积图像必须 GENERAL->GENERAL 保内容
                    let pt_bar = vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::NONE).dst_access_mask(vk::AccessFlags::SHADER_WRITE)
                        .old_layout(vk::ImageLayout::UNDEFINED).new_layout(vk::ImageLayout::GENERAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(self.pt_img)
                        .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                    let acc_bar = vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                        .old_layout(vk::ImageLayout::GENERAL).new_layout(vk::ImageLayout::GENERAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(self.pt_acc)
                        .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                    self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &[pt_bar, acc_bar]);
                    self.device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, self.pt_pipeline);
                    self.device.cmd_bind_descriptor_sets(command_buffer, vk::PipelineBindPoint::COMPUTE, self.pt_layout, 0, &[self.pt_dset], &[]);
                    let pc = self.pt_params.pack(
                        pw, ph,
                        self.pt_frame.get(),
                        self.pt_reset.get(),
                        self.pt_spp_target,
                        // 运动量：相机位移与朝向变化速度 → 0..1（移动/跳跃 → 高 spp + 短时域）
                        {
                            let c0 = self.pt_move_base_cam.get();
                            let f0 = self.pt_move_base_fwd.get();
                            let c1 = self.pt_params.cam.to_array();
                            let f1 = self.pt_params.fwd.to_array();
                            let mut d = 0.0f32;
                            for k in 0..3 { let dc = c1[k] - c0[k]; d += dc * dc; let df = f1[k] - f0[k]; d += df * df * 36.0; }
                            d = d.sqrt();
                            self.pt_move_base_cam.set(c1);
                            self.pt_move_base_fwd.set(f1);
                            (d * 20.0).min(1.0)
                        },
                        // 🏢 盒体三角形边界（道具路径分流判据，见 pt_panorama.glsl 的 pc.g）
                        (self.pt_box_count * 12) as u32,
                    );
                    self.pt_reset.set(false);
                    self.device.cmd_push_constants(
                        command_buffer,
                        self.pt_layout,
                        vk::ShaderStageFlags::COMPUTE,
                        0,
                        bytemuck_bytes(&pc),
                    );
                    self.device.cmd_dispatch(command_buffer, (pw + 7) / 8, (ph + 7) / 8, 1);
                    self.pt_frame.set(self.pt_frame.get() + 1);
                }
                // PT 写完成 -> Transfer 读
                let pt_bar2 = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE).dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::GENERAL).new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.pt_img)
                    .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[pt_bar2]);
                // swapchain -> TRANSFER_DST
                let sw_bar = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::NONE).dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED).new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(sw_img)
                    .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[sw_bar]);
                // blit PT -> swapchain
                let blit = vk::ImageBlit::default()
                    .src_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                    .src_offsets([vk::Offset3D { x: 0, y: 0, z: 0 }, vk::Offset3D { x: pw as i32, y: ph as i32, z: 1 }])
                    .dst_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                    .dst_offsets([vk::Offset3D { x: 0, y: 0, z: 0 }, vk::Offset3D { x: 2560, y: 1600, z: 1 }]);
                self.device.cmd_blit_image(command_buffer, self.pt_img, vk::ImageLayout::GENERAL, sw_img, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[blit], vk::Filter::NEAREST);
                // swapchain -> PRESENT_SRC
                let sw_back = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE).dst_access_mask(vk::AccessFlags::MEMORY_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL).new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(sw_img)
                    .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[sw_back]);
                // 2026-09-01：HUD/UI 重绘在 PT 之上（load=LOAD 保留 PT 画面！）
                if self.hud_render_pass != vk::RenderPass::null() && self.hud_vertex_count > 0 {
                    let hud_bar = vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::MEMORY_READ).dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                        .old_layout(vk::ImageLayout::PRESENT_SRC_KHR).new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(sw_img)
                        .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                    self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT, vk::DependencyFlags::empty(), &[], &[], &[hud_bar]);
                    self.device.cmd_begin_render_pass(command_buffer, &vk::RenderPassBeginInfo::default()
                        .render_pass(self.hud_render_pass)
                        .framebuffer(self.hud_framebuffers[image_index as usize])
                        .render_area(vk::Rect2D { offset: vk::Offset2D { x: 0, y: 0 }, extent: self.swapchain_extent }),
                        vk::SubpassContents::INLINE);
                    // ⚠ 必须绑 **overlay 专用**管线：overlay pass 是 1 采样、无深度，
                    // 而 `hud_pipeline` 是给主 pass（MSAA + 深度）建的 —— 绑错就是
                    // `VUID-vkCmdDraw-renderPass-02684`（管线与 render pass 不兼容 = UB）
                    self.device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, self.hud_overlay_pipeline);
                    let vb = [self.hud_vertex_buffer];
                    let offs = [0u64];
                    self.device.cmd_bind_vertex_buffers(command_buffer, 0, &vb, &offs);
                    self.device.cmd_draw(command_buffer, self.hud_vertex_count, 1, 0, 0);
                    self.device.cmd_end_render_pass(command_buffer);
                    // ⚠ 这里**不再**补 `COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR` 的 barrier：
                    // render pass 的 `finalLayout` 已经做了那次转换，再补一条就是"从已经变成
                    // PRESENT_SRC 的图再转一次 COLOR_ATTACHMENT_OPTIMAL" ⇒
                    // `VUID-VkImageMemoryBarrier-oldLayout-01197`（2026-09-15 删掉的就是它）。
                }
            }
        }

        unsafe {
            self.device
                .end_command_buffer(command_buffer)
                .map_err(|e| format!("结束命令缓冲失败: {}", e))?;
        }
        Ok(())
    }

    /// 记录阴影 depth-only pass：布局转换（UNDEFINED → DEPTH_STENCIL_ATTACHMENT_OPTIMAL）
    /// → 渲几何到 2048x2048 阴影图 →（DEPTH_STENCIL_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL）。
    /// 绘制几何与主 pass 传统路径一致：地形 + 地面实例场 + marker + NPC + 自发光。
    fn record_shadow_pass(
        &self,
        command_buffer: vk::CommandBuffer,
        near_count: u32,
        far_count: u32,
        terrain_lod: usize,
    ) -> Result<(), String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;

        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::DEPTH)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);

        // UNDEFINED → DEPTH_STENCIL_ATTACHMENT_OPTIMAL（内容作废，反正 render pass 会 CLEAR）
        let to_attachment = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.shadow_image)
            .subresource_range(subresource)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE);
        let to_attachment_barriers = [to_attachment];
        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_attachment_barriers,
            );
        }

        // ---- shadow render pass：绑 shadow pipeline + shadow descriptor set ----
        let clear_depth = [vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        }];
        let shadow_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.shadow_render_pass)
            .framebuffer(self.shadow_framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: SHADOW_MAP_SIZE,
                    height: SHADOW_MAP_SIZE,
                },
            })
            .clear_values(&clear_depth);
        unsafe {
            self.device.cmd_begin_render_pass(
                command_buffer,
                &shadow_pass_info,
                vk::SubpassContents::INLINE,
            );
            self.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.shadow_pipeline,
            );
            let shadow_sets = [self.shadow_descriptor_sets[self.current_frame]];
            self.device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.shadow_pipeline_layout,
                0,
                &shadow_sets,
                &[],
            );
        }

        // 地形（保留 identity 实例 = INSTANCE_COUNT，与主 pass 一致）
        if let Some(mesh) = self.terrain_lods.get(terrain_lod) {
            let terrain_vertex_buffers = [mesh.vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &terrain_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    mesh.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    mesh.index_count,
                    1,
                    0,
                    0,
                    INSTANCE_COUNT,
                );
            }
        }

        // 地面实例场（近/远档，与主 pass 同一槽位布局）
        self.draw_shadow_range(
            command_buffer,
            self.ground_vertex_buffer,
            self.ground_index_buffer,
            GROUND_INDICES.len() as u32,
            near_count,
            0,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.ground_vertex_buffer,
            self.ground_index_buffer,
            GROUND_INDICES.len() as u32,
            far_count,
            near_count,
        )?;
        // marker
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            self.last_marker_near,
            MARKER_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            self.last_marker_far,
            MARKER_SLOT_BASE + self.last_marker_near,
        )?;
        // NPC 盒体区（躯干/脚/枪；阴影以盒体近似）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            self.last_npc_box_near,
            NPC_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            self.last_npc_box_far,
            NPC_SLOT_BASE + self.last_npc_box_near,
        )?;
        // NPC 圆柱区（四肢；阴影以盒体近似）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            self.last_npc_cyl_near,
            NPC_CYL_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            self.last_npc_cyl_far,
            NPC_CYL_SLOT_BASE + self.last_npc_cyl_near,
        )?;
        // NPC 球体区（头；阴影以盒体近似）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            self.last_npc_sph_near,
            NPC_SPH_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            self.last_npc_sph_far,
            NPC_SPH_SLOT_BASE + self.last_npc_sph_near,
        )?;
        // 🪖 士兵 GLB（2026-09-14 补）——**这条是补我自己的回归**。
        //
        // 上面那三对 `npc_box/cyl/sph` 是 18 段箱体的阴影近似。而 `set_npc_visuals`
        // 在 `soldier_on` 时**不再生成任何箱体段** ⇒ 那三对的实例数全是 0 ⇒
        // **士兵一度完全不投影**，而阴影 pass 不报任何错、画面上只是"人浮在地上"。
        //
        // 实例矩阵不用重算：`upload_soldiers` 已经把 N 个根变换写进
        // `SOLDIER_INSTANCE_BASE` 起的槽位，阴影 pass 只要用同一段槽位再画一遍即可。
        // `soldier_drawn` 为 0（网格没上传）时 `draw_shadow_range` 自己会早退。
        self.draw_shadow_range(
            command_buffer,
            self.soldier_vertex_buffer,
            self.soldier_index_buffer,
            self.soldier_index_count,
            self.soldier_drawn,
            SOLDIER_INSTANCE_BASE,
        )?;
        // 🌳 道具（2026-09-14 补）—— 此前**道具完全不投影**（未结案 #14 定案）。
        //
        // 树、楼、沙袋这些本来是场景里体积最大的一批几何，没有影子会让"东西贴在地上"
        // 这件事失去线索。主 pass 的 bin 循环就在 `record_command_buffer` 里，这里复刻它。
        //
        // ⚠️ **刻意不做视锥剔除。** 主 pass 用的是 `bin_visible(bin, &self.frame_frustum, …)`
        // ——那是**相机**视锥；而阴影 pass 覆盖的是**光源**视锥，两者是不同的体积。
        // 照抄那行会把"相机看不见、但在阴影图里"的道具剔掉 ⇒ **影子缺一块**，
        // 而且缺的位置随视角移动，是最难查的那类伪影。
        // 代价是多几十次 `cmd_draw_indexed`（全城约 9×9 桶），远低于一次剔除错判的代价。
        // ⚠️ **剔除必须用光源视锥，不能用相机视锥。** 主 pass 那行用的是
        // `bin_visible(bin, &self.frame_frustum, …)` —— 那是**相机**视锥；阴影 pass 覆盖的是
        // **光源**视锥，两者是不同的体积。照抄相机会把"相机看不见、但在阴影图里"的道具剔掉
        // ⇒ 影子缺一块，而且缺的位置随视角移动（最难查的那类伪影）。
        //
        // 但也不能不剔除：全画 81 个桶实测把帧率从约 250 压到 134。
        // 正解是**用光源自己的视锥剔**（`light_view_proj` 的 6 个平面），
        // 于是"正确"和"便宜"同时成立。margin 给 2m，与主 pass 同档。
        // 🏢 阴影建筑 LOD（2026-09-19 专项）：优先盒壳几何；未建成或
        // RV3D_SHADOW_LOD=0 时退回全量——退化方向是"多画三角形"，永不缺阴影。
        let use_lod = self.shadow_lod
            && self.prop_sh_index_count > 0
            && !self.prop_sh_bins.is_empty()
            && self.prop_sh_vertex_buffer != vk::Buffer::null()
            && self.prop_sh_index_buffer != vk::Buffer::null();
        let (sh_vb, sh_ib, sh_bins): (vk::Buffer, vk::Buffer, &[crate::engine::props::PropBin]) =
            if use_lod {
                (
                    self.prop_sh_vertex_buffer,
                    self.prop_sh_index_buffer,
                    &self.prop_sh_bins,
                )
            } else if self.prop_vertex_buffer != vk::Buffer::null()
                && self.prop_index_buffer != vk::Buffer::null()
            {
                (self.prop_vertex_buffer, self.prop_index_buffer, &self.prop_bins)
            } else {
                (vk::Buffer::null(), vk::Buffer::null(), &[])
            };
        if sh_ib != vk::Buffer::null() {
            let light_frustum =
                Self::extract_frustum_planes_from(self.light_data.shadow.light_view_proj);
            let bind_vb = [sh_vb];
            let prop_off = [0u64];
            unsafe {
                self.device
                    .cmd_bind_vertex_buffers(command_buffer, 0, &bind_vb, &prop_off);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    sh_ib,
                    0,
                    vk::IndexType::UINT32,
                );
            }
            for bin in sh_bins {
                if bin.index_count == 0 {
                    continue;
                }
                if !crate::engine::props::bin_visible(bin, &light_frustum, 2.0) {
                    continue;
                }
                unsafe {
                    self.device.cmd_draw_indexed(
                        command_buffer,
                        bin.index_count,
                        1,
                        bin.first_index,
                        0,
                        PROP_INSTANCE_INDEX,
                    );
                }
            }
        }
        // 自发光（爆炸闪光等）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            self.last_emissive_near,
            EMISSIVE_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            self.last_emissive_far,
            EMISSIVE_SLOT_BASE + self.last_emissive_near,
        )?;

        unsafe {
            self.device.cmd_end_render_pass(command_buffer);
        }

        // DEPTH_STENCIL_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL（主 pass 采样）
        let to_read = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.shadow_image)
            .subresource_range(subresource)
            .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);
        let to_read_barriers = [to_read];
        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                    | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_read_barriers,
            );
        }
        Ok(())
    }

    /// shadow pass 内的一次实例区 draw（bind 顶点/索引缓冲 + draw_indexed）。
    fn draw_shadow_range(
        &self,
        command_buffer: vk::CommandBuffer,
        vertex_buffer: vk::Buffer,
        index_buffer: vk::Buffer,
        index_count: u32,
        instance_count: u32,
        first_instance: u32,
    ) -> Result<(), String> {
        if instance_count == 0 {
            return Ok(());
        }
        let vertex_buffers = [vertex_buffer];
        let offsets = [0u64];
        unsafe {
            self.device.cmd_bind_vertex_buffers(
                command_buffer,
                0,
                &vertex_buffers,
                &offsets,
            );
            self.device.cmd_bind_index_buffer(
                command_buffer,
                index_buffer,
                0,
                vk::IndexType::UINT32,
            );
            self.device.cmd_draw_indexed(
                command_buffer,
                index_count,
                instance_count,
                0,
                0,
                first_instance,
            );
        }
        Ok(())
    }

    /// mesh 路径单次 draw：写入 base_slot push constant 后调用 vkCmdDrawMeshTasksEXT。
    /// count=0 直接返回（Vulkan 允许 group_count=0，这里避免无意义调用）。
    fn draw_mesh_range(
        &self,
        command_buffer: vk::CommandBuffer,
        mesh: &MeshShaderDevice,
        base_slot: u32,
        count: u32,
    ) {
        if count == 0 {
            return;
        }
        // push constant = (base_slot + chunk_start, 0, 0, 0)：
        // workgroup_id.x 从 0 起，槽位 = base + wg.x；每次下发不超过 maxMeshWorkGroupCount[0]。
        let chunk = self.mesh_max_wg_x.max(1);
        let mut drawn = 0u32;
        while drawn < count {
            let n = (count - drawn).min(chunk);
            let push: [u32; 4] = [base_slot + drawn, 0, 0, 0];
            let push_bytes = unsafe {
                std::slice::from_raw_parts(
                    push.as_ptr() as *const u8,
                    std::mem::size_of::<[u32; 4]>(),
                )
            };
            unsafe {
                self.device.cmd_push_constants(
                    command_buffer,
                    self.mesh_pipeline_layout,
                    vk::ShaderStageFlags::MESH_EXT,
                    0,
                    push_bytes,
                );
                mesh.cmd_draw_mesh_tasks(command_buffer, n, 1, 1);
            }
            drawn += n;
        }
    }

    fn init_sync_objects(&mut self) -> Result<(), String> {
        let semaphore_create_info = vk::SemaphoreCreateInfo::default();
        let fence_create_info =
            vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

        // image-available 与 fence **按在飞帧**分配：围栏在每帧开头就被等到，
        // 所以轮到同一个槽位时，上一次等待它的 submit 必然已经完成 ⇒ 复用合法。
        for _ in 0..self.max_frames_in_flight {
            let image_available = unsafe {
                self.device
                    .create_semaphore(&semaphore_create_info, None)
                    .map_err(|e| format!("创建信号量失败: {}", e))?
            };
            let fence = unsafe {
                self.device
                    .create_fence(&fence_create_info, None)
                    .map_err(|e| format!("创建围栏失败: {}", e))?
            };
            self.image_available_semaphores.push(image_available);
            self.in_flight_fences.push(fence);
        }
        // render-finished **按交换链图像**分配（理由见该函数文档）
        self.resize_render_finished_semaphores()?;
        Ok(())
    }

    /// 按**当前交换链图像数**重排 render-finished 信号量。
    ///
    /// ## 为什么不能按「在飞帧」分配（2026-09-15 由验证层抓出）
    ///
    /// 原来是 `render_finished_semaphores[current_frame]`（在飞帧 = 2），而交换链有 **3** 张图像。
    /// 打开验证层（`RV3D_VALIDATION=1`，本轮 mesh.spv 修好后才第一次真跑起来）立刻报：
    ///
    /// ```text
    /// vkQueueSubmit(): pSubmits[0].pSignalSemaphores[0] (VkSemaphore 0x910000000091) is being
    /// signaled by VkQueue ..., but it may still be in use by VkSwapchainKHR ...
    /// Most recently acquired image indices: [0], 1, 2.
    /// (Brackets mark the last use of VkSemaphore ... in a presentation operation.)
    /// VUID-vkQueueSubmit-pSignalSemaphores-00067
    /// ```
    ///
    /// 方括号标出那个信号量最后是被**图像 0** 的 present 用掉的 ——
    /// `vkQueuePresentKHR` **不保证**在 `vkQueueSubmit` 返回时就已经消费掉等待的信号量，
    /// 于是在飞帧轮回到同一槽位时会**重复 signal 一个仍被 present 持有的二值信号量**。
    ///
    /// 改成「每张交换链图像一个」之后契约才成立：`vkAcquireNextImageKHR` 返回图像 i
    /// **本身就保证**图像 i 不再被使用（那次 present 已执行完并释放它），
    /// 所以此刻重新 signal `render_finished[i]` 是合法的。
    ///
    /// ⚠ 调用前必须保证**设备已空闲**（`recreate_swapchain` 开头就 `wait_idle()`）：
    /// 销毁可能仍在被 pending present 等待的信号量同样是未定义行为。
    fn resize_render_finished_semaphores(&mut self) -> Result<(), String> {
        let want = self.swapchain_images.len();
        if self.render_finished_semaphores.len() == want {
            // 图像数没变：`recreate_swapchain` 已经 `wait_idle()`，旧信号量必然已无主，直接复用
            return Ok(());
        }
        let semaphore_create_info = vk::SemaphoreCreateInfo::default();
        for semaphore in std::mem::take(&mut self.render_finished_semaphores) {
            unsafe { self.device.destroy_semaphore(semaphore, None) };
        }
        for _ in 0..want {
            let semaphore = unsafe {
                self.device
                    .create_semaphore(&semaphore_create_info, None)
                    .map_err(|e| format!("创建 render-finished 信号量失败: {}", e))?
            };
            self.render_finished_semaphores.push(semaphore);
        }
        log::info!(
            "render-finished 信号量重排为 {} 个（= 交换链图像数，不再跟在飞帧数 {} 走）",
            self.render_finished_semaphores.len(),
            self.max_frames_in_flight
        );
        Ok(())
    }

    // ============================================================
    // 画质预设 / PNG 截图（公开 API）
    // ============================================================

    /// 设置画质预设（纯 CPU 侧参数：地形 LOD 切换距离 + 实例近/远档分界距离等，
    /// 不触碰 pipeline/shader/swapchain 创建路径）。由外部（main.rs）按需调用。
    #[allow(dead_code)]
    pub fn set_quality(&mut self, preset: QualityPreset) {
        self.quality = preset;
        log::info!("画质预设已切换: {}", preset.label());
    }

    /// 当前画质预设
    pub fn quality(&self) -> QualityPreset {
        self.quality
    }

    /// 请求截图：置 pending 标记，本帧渲染完成后读回 swapchain 图像并保存 PNG。
    /// 支持 B8G8R8A8 / R8G8B8A8 的 UNORM/SRGB 像素格式；一切失败返回 Err（不 panic）。
    #[allow(dead_code)]
    pub fn capture_screenshot(&mut self, path: &std::path::Path) -> Result<(), String> {
        if self.screenshot_buffers.is_empty() {
            self.init_screenshot_resources()?;
        }
        self.screenshot_request = Some(path.to_path_buf());
        Ok(())
    }

    // ============================================================
    // 渲染循环
    // ============================================================

    pub fn render(&mut self, view: glam::Mat4, proj: glam::Mat4) -> Result<(), String> {
        // GPU 侧已被判定卡死（连续 N 次围栏超时）：不再等待/提交/呈现 —— 否则每帧都要
        // 卡满 5 秒超时，主循环形同僵死。保持响应、把结论留在日志里，交给上层决定。
        if self.gpu_stalled {
            return Ok(());
        }
        let frame_start = Instant::now();
        let fence = self.in_flight_fences[self.current_frame];
        let t0 = Instant::now();
        match unsafe {
            // 🔴 超时有限（5s）：`u64::MAX` 会让"GPU 侧再也不会 signal"变成**静默死锁**
            // （日志停住、无 panic、无 VUID）。超时给出可诊断的错误。
            self.device
                .wait_for_fences(&[fence], true, FENCE_WAIT_TIMEOUT_NS)
        } {
            Ok(()) => {
                self.fence_timeouts = 0;
            }
            Err(e) => {
                self.fence_timeouts += 1;
                let n = self.fence_timeouts;
                if n == 1 {
                    log::error!(
                        "等待围栏超时（{:.0}s 内这一帧没完成）—— GPU 侧没有 signal；\
                         旧写法在这里用 u64::MAX 无限等 ⇒ 整个进程静默卡死",
                        FENCE_WAIT_TIMEOUT_NS as f32 / 1e9
                    );
                }
                if fence_stall_due(n) {
                    self.gpu_stalled = true;
                    log::error!(
                        "连续 {} 次围栏超时（≈{:.0}s 无任何一帧完成）⇒ 判定 GPU 侧卡死：\
                         后续帧不再等待/提交/呈现（画面会静止，但进程与输入保持响应）",
                        n,
                        n as f32 * FENCE_WAIT_TIMEOUT_NS as f32 / 1e9
                    );
                }
                return Err(format!("等待围栏失败: {}", e));
            }
        }
        self.stage_wait_fence_us = t0.elapsed().as_micros() as u64;

        let t0 = Instant::now();
        // 有限超时 + 分类重试（判据见 `classify_acquire_err`）：呈现引擎不给图像时
        // 不能无限等 —— 先记日志，再降级到 mailbox 重建，最后才报错交出这一帧。
        let (image_index, suboptimal) = loop {
            let r = unsafe {
                self.swapchain_loader.acquire_next_image(
                    self.swapchain,
                    ACQUIRE_TIMEOUT_NS,
                    self.image_available_semaphores[self.current_frame],
                    vk::Fence::null(),
                )
            };
            match r {
                Ok(v) => break v,
                Err(e) => match classify_acquire_err(e) {
                    AcquireOutcome::Retry => {
                        self.acquire_timeouts += 1;
                        let n = self.acquire_timeouts;
                        if n == 1 || n % 5 == 0 {
                            log::warn!(
                                "获取交换链图像超时（已连续 {} 次，累计 {:.1}s）—— 呈现引擎没有交出图像；\
                                 隐藏/被遮挡的窗口 + IMMEDIATE 是已知诱因",
                                n,
                                t0.elapsed().as_secs_f32()
                            );
                        }
                        if n >= ACQUIRE_STALL_FALLBACK
                            && self.present_mode_override != Some(vk::PresentModeKHR::MAILBOX)
                        {
                            // 自动恢复：换 mailbox 重建交换链（mailbox 会丢弃待呈现图像，
                            // 不会像 IMMEDIATE 那样把图像全留在呈现引擎手里）
                            log::error!(
                                "连续 {} 次拿不到交换链图像 ⇒ 把呈现模式降级为 MAILBOX 并重建交换链",
                                n
                            );
                            self.present_mode_override = Some(vk::PresentModeKHR::MAILBOX);
                            return Err("交换链过期".to_string());
                        }
                        if n >= ACQUIRE_STALL_MAX {
                            log::error!(
                                "连续 {} 次拿不到交换链图像（{:.1}s）—— 放弃这一帧",
                                n,
                                t0.elapsed().as_secs_f32()
                            );
                            return Err(format!("交换链长时间无可用图像（连续 {} 次超时）", n));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    }
                    AcquireOutcome::RecreateSwapchain => {
                        log::warn!("获取交换链图像返回 {:?}，重建交换链...", e);
                        return Err("交换链过期".to_string());
                    }
                    AcquireOutcome::Failed => {
                        return Err(format!("获取交换链图像失败: {:?}", e));
                    }
                },
            }
        };
        self.acquire_timeouts = 0;
        self.stage_acquire_us = t0.elapsed().as_micros() as u64;

        if suboptimal {
            log::warn!("交换链 SUBOPTIMAL，重建...");
            return Err("交换链过期".to_string());
        }

        unsafe {
            self.device
                .reset_fences(&[fence])
                .map_err(|e| format!("重置围栏失败: {}", e))?;
        }

        // ---- 每帧视锥剔除：可见实例压缩上传到当前帧 slot 的 HOST_VISIBLE buffer ----
        // 相机世界位置（view 为刚体变换，其逆矩阵的平移列即相机坐标），每帧只算一次
        let cam_pos = view.inverse().w_axis.truncate();
        let cull_start = Instant::now();
        let (near_count, far_count) = if self.void_mode {
            // 虚空检视模式：跳过世界几何剔除/上传（仅枪模）
            (0, 0)
        } else if self.mesh_enabled {
            // mesh 路径：地面实例场静态一次性上传（见 create_instance_buffer），
            // 完全跳过 CPU SIMD 剔除/压缩——剔除与顶点变换全部移到 GPU mesh shader。
            // 性能日志 visible 语义 = 已上传槽位数（INSTANCE_COUNT）。
            (INSTANCE_COUNT, 0)
        } else {
            self.cull_and_upload(view, proj, cam_pos)
        };
        // ---- 世界障碍 marker：独立槽位上传（见 MARKER_SLOT_BASE），计数供 draw call 使用 ----
        // RV3D_NO_MARKERS=1：A/B 用 —— 跳过**障碍标记实例**（`marker=` 那个计数）。
        // 与 `RV3D_NO_TERRAIN_FIELD` / `RV3D_NO_PROPS` 同类的对照开关：
        // 第 37 轮量出道具 3.2ms + 地形场 0.87ms 只占 9.44ms 帧的 43%，
        // 剩下的未知要靠逐个开关消掉，而不是靠猜。
        let (marker_near, marker_far) = if self.void_mode
            || std::env::var("RV3D_NO_MARKERS").is_ok()
        {
            (0, 0)
        } else {
            self.upload_markers(cam_pos)
        };
        self.last_marker_near = marker_near;
        self.last_marker_far = marker_far;
        // ---- NPC 士兵段：独立槽位上传（见 NPC_SLOT_BASE），计数供 draw call 使用 ----
        let ((box_near, box_far), (cyl_near, cyl_far), (sph_near, sph_far)) = if self.void_mode {
            ((0, 0), (0, 0), (0, 0))
        } else {
            self.upload_npcs(cam_pos)
        };
        self.last_npc_box_near = box_near;
        self.last_npc_box_far = box_far;
        self.last_npc_cyl_near = cyl_near;
        self.last_npc_cyl_far = cyl_far;
        self.last_npc_sph_near = sph_near;
        self.last_npc_sph_far = sph_far;
        // 🪖 士兵 GLB 实例上传（在 NPC 实例同一批里做完，避免多开一次遍历）
        //
        // 🔴 2026-09-13：**顺便打一个无歧义的计数**（`RV3D_SOLDIER_STATS=1` 才开，默认静默）。
        //
        // 存在的理由：我今晚**两次**用没验证过的指标下结论 —— 一次是单次 A/B（方差比效应大），
        // 一次是把 HUD 的 `npc: I{} P{} C{} A{}` 读成 `npc={}`（读丢了 `I`，那其实是
        // Idle 人数，与渲染毫无关系）。两次都是"先有结论、再找一个看起来支持它的数字"。
        //
        // ⇒ 这个计数**没有歧义**：左边是真正提交的 GLB 士兵实例数，右边是箱体实例数。
        //   GLB 生效时右边应当是 **0**（活体每人 1 个实例、尸体也是 1 个），
        //   所以"右边不为 0"就是回退路径被走到的**直接证据**，不必再靠别的字段推断。
        let soldiers = self.upload_soldiers();
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static TICK: AtomicU32 = AtomicU32::new(0);
            if std::env::var("RV3D_SOLDIER_STATS").is_ok()
                && TICK.fetch_add(1, Ordering::Relaxed) % 120 == 0
            {
                log::info!(
                    "soldier-stats: GLB 实例 {} / 箱体 {}（活体+尸体；GLB 生效时箱体应为 0）",
                    soldiers,
                    self.last_npc_box_near + self.last_npc_box_far
                );
            }
        }
        // ---- 自发光实体（爆炸闪光等）：独立槽位上传（见 EMISSIVE_SLOT_BASE）----
        let (emissive_near, emissive_far) = if self.void_mode { (0, 0) } else { self.upload_emissive(cam_pos) };
        self.last_emissive_near = emissive_near;
        self.last_emissive_far = emissive_far;
        let cull_us = cull_start.elapsed().as_micros() as u64;
        self.last_cull_us = cull_us;
        // RV3D_NO_TERRAIN_FIELD=1：A/B 用 —— 跳过**地形实例场**（近档 + 远档）的绘制。
        //
        // 目的（第 37 轮）：日志里 `visible=65536/65536 near=65536 far=0` 说明
        // 65,536 个地形实例**每帧全部进管线、一个都没被剔除**。与其继续猜 near/far
        // 的语义，不如直接量它值多少毫秒 —— 清零这两个计数即跳过对应 draw call。
        // 与 `RV3D_NO_PROPS` / `RV3D_NO_SHADOW` 同类的对照开关。
        let (near_count, far_count) = if std::env::var("RV3D_NO_TERRAIN_FIELD").is_ok() {
            (0, 0)
        } else {
            (near_count, far_count)
        };
        self.last_near_count = near_count;
        self.last_far_count = far_count;

        // ---- 地形网格 LOD：按相机到地形中心地面距离选级，过渡带内 morph 高度 ----
        let terrain_dist = (cam_pos.x * cam_pos.x + cam_pos.z * cam_pos.z).sqrt();
        let quality = quality_params(self.quality);
        let (terrain_lod, terrain_blend) = terrain_lod_blend_with_params(terrain_dist, quality);
        self.last_terrain_lod_name = terrain_lod.name();
        let t0 = Instant::now();
        if !self.void_mode {
            self.update_terrain_lod_morph(terrain_lod, terrain_blend);
        }
        let terrain_lod_index = if self.void_mode { 0 } else { terrain_lod as usize };
        self.stage_terrain_us = t0.elapsed().as_micros() as u64;

        // ---- 性能日志（1 次/秒）：visible / cull_us / fps ----
        self.frame_count += 1;
        if self.last_perf_log.elapsed().as_secs_f32() >= 1.0 {
            let window_secs = self.perf_window_start.elapsed().as_secs_f32();
            let fps = if window_secs > 0.0 {
                self.frame_count as f32 / window_secs
            } else {
                0.0
            };
            log::info!(
                "visible={}/{} near={} far={} fps={:.1} frame_us={} cull_us={} terrain_us={} wait_fence_us={} acquire_us={} record_us={} submit_us={} present_us={} terrain_lod={} blend={:.3} quality={} marker={} npc={}",
                near_count + far_count,
                INSTANCE_COUNT,
                near_count,
                far_count,
                fps,
                self.last_frame_us,
                cull_us,
                self.stage_terrain_us,
                self.stage_wait_fence_us,
                self.stage_acquire_us,
                self.stage_record_us,
                self.stage_submit_us,
                self.stage_present_us,
                terrain_lod.name(),
                terrain_blend,
                self.quality().label(),
                self.last_marker_near
                    + self.last_marker_far
                    + self.last_emissive_near
                    + self.last_emissive_far,
                self.last_npc_box_near
                    + self.last_npc_box_far
                    + self.last_npc_cyl_near
                    + self.last_npc_cyl_far
                    + self.last_npc_sph_near
                    + self.last_npc_sph_far
            );
            self.frame_count = 0;
            self.perf_window_start = Instant::now();
            self.last_perf_log = Instant::now();
        }

        // ---- 每帧把 view/proj 写进 Uniform Buffer（按 frame-in-flight 多份）----
        // 扩展字段（planes / cam_pos）仅网格着色器读取；传统顶点着色器只读前 144 字节。
        let (planes, cam_pos_w) = if self.mesh_enabled {
            let near_sq = quality_params(self.quality).instance_lod_distance;
            (Self::extract_frustum_planes(view, proj), near_sq * near_sq)
        } else {
            ([[0.0f32; 4]; 6], 0.0)
        };
        // 道具分桶剔除用的平面：**必须与 mesh 路径无关地存下来**。
        // 上面那个三元只在 mesh 路径算平面，而道具走的是传统顶点管线、在 mesh_enabled
        // 为 false 时也要能剔除；全零平面会让 bin_visible 恒真（退化为不剔除，安全但无效），
        // 所以这里无条件算一次。extract_frustum_planes 是纯算术，成本可忽略。
        self.frame_frustum = Self::extract_frustum_planes(view, proj);
        let ubo = CameraUniform {
            view,
            proj,
            // x/w = 地形 LOD 切换距离（shader 未读取，仅 CPU 侧语义），y/z = 实例淡出区间
            lod_params: [
                quality.terrain_lod_high_end,
                FADE_START,
                FADE_END,
                quality.terrain_lod_med_end,
            ],
            planes,
            cam_pos: [cam_pos.x, cam_pos.y, cam_pos.z, cam_pos_w],
        };
        if let Some(&ptr) = self.uniform_mapped.get(self.current_frame) {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &ubo as *const _ as *const u8,
                    ptr as *mut u8,
                    std::mem::size_of::<CameraUniform>(),
                );
            }
        }

        // ---- 光照 Uniform：写入 game 每帧更新的 light_data（默认全零 = 光照关闭）----
        let mut light_ubo = self.light_data;
        // RV3D_SKIN_TEX=1：flags.z 置 1 通知片元着色器启用 marker/NPC 程序化皮肤纹理
        // （缺省 0 保持纯色路径，冒烟基线不变；flags.x/y 语义不变）
        if self.skin_tex_enabled {
            light_ubo.flags.z = 1.0;
        }
        // flags.w：通知片元"地面微细节层（binding 9）真的存在，可以采样"。
        // 这个门是 build.rs 侧后加的防御：在它之前，binding 9 从未进过描述符集布局，
        // 未绑定描述符采样恒返回 0，而地面分支是乘性的（`mixed *= mix(1.0, g*2, gdetail)`），
        // 于是相机周边近处整圈地面被乘成纯黑。以图像句柄非空为条件是必要的——万一
        // init_texture 建图失败，这里保持 0，着色器就退回"没有细节层"而不是回到黑地。
        if self.ground_detail_image_view != vk::ImageView::null() {
            light_ubo.flags.w = 1.0;
        }
        if let Some(&ptr) = self.light_uniform_mapped.get(self.current_frame) {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &light_ubo as *const _ as *const u8,
                    ptr as *mut u8,
                    std::mem::size_of::<LightUniform>(),
                );
            }
        }

        // ---- 阴影 UBO：写入光空间 view-proj（每帧 slot 独立，避免 in-flight 竞态）----
        if let Some(&ptr) = self.shadow_ubo_mapped.get(self.current_frame) {
            let shadow_vp = self.light_data.shadow.light_view_proj;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &shadow_vp as *const _ as *const u8,
                    ptr as *mut u8,
                    std::mem::size_of::<glam::Mat4>(),
                );
            }
        }

        // 每帧重录 command buffer（instance_count 随剔除结果变化）
        let t0 = Instant::now();
        self.record_command_buffer(
            self.command_buffers[image_index as usize],
            image_index as usize,
            near_count,
            far_count,
            terrain_lod_index,
        )?;
        self.stage_record_us = t0.elapsed().as_micros() as u64;

        let wait_semaphores = [self.image_available_semaphores[self.current_frame]];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        // ⚠ **按下标 `image_index` 取，不是 `current_frame`** —— 理由见
        // `resize_render_finished_semaphores` 的文档（VUID-vkQueueSubmit-pSignalSemaphores-00067）。
        let render_finished = *self
            .render_finished_semaphores
            .get(image_index as usize)
            .ok_or_else(|| {
                format!(
                    "render-finished 信号量缺失：image_index={}，共 {} 个（应等于交换链图像数 {}）",
                    image_index,
                    self.render_finished_semaphores.len(),
                    self.swapchain_images.len()
                )
            })?;
        let signal_semaphores = [render_finished];
        let cmd_buffers = [self.command_buffers[image_index as usize]];

        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&cmd_buffers)
            .signal_semaphores(&signal_semaphores);

        let t0 = Instant::now();
        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence)
                .map_err(|e| format!("提交队列失败: {}", e))?;
        }
        self.stage_submit_us = t0.elapsed().as_micros() as u64;

        // ---- 截图：本帧若已请求，在 present 前读回 swapchain 图像并保存 PNG ----
        // （图像内容已确定；render_finished 信号量尚未被 present 消费，主机等待不会死锁）
        // 读回失败不跳过 present：未呈现的 swapchain 图像不会被回收，连续失败会耗尽图像导致卡死
        let mut screenshot_err: Option<String> = None;
        if self.screenshot_request.is_some() {
            if let Some(&image) = self.swapchain_images.get(image_index as usize) {
                if let Err(e) = self.do_screenshot_readback(image) {
                    screenshot_err = Some(e);
                }
            } else {
                self.screenshot_request = None;
                screenshot_err = Some("交换链图像索引越界".to_string());
            }
        }

        let swapchains = [self.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        let t0 = Instant::now();
        let present_result = unsafe {
            self.swapchain_loader
                .queue_present(self.present_queue, &present_info)
        };
        self.stage_present_us = t0.elapsed().as_micros() as u64;

        // 🔴 2026-09-22 复查补：呈现结果**必须每条都处理**（分类见 `classify_present`）。
        // 旧写法只处理 OUT_OF_DATE 与 SUBOPTIMAL，其余 Err（SURFACE_LOST / DEVICE_LOST）
        // 落空 ⇒ 主循环以为呈现成功，继续按"一切正常"跑下去。
        match classify_present(present_result) {
            PresentOutcome::Presented => {}
            PresentOutcome::RecreateSwapchain => {
                log::warn!("呈现 {:?}，重建交换链...", present_result);
                return Err("交换链过期".to_string());
            }
            PresentOutcome::Failed => {
                log::error!("呈现失败（{:?}）—— 不能当成成功", present_result);
                return Err(format!("呈现失败: {:?}", present_result));
            }
        }

        if let Some(e) = screenshot_err {
            return Err(format!("截图失败: {}", e));
        }

        self.last_frame_us = frame_start.elapsed().as_micros() as u64;
        self.current_frame = (self.current_frame + 1) % self.max_frames_in_flight;
        Ok(())
    }

    pub fn wait_idle(&self) -> Result<(), String> {
        unsafe {
            self.device
                .device_wait_idle()
                .map_err(|e| format!("等待设备空闲失败: {}", e))
        }
    }

    #[allow(dead_code)]
    pub fn recreate_swapchain(&mut self) -> Result<(), String> {
        self.wait_idle()?;
        self.destroy_swapchain();
        self.init_swapchain()?;
        // 交换链图像数可能变了 ⇒ render-finished 信号量的个数必须跟着变
        // （此处设备已空闲，销毁/重建都安全）
        self.resize_render_finished_semaphores()?;
        // 🔴 HUD overlay 的 framebuffer 绑的是**交换链 ImageView**，必须跟着重建 ——
        // 漏掉这一步就是未结案 #2：PT 通路随后用一组指向已销毁 ImageView 的 framebuffer
        // （见 `recreate_hud_framebuffers` 的文档）
        self.recreate_hud_framebuffers()?;
        self.init_msaa_resources()?;
        self.init_depth_resources()?;
        self.init_framebuffers()?;
        self.recreate_command_buffers()?;
        // 交换链尺寸/图像已变化：截图读回资源按旧 extent 创建，作废并清掉 pending 请求
        // （下次 capture_screenshot 时惰性重建）
        self.destroy_screenshot_resources();
        self.screenshot_request = None;
        Ok(())
    }

    fn recreate_command_buffers(&mut self) -> Result<(), String> {
        unsafe {
            self.device
                .free_command_buffers(self.command_pool, &self.command_buffers);
        }
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(self.framebuffers.len() as u32);
        self.command_buffers = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("重新分配命令缓冲失败: {}", e))?
        };
        for (i, &command_buffer) in self.command_buffers.iter().enumerate() {
            self.record_command_buffer(command_buffer, i, INSTANCE_COUNT, 0, TerrainLod::High as usize)?;
        }
        Ok(())
    }

    fn destroy_swapchain(&mut self) {
        // HUD overlay 的 framebuffer 引用 `swapchain_image_views`，**必须先于它们销毁**
        // （2026-09-15 补：此前这里漏了它，于是每次重建都留下一组悬空句柄）
        for &framebuffer in &self.hud_framebuffers {
            unsafe { self.device.destroy_framebuffer(framebuffer, None) };
        }
        self.hud_framebuffers.clear();
        for &framebuffer in &self.framebuffers {
            unsafe { self.device.destroy_framebuffer(framebuffer, None) };
        }
        self.framebuffers.clear();
        for &image_view in &self.swapchain_image_views {
            unsafe { self.device.destroy_image_view(image_view, None) };
        }
        self.swapchain_image_views.clear();
        // 深度资源
        for &view in &self.depth_image_views {
            unsafe { self.device.destroy_image_view(view, None) };
        }
        self.depth_image_views.clear();
        for (&image, &memory) in self
            .depth_images
            .iter()
            .zip(self.depth_images_memory.iter())
        {
            unsafe {
                self.device.destroy_image(image, None);
                self.device.free_memory(memory, None);
            }
        }
        self.depth_images.clear();
        self.depth_images_memory.clear();
        // MSAA 颜色附件
        for &view in &self.msaa_image_views {
            unsafe { self.device.destroy_image_view(view, None) };
        }
        self.msaa_image_views.clear();
        for (&image, &memory) in self
            .msaa_images
            .iter()
            .zip(self.msaa_image_memory.iter())
        {
            unsafe {
                self.device.destroy_image(image, None);
                self.device.free_memory(memory, None);
            }
        }
        self.msaa_images.clear();
        self.msaa_image_memory.clear();
        if self.swapchain != vk::SwapchainKHR::null() {
            unsafe {
                self.swapchain_loader
                    .destroy_swapchain(self.swapchain, None);
            }
            self.swapchain = vk::SwapchainKHR::null();
        }
    }
}

// ============================================================
// Drop：释放所有资源
// ============================================================

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();

            // 2026-08-29 显存纪律：PT 常驻资源（AS/管线/图像）显式销毁——退出后驱动立刻回收！
            self.destroy_pt_resident();

            // 释放截图读回资源（staging buffer + fence）
            self.destroy_screenshot_resources();

            // 释放同步对象
            for &fence in &self.in_flight_fences {
                self.device.destroy_fence(fence, None);
            }
            for &semaphore in &self.render_finished_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }
            for &semaphore in &self.image_available_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }

            // 释放命令池
            self.device.destroy_command_pool(self.command_pool, None);

            // 释放帧缓冲
            for &framebuffer in &self.framebuffers {
                self.device.destroy_framebuffer(framebuffer, None);
            }
            // HUD overlay 的 framebuffer（只被 PT 通路消费，见 recreate_hud_framebuffers）
            for &framebuffer in &self.hud_framebuffers {
                self.device.destroy_framebuffer(framebuffer, None);
            }

            // 释放管线
            if self.pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.pipeline, None);
            }
            for (b, m) in [
                (self.soldier_vertex_buffer, self.soldier_vertex_buffer_memory),
                (self.soldier_index_buffer, self.soldier_index_buffer_memory),
            ] {
                if b != vk::Buffer::null() { self.device.destroy_buffer(b, None); }
                if m != vk::DeviceMemory::null() { self.device.free_memory(m, None); }
            }
            if self.gun_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.gun_pipeline, None);
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device.destroy_pipeline_layout(self.pipeline_layout, None);
            }
            // 释放可选网格着色器管线（mesh_enabled=false 时为 null，直接跳过）
            if self.mesh_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.mesh_pipeline, None);
            }
            if self.mesh_pipeline_layout != vk::PipelineLayout::null() {
                self.device.destroy_pipeline_layout(self.mesh_pipeline_layout, None);
            }
            // 释放 HUD 覆盖层（独立 pipeline / 顶点缓冲）
            if self.hud_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.hud_pipeline, None);
            }
            if self.hud_overlay_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.hud_overlay_pipeline, None);
            }
            if self.hud_pipeline_layout != vk::PipelineLayout::null() {
                self.device.destroy_pipeline_layout(self.hud_pipeline_layout, None);
            }
            if self.hud_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.hud_vertex_buffer, None);
            }
            if self.hud_vertex_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.hud_vertex_buffer_memory, None);
            }
            // 释放第一人称枪模缓冲
            if self.gun_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.gun_vertex_buffer, None);
            }
            if self.gun_vertex_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.gun_vertex_buffer_memory, None);
            }
            if self.gun_index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.gun_index_buffer, None);
            }
            if self.gun_index_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.gun_index_buffer_memory, None);
            }
            // 道具合并网格缓冲（先解映射再释放内存，顺序不能反）
            if self.prop_mapped != std::ptr::null_mut() {
                self.device.unmap_memory(self.prop_vertex_memory);
                self.prop_mapped = std::ptr::null_mut();
            }
            if self.prop_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_vertex_buffer, None);
            }
            if self.prop_vertex_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_vertex_memory, None);
            }
            if self.prop_index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_index_buffer, None);
            }
            if self.prop_index_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_index_memory, None);
            }
            if self.prop_sh_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_sh_vertex_buffer, None);
            }
            if self.prop_sh_vertex_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_sh_vertex_memory, None);
            }
            if self.prop_sh_index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_sh_index_buffer, None);
            }
            if self.prop_sh_index_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_sh_index_memory, None);
            }
            // 🏢 PT 道具逐三角属性表（道具进 BLAS 专项）：渲染器自有的 device-local
            //   缓冲，teardown 必须销毁，否则验证层在设备销毁时报泄漏
            if self.prop_attr_buf != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_attr_buf, None);
            }
            if self.prop_attr_mem != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_attr_mem, None);
            }
            if self.render_pass != vk::RenderPass::null() {
                self.device.destroy_render_pass(self.render_pass, None);
            }

            // ---- 新增：释放 Descriptor 和 Uniform Buffer ----
            if self.descriptor_pool != vk::DescriptorPool::null() {
                self.device.destroy_descriptor_pool(self.descriptor_pool, None);
            }
            if self.descriptor_set_layout != vk::DescriptorSetLayout::null() {
                self.device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            }
            for &mapped in &self.uniform_mapped {
                // unmap 不需要判断 null，ash 会处理
                if !mapped.is_null() {
                    // 注意：ash 的 unmap 需要 DeviceMemory，我们逐个处理
                }
            }
            for (i, &buffer) in self.uniform_buffers.iter().enumerate() {
                if buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(buffer, None);
                }
                if let Some(&mem) = self.uniform_buffers_memory.get(i) {
                    if mem != vk::DeviceMemory::null() {
                        self.device.free_memory(mem, None);
                    }
                }
            }

            // 释放光照 Uniform Buffer
            for (i, &buffer) in self.light_uniform_buffers.iter().enumerate() {
                if buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(buffer, None);
                }
                if let Some(&mem) = self.light_uniform_buffers_memory.get(i) {
                    if mem != vk::DeviceMemory::null() {
                        self.device.free_memory(mem, None);
                    }
                }
            }

            // 释放阴影贴图资源（framebuffer 先于 render pass；descriptor sets 随 pool 释放）
            if self.shadow_framebuffer != vk::Framebuffer::null() {
                self.device.destroy_framebuffer(self.shadow_framebuffer, None);
            }
            if self.shadow_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.shadow_pipeline, None);
            }
            if self.shadow_pipeline_layout != vk::PipelineLayout::null() {
                self.device.destroy_pipeline_layout(self.shadow_pipeline_layout, None);
            }
            if self.shadow_render_pass != vk::RenderPass::null() {
                self.device.destroy_render_pass(self.shadow_render_pass, None);
            }
            if self.shadow_descriptor_set_layout != vk::DescriptorSetLayout::null() {
                self.device
                    .destroy_descriptor_set_layout(self.shadow_descriptor_set_layout, None);
            }
            for (i, &buffer) in self.shadow_ubo_buffers.iter().enumerate() {
                if buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(buffer, None);
                }
                if let Some(&mem) = self.shadow_ubo_memory.get(i) {
                    if mem != vk::DeviceMemory::null() {
                        self.device.free_memory(mem, None);
                    }
                }
            }
            if self.shadow_sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.shadow_sampler, None);
            }
            if self.shadow_image_view != vk::ImageView::null() {
                self.device.destroy_image_view(self.shadow_image_view, None);
            }
            if self.shadow_image != vk::Image::null() {
                self.device.destroy_image(self.shadow_image, None);
            }
            if self.shadow_image_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.shadow_image_memory, None);
            }

            // 释放图像视图
            for &image_view in &self.swapchain_image_views {
                self.device.destroy_image_view(image_view, None);
            }

            // 释放深度资源
            for &view in &self.depth_image_views {
                self.device.destroy_image_view(view, None);
            }
            for (&image, &memory) in self
                .depth_images
                .iter()
                .zip(self.depth_images_memory.iter())
            {
                self.device.destroy_image(image, None);
                self.device.free_memory(memory, None);
            }

            // 释放交换链
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_loader.destroy_swapchain(self.swapchain, None);
            }

            // 释放顶点缓冲
            if self.vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.vertex_buffer, None);
            }
            if self.vertex_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.vertex_buffer_memory, None);
            }
            if self.index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.index_buffer, None);
            }
            if self.index_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.index_buffer_memory, None);
            }
            if self.far_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.far_vertex_buffer, None);
            }
            if self.far_vertex_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.far_vertex_buffer_memory, None);
            }
            if self.far_index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.far_index_buffer, None);
            }
            if self.far_index_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.far_index_buffer_memory, None);
            }
            if self.ground_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.ground_vertex_buffer, None);
            }
            if self.ground_vertex_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.ground_vertex_buffer_memory, None);
            }
            if self.ground_index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.ground_index_buffer, None);
            }
            if self.ground_index_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.ground_index_buffer_memory, None);
            }
            // 释放地形 LOD 网格（顶点/索引缓冲）
            for mesh in &self.terrain_lods {
                if mesh.vertex_buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(mesh.vertex_buffer, None);
                }
                if mesh.vertex_memory != vk::DeviceMemory::null() {
                    self.device.free_memory(mesh.vertex_memory, None);
                }
                if mesh.index_buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(mesh.index_buffer, None);
                }
                if mesh.index_memory != vk::DeviceMemory::null() {
                    self.device.free_memory(mesh.index_memory, None);
                }
            }
            for &buffer in &self.instance_buffers {
                if buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(buffer, None);
                }
            }
            for &memory in &self.instance_buffers_memory {
                if memory != vk::DeviceMemory::null() {
                    self.device.free_memory(memory, None);
                }
            }

            // 释放纹理资源
            if self.texture_sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.texture_sampler, None);
            }
            if self.texture_image_view != vk::ImageView::null() {
                self.device.destroy_image_view(self.texture_image_view, None);
            }
            if self.texture_image != vk::Image::null() {
                self.device.destroy_image(self.texture_image, None);
            }
            if self.texture_image_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.texture_image_memory, None);
            }
            // 释放 marker/NPC 程序化皮肤纹理 + 地面微细节层
            for (img, mem, view) in [
                (
                    self.skin_marker_image,
                    self.skin_marker_memory,
                    self.skin_marker_image_view,
                ),
                (
                    self.skin_npc_image,
                    self.skin_npc_memory,
                    self.skin_npc_image_view,
                ),
                (
                    self.ground_detail_image,
                    self.ground_detail_memory,
                    self.ground_detail_image_view,
                ),
            ] {
                if view != vk::ImageView::null() {
                    self.device.destroy_image_view(view, None);
                }
                if img != vk::Image::null() {
                    self.device.destroy_image(img, None);
                }
                if mem != vk::DeviceMemory::null() {
                    self.device.free_memory(mem, None);
                }
            }

            // 释放逻辑设备
            self.device.destroy_device(None);

            // 释放表面
            self.surface_loader.destroy_surface(self.surface, None);

            // 释放调试回调
            if let (Some(ref debug_utils), Some(messenger)) =
                (&self.debug_utils, self.debug_messenger)
            {
                debug_utils.destroy_debug_utils_messenger(messenger, None);
            }

            // 释放实例
            self.instance.destroy_instance(None);
        }
    }
}

// ============================================================
// 调试回调
// ============================================================

unsafe extern "system" fn vulkan_debug_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _user_data: *mut std::ffi::c_void,
) -> vk::Bool32 {
    let severity = if message_severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        "ERROR"
    } else if message_severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        "WARNING"
    } else if message_severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::INFO) {
        "INFO"
    } else {
        "VERBOSE"
    };

    let ty = if message_type.contains(vk::DebugUtilsMessageTypeFlagsEXT::GENERAL) {
        "General"
    } else if message_type.contains(vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION) {
        "Validation"
    } else {
        "Performance"
    };

    if let Some(data) = p_callback_data.as_ref() {
        let msg = if data.p_message.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(data.p_message) }
                .to_string_lossy()
                .to_string()
        };
        match severity {
            "ERROR" => log::error!("[Vulkan][{}] {}", ty, msg),
            "WARNING" => log::warn!("[Vulkan][{}] {}", ty, msg),
            _ => log::info!("[Vulkan][{}] {}", ty, msg),
        }
    }
    vk::FALSE
}

// ============================================================
// 障碍 marker 尺寸单元测试
// ============================================================

#[cfg(test)]
mod marker_scale_tests {
    use super::*;
    use crate::engine::geom::Shape;

    /// 端到端不变式：**画出来的尺寸逐轴恒等于碰撞 AABB**。
    ///
    /// 存在理由：`for_obstacle` 此前对三个轴一律写 `2*half`，而模板是 ±1 的立方体/球、
    /// r=1 的圆柱 ⇒ 全城 1700 多个程序化构件画成设计尺寸的 **2 倍**（灌木球 φ2.86 画成
    /// φ5.7、路缘石 0.55 宽画成 1.1 m 矮墙、柱头 φ0.92 画成 φ1.84 的悬空圆盘），并且
    /// 玩家能站进"看得见的那半个盒子"里 = 穿模。它存活了三周，因为**圆柱的高度恰好是对的**
    /// （模板 y 已是 ±0.5），"部分正确"让每次目视核对都能找到一条反例说服自己。
    ///
    /// 这条测试按形状逐轴验，任何一侧（模板 or 缩放推导）单独改动都会让它红。
    #[test]
    fn marker_visible_size_matches_aabb() {
        for shape in [
            Shape::Legacy,
            Shape::Cylinder,
            Shape::Sphere,
            Shape::Authored,
        ] {
            let ob = MapObstacle::new(ObstacleKind::Block, 1.0, -2.0, 0.35, 0.6)
                .shaped(0.5, 1.25, Some([0.5, 0.5, 0.5]))
                .geom(shape);
            let m = WorldMarker::for_obstacle(&ob);
            // 模型 = T × S ⇒ 三个基向量的模长就是逐轴缩放
            let scale = [
                m.model.x_axis.length(),
                m.model.y_axis.length(),
                m.model.z_axis.length(),
            ];
            let half = [ob.half_w, ob.half_h, ob.half_d];
            for axis in 0..3 {
                let drawn = scale[axis] * shape.template_half_extent(axis) * 2.0;
                let want = half[axis] * 2.0;
                assert!(
                    (drawn - want).abs() < 1e-5,
                    "{shape:?} 轴 {axis}：画出来 {drawn} m，碰撞 AABB 是 {want} m —— \
                     可见尺寸与碰撞盒不一致（判据见 geom::Shape::template_half_extent）"
                );
            }
            // 盒心必须原样落在障碍中心，不得被缩放带偏
            let t = m.model.w_axis.truncate();
            assert_eq!([t.x, t.y, t.z], [ob.x, ob.y, ob.z]);
        }
    }

    /// 缩放必须是**纯对角**的：模板按轴归一后若还带旋转/剪切，光照法线（屏幕导数重建）
    /// 与阴影深度都会错，而且绕序判定不再成立。
    #[test]
    fn marker_model_is_pure_translation_and_scale() {
        let ob = MapObstacle::new(ObstacleKind::Building, 3.0, -7.0, 1.5, 2.25).geom(Shape::Cylinder);
        let m = WorldMarker::for_obstacle(&ob).model;
        for (i, col) in [m.x_axis, m.y_axis, m.z_axis, m.w_axis].iter().enumerate() {
            for j in 0..3 {
                if i == j || (i == 3 && j != 3) {
                    continue;
                }
                let v = [col.x, col.y, col.z][j];
                assert_eq!(v, 0.0, "模型矩阵第 {i} 列第 {j} 行应为 0，实际 {v}");
            }
        }
    }
}

// ============================================================
// 实例槽位布局单元测试
// ============================================================

#[cfg(test)]
mod instance_slot_layout_tests {
    use super::*;

    /// 槽位布局钉死测试。
    ///
    /// `build.rs` 的两段 WGSL（顶点/网格着色器）里，枪模槽是**字面量** `78913u`，
    /// 而它由 `MAX_MARKER_INSTANCES` 推导。历史上这里已经因为"改了容量忘了改字面量"
    /// 出过两次真 bug（枪槽区间覆盖 NPC 圆柱/球体段 → 四肢和头被 z=0 深度覆盖，
    /// 表现为"鬼魂穿模"）。字面量没法被 Rust 类型系统检查，所以用测试兜住：
    /// 改任何一档容量都必须同时改 build.rs 的两处字面量，否则本测试失败。
    ///
    /// `#[allow(clippy::assertions_on_constants)]`：本条测试**全部内容**就是断言常量之间
    /// 的关系，这正是它的价值所在。clippy 那条 lint 针对的是 `assert!(true)` 这类笔误，
    /// 套不到故意钉死布局的回归护栏上——删掉才是丢保护。
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn gun_slot_layout_is_pinned() {
        assert_eq!(MARKER_SLOT_BASE, 65537, "marker 区起点 = 地形 identity 之后一槽");
        assert_eq!(NPC_SLOT_BASE, 65537 + 8192);
        assert_eq!(NPC_CYL_SLOT_BASE, 73729 + 3072);
        assert_eq!(NPC_SPH_SLOT_BASE, 73729 + 6144);
        assert_eq!(EMISSIVE_SLOT_BASE, 73729 + 9216);
        assert_eq!(
            GUN_INSTANCE_INDEX, 83009,
            "枪槽变了：必须同步改 build.rs 里两处 `== 83009u` 字面量"
        );
        // 枪槽必须紧贴自发光区之后、且落在自发光区间之外，否则会被当成自发光刷白。
        assert_eq!(GUN_INSTANCE_INDEX, EMISSIVE_SLOT_BASE + MAX_EMISSIVE_INSTANCES);
        assert!(
            GUN_INSTANCE_INDEX >= EMISSIVE_SLOT_BASE + 64,
            "枪槽落进自发光区间会被判成 emissive（历史 bug）"
        );
    }

    /// marker 区不得越界侵占 NPC 区（曾因 1024/3072 写错导致前 2048 个 NPC 盒体被当 marker）。
    #[test]
    fn marker_band_does_not_bleed_into_npc_band() {
        assert_eq!(NPC_SLOT_BASE - MARKER_SLOT_BASE, MAX_MARKER_INSTANCES);
        assert_eq!(EMISSIVE_SLOT_BASE - NPC_SLOT_BASE, MAX_NPC_INSTANCES * 3);
    }
}

// ============================================================
// 地形 LOD 单元测试
// ============================================================

#[cfg(test)]
mod terrain_lod_tests {
    use super::*;

    #[test]
    fn terrain_lod_level_selection_by_distance() {
        // 近距离 → 高级
        assert_eq!(terrain_lod_for_distance(0.0), TerrainLod::High);
        assert_eq!(
            terrain_lod_for_distance(TERRAIN_LOD_HIGH_END - 1.0),
            TerrainLod::High
        );
        // 中距离 → 中级
        assert_eq!(
            terrain_lod_for_distance(TERRAIN_LOD_HIGH_END),
            TerrainLod::Medium
        );
        assert_eq!(
            terrain_lod_for_distance(TERRAIN_LOD_MED_END - 1.0),
            TerrainLod::Medium
        );
        // 远距离 → 低级
        assert_eq!(
            terrain_lod_for_distance(TERRAIN_LOD_MED_END),
            TerrainLod::Low
        );
        assert_eq!(terrain_lod_for_distance(f32::MAX), TerrainLod::Low);
    }

    #[test]
    fn terrain_lod_density_table() {
        // 高级 257²（256 格，间距 2.0）
        assert_eq!(TerrainLod::High.cells(), 256);
        assert_eq!(TerrainLod::High.verts(), 257);
        assert_eq!(TerrainLod::High.cell_size(), 2.0);
        assert_eq!(TerrainLod::High.index_count(), (256 * 256 * 6) as u32);
        // 中级 129²（128 格，间距 4.0）
        assert_eq!(TerrainLod::Medium.cells(), 128);
        assert_eq!(TerrainLod::Medium.verts(), 129);
        assert_eq!(TerrainLod::Medium.cell_size(), 4.0);
        // 低级 65²（64 格，间距 8.0）
        assert_eq!(TerrainLod::Low.cells(), 64);
        assert_eq!(TerrainLod::Low.verts(), 65);
        assert_eq!(TerrainLod::Low.cell_size(), 8.0);
        assert_eq!(TerrainLod::Low.index_count(), (64 * 64 * 6) as u32);
    }

    #[test]
    fn terrain_lod_blend_morphs_between_levels() {
        // 过渡带端点：blend 0→1，smoothstep 中点 = 0.5
        assert_eq!(terrain_lod_blend(0.0), (TerrainLod::High, 0.0));
        assert_eq!(
            terrain_lod_blend(TERRAIN_LOD_HIGH_MORPH_START),
            (TerrainLod::High, 0.0)
        );
        let mid = (TERRAIN_LOD_HIGH_MORPH_START + TERRAIN_LOD_HIGH_END) * 0.5;
        let (level, blend) = terrain_lod_blend(mid);
        assert_eq!(level, TerrainLod::High);
        assert!((blend - 0.5).abs() < 1e-4, "blend={}", blend);

        // 级别边界：High@blend→1 与 Medium@blend=0 几何重合，无 popping
        let (level, blend) = terrain_lod_blend(TERRAIN_LOD_HIGH_END - 0.5);
        assert_eq!(level, TerrainLod::High);
        assert!(blend > 0.99, "blend={}", blend);
        assert_eq!(
            terrain_lod_blend(TERRAIN_LOD_HIGH_END),
            (TerrainLod::Medium, 0.0)
        );
        assert_eq!(
            terrain_lod_blend(TERRAIN_LOD_HIGH_END + 1.0),
            (TerrainLod::Medium, 0.0)
        );

        let (level, blend) = terrain_lod_blend(TERRAIN_LOD_MED_END - 0.5);
        assert_eq!(level, TerrainLod::Medium);
        assert!(blend > 0.99, "blend={}", blend);
        assert_eq!(
            terrain_lod_blend(TERRAIN_LOD_MED_END),
            (TerrainLod::Low, 1.0)
        );
        assert_eq!(
            terrain_lod_blend(TERRAIN_LOD_MED_END + 1.0),
            (TerrainLod::Low, 1.0)
        );

        // 全距离扫描：级别只前进不后退，同级内 blend 单调不减
        let mut last_blend = -1.0f32;
        let mut last_level = 0usize;
        for i in 0..=1000 {
            let dist = i as f32 * 3.0;
            let (level, blend) = terrain_lod_blend(dist);
            let level_idx = level as usize;
            assert!(level_idx >= last_level, "级别回退 at dist={}", dist);
            if level_idx == last_level {
                assert!(blend + 1e-6 >= last_blend, "blend 回退 at dist={}", dist);
            }
            last_blend = blend;
            last_level = level_idx;
        }
    }

    #[test]
    fn terrain_coarse_height_interpolates_coarse_surface() {
        // 低级网格高度
        let low_cells = TerrainLod::Low.cells();
        let w = low_cells + 1;
        let cell = TerrainLod::Low.cell_size();
        let mut heights = Vec::with_capacity(w * w);
        for iz in 0..w {
            for ix in 0..w {
                let x = -TERRAIN_HALF + ix as f32 * cell;
                let z = -TERRAIN_HALF + iz as f32 * cell;
                heights.push(terrain_height(x, z));
            }
        }

        // 中级网格中与低级网格重合的顶点（ix/iz 均为偶数）：
        // 粗曲面插值必须精确等于该点 terrain_height
        let med_w = TerrainLod::Medium.verts();
        let med_cell = TerrainLod::Medium.cell_size();
        for iz in 0..med_w {
            for ix in 0..med_w {
                if ix % 2 != 0 || iz % 2 != 0 {
                    continue;
                }
                let x = -TERRAIN_HALF + ix as f32 * med_cell;
                let z = -TERRAIN_HALF + iz as f32 * med_cell;
                let interp = terrain_coarse_height(x, z, &heights, low_cells);
                let direct = terrain_height(x, z);
                assert!(
                    (interp - direct).abs() < 1e-4,
                    "({}, {}): interp={} direct={}",
                    x,
                    z,
                    interp,
                    direct
                );
            }
        }

        // 低级 cell 中心位于对角线上：插值 = 两对角角点高度均值（两三角形一致）
        let cx = 3usize;
        let cz = 5usize;
        let x = -TERRAIN_HALF + (cx as f32 + 0.5) * cell;
        let z = -TERRAIN_HALF + (cz as f32 + 0.5) * cell;
        let h00 = heights[cz * w + cx];
        let h11 = heights[(cz + 1) * w + cx + 1];
        let interp = terrain_coarse_height(x, z, &heights, low_cells);
        assert!((interp - (h00 + h11) * 0.5).abs() < 1e-5);
    }
}

// ============================================================
// 程序化地形高度单元测试
// ============================================================

#[cfg(test)]
mod terrain_height_tests {
    use super::*;

    #[test]
    fn terrain_height_deterministic_and_same_source() {
        // 同参数同输入同输出（单测/回放依赖）；terrain_height_at 与 terrain_height 同源
        for &(x, z) in &[
            (0.0, 0.0),
            (30.0, -30.0),
            (120.0, 80.0),
            (150.0, 10.0),
            (-200.0, 250.0),
            (255.0, -255.0),
        ] {
            assert_eq!(terrain_height(x, z), terrain_height(x, z), "({},{})", x, z);
            assert_eq!(
                terrain_height_at(x, z),
                terrain_height(x, z),
                "({},{})",
                x,
                z
            );
        }
    }

    #[test]
    fn terrain_flat_within_central_and_ring_zones() {
        // 中央 60×60（|x|≤30 且 |z|≤30）恒 y=0
        for &x in &[-30.0, -15.0, 0.0, 15.0, 30.0] {
            for &z in &[-30.0, 0.0, 30.0] {
                assert_eq!(terrain_height(x, z), 0.0, "central ({},{})", x, z);
            }
        }
        // 半径 ≤ 140m（覆盖障碍环带 58–130m 与两军接火区）恒 y=0
        for &(x, z) in &[
            (0.0, 140.0),
            (140.0, 0.0),
            (-140.0, 0.0),
            (0.0, -140.0),
            (90.0, 107.0),
            (-90.0, -107.0),
        ] {
            assert_eq!(terrain_height(x, z), 0.0, "ring ({},{})", x, z);
        }
    }

    #[test]
    fn terrain_hills_bounded_varied_and_gentle() {
        // 全图扫描（间距 2m，与 High LOD 网格同采样）：|高度| ≤ 15m、
        // 相邻点高度差 ≤ 0.6m（坡度 ≤ ~17°，平缓，LOD morph 不突兀）
        let mut max_h = 0.0f32;
        let mut ring_max = 0.0f32;
        for iz in 0..=255usize {
            for ix in 0..=255usize {
                let x = -TERRAIN_HALF + ix as f32 * 2.0;
                let z = -TERRAIN_HALF + iz as f32 * 2.0;
                let h = terrain_height(x, z);
                assert!(
                    h.abs() <= TERRAIN_HILL_AMPLITUDE + 1e-6,
                    "|h|={} 超限 at ({},{})",
                    h,
                    x,
                    z
                );
                max_h = max_h.max(h.abs());
                let r = (x * x + z * z).sqrt();
                if r >= 250.0 {
                    ring_max = ring_max.max(h.abs());
                }
                if ix < 255 {
                    let dx = terrain_height(x + 2.0, z) - h;
                    assert!(dx.abs() <= 0.6, "dx={} at ({},{})", dx, x, z);
                }
                if iz < 255 {
                    let dz = terrain_height(x, z + 2.0) - h;
                    assert!(dz.abs() <= 0.6, "dz={} at ({},{})", dz, x, z);
                }
            }
        }
        // 外围确实有起伏（防回退成全平）
        assert!(ring_max > 1.0, "外围丘陵应有起伏，实际 ring_max={}", ring_max);
        assert!(max_h > 1.0, "全图应有非零地形，实际 max_h={}", max_h);
    }
}

// ============================================================
// 画质预设单元测试
// ============================================================

#[cfg(test)]
mod quality_preset_tests {
    use super::*;

    #[test]
    fn quality_medium_matches_existing_constants() {
        // Medium 必须保持当前行为：阈值与现有常量完全一致
        let p = quality_params(QualityPreset::Medium);
        assert_eq!(p.terrain_lod_high_end, TERRAIN_LOD_HIGH_END);
        assert_eq!(p.terrain_lod_med_end, TERRAIN_LOD_MED_END);
        assert_eq!(p.terrain_lod_high_morph_start, TERRAIN_LOD_HIGH_MORPH_START);
        assert_eq!(p.terrain_lod_med_morph_start, TERRAIN_LOD_MED_MORPH_START);
        assert_eq!(p.instance_lod_distance, LOD_DISTANCE);
    }

    #[test]
    fn quality_low_medium_high_ordering() {
        // Low 阈值减小、High 阈值增大
        let low = quality_params(QualityPreset::Low);
        let med = quality_params(QualityPreset::Medium);
        let high = quality_params(QualityPreset::High);
        assert!(low.terrain_lod_high_end < med.terrain_lod_high_end);
        assert!(med.terrain_lod_high_end < high.terrain_lod_high_end);
        assert!(low.terrain_lod_med_end < med.terrain_lod_med_end);
        assert!(med.terrain_lod_med_end < high.terrain_lod_med_end);
        assert!(low.instance_lod_distance < med.instance_lod_distance);
        assert!(med.instance_lod_distance < high.instance_lod_distance);
        // morph 过渡带起点必须小于对应终点
        assert!(low.terrain_lod_high_morph_start < low.terrain_lod_high_end);
        assert!(low.terrain_lod_med_morph_start < low.terrain_lod_med_end);
        assert!(high.terrain_lod_high_morph_start < high.terrain_lod_high_end);
        assert!(high.terrain_lod_med_morph_start < high.terrain_lod_med_end);
    }

    #[test]
    fn quality_preset_default_and_label() {
        assert_eq!(QualityPreset::DEFAULT, QualityPreset::Medium);
        assert_eq!(QualityPreset::Low.label(), "低画质");
        assert_eq!(QualityPreset::Medium.label(), "中画质");
        assert_eq!(QualityPreset::High.label(), "高画质");
    }

    #[test]
    fn terrain_lod_switch_uses_quality_params() {
        // 同一距离下不同画质的 LOD 级别不同：距离 90 时 Low 已降级、Medium/High 仍高级
        let low = quality_params(QualityPreset::Low);
        let med = quality_params(QualityPreset::Medium);
        let high = quality_params(QualityPreset::High);
        assert_eq!(
            terrain_lod_for_distance_with_params(90.0, low),
            TerrainLod::Medium
        );
        assert_eq!(
            terrain_lod_for_distance_with_params(90.0, med),
            TerrainLod::High
        );
        assert_eq!(
            terrain_lod_for_distance_with_params(90.0, high),
            TerrainLod::High
        );
        // 距离 150：Medium 已中级；High 高阈值 145，140 时仍高级、150 时降为中级
        assert_eq!(
            terrain_lod_for_distance_with_params(150.0, med),
            TerrainLod::Medium
        );
        assert_eq!(
            terrain_lod_for_distance_with_params(140.0, high),
            TerrainLod::High
        );
        assert_eq!(
            terrain_lod_for_distance_with_params(150.0, high),
            TerrainLod::Medium
        );
    }
}

// ============================================================
// PNG 截图纯逻辑单元测试
// ============================================================

#[cfg(test)]
mod screenshot_pixel_tests {
    use super::*;

    #[test]
    fn pixel_order_supported_formats() {
        assert_eq!(
            pixel_order_for_format(vk::Format::B8G8R8A8_UNORM),
            Ok(PixelOrder::Bgra)
        );
        assert_eq!(
            pixel_order_for_format(vk::Format::B8G8R8A8_SRGB),
            Ok(PixelOrder::Bgra)
        );
        assert_eq!(
            pixel_order_for_format(vk::Format::R8G8B8A8_UNORM),
            Ok(PixelOrder::Rgba)
        );
        assert_eq!(
            pixel_order_for_format(vk::Format::R8G8B8A8_SRGB),
            Ok(PixelOrder::Rgba)
        );
        // 未知格式 → Err
        assert!(pixel_order_for_format(vk::Format::A2B10G10R10_UNORM_PACK32).is_err());
        assert!(pixel_order_for_format(vk::Format::UNDEFINED).is_err());
    }

    #[test]
    fn convert_bgra_pixels_to_rgba() {
        // BGRA 蓝色 [255,0,0,255] → RGBA 红色 [0,0,255,255]
        let src = [255u8, 0, 0, 255];
        let mut dst = [0u8; 4];
        convert_pixels_to_rgba(vk::Format::B8G8R8A8_UNORM, &src, &mut dst).unwrap();
        assert_eq!(dst, [0, 0, 255, 255]);
        // 多像素行：R/B 交换、G/A 保持
        let src2 = [10u8, 20, 30, 40, 1, 2, 3, 4];
        let mut dst2 = [0u8; 8];
        convert_pixels_to_rgba(vk::Format::B8G8R8A8_SRGB, &src2, &mut dst2).unwrap();
        assert_eq!(dst2, [30, 20, 10, 40, 3, 2, 1, 4]);
    }

    #[test]
    fn convert_rgba_pixels_passthrough() {
        let src = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut dst = [0u8; 8];
        convert_pixels_to_rgba(vk::Format::R8G8B8A8_SRGB, &src, &mut dst).unwrap();
        assert_eq!(dst, src);
    }

    #[test]
    fn convert_pixels_rejects_bad_length_or_format() {
        // 长度不匹配 → Err
        let src = [1u8, 2, 3];
        let mut dst = [0u8; 4];
        assert!(convert_pixels_to_rgba(vk::Format::R8G8B8A8_UNORM, &src, &mut dst).is_err());
        let src2 = [1u8, 2, 3, 4];
        let mut dst2 = [0u8; 3];
        assert!(convert_pixels_to_rgba(vk::Format::R8G8B8A8_UNORM, &src2, &mut dst2).is_err());
        // 未知格式 → Err
        let mut dst3 = [0u8; 4];
        assert!(convert_pixels_to_rgba(vk::Format::UNDEFINED, &src2, &mut dst3).is_err());
    }
}

#[cfg(test)]
mod simd_cull_tests {
    use super::*;

    /// 简单确定性伪随机（SplitMix64），纯逻辑测试不碰 GPU
    struct Rng(u64);
    impl Rng {
        fn next_f32(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 33) as u32 as f64 / (1u64 << 31) as f64) as f32
        }
    }

    #[test]
    fn simd_cull_matches_scalar() {
        let mut rng = Rng(0x5EED_2026);
        // 6 个随机朝向平面（法线随机、d 随机），覆盖可见/剔除/边界混合场景
        let mut planes = [[0f32; 4]; 6];
        for p in &mut planes {
            let (nx, ny, nz) = (rng.next_f32() * 2.0 - 1.0, rng.next_f32() * 2.0 - 1.0, rng.next_f32() * 2.0 - 1.0);
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            p[0] = nx / len;
            p[1] = ny / len;
            p[2] = nz / len;
            p[3] = rng.next_f32() * 4.0 - 2.0;
        }

        let n = 65536;
        let mut cx = Vec::with_capacity(n);
        let mut cy = Vec::with_capacity(n);
        let mut cz = Vec::with_capacity(n);
        let mut radii = Vec::with_capacity(n);
        for _ in 0..n {
            cx.push(rng.next_f32() * 200.0 - 100.0);
            cy.push(rng.next_f32() * 200.0 - 100.0);
            cz.push(rng.next_f32() * 200.0 - 100.0);
            radii.push(rng.next_f32() * 4.0);
        }

        let mut scalar_out = Vec::new();
        Renderer::cull_spheres_scalar(&cx, &cy, &cz, &radii, &planes, &mut scalar_out);

        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx2") {
            let mut avx_out = Vec::new();
            // safety: 已运行时检测 AVX2
            unsafe {
                Renderer::cull_spheres_avx2(&cx, &cy, &cz, &radii, &planes, &mut avx_out);
            }
            assert_eq!(avx_out, scalar_out, "AVX2 剔除结果与标量逐位不一致");
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx512f") {
            let mut avx512_out = Vec::new();
            // safety: 已运行时检测 AVX-512
            unsafe {
                Renderer::cull_spheres_avx512(&cx, &cy, &cz, &radii, &planes, &mut avx512_out);
            }
            assert_eq!(avx512_out, scalar_out, "AVX-512 剔除结果与标量逐位不一致");
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx") {
            let mut avx_out = Vec::new();
            // safety: 已运行时检测 AVX
            unsafe {
                Renderer::cull_spheres_avx(&cx, &cy, &cz, &radii, &planes, &mut avx_out);
            }
            assert_eq!(avx_out, scalar_out, "AVX 剔除结果与标量逐位不一致");
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("sse4.2") {
            let mut sse_out = Vec::new();
            // safety: 已运行时检测 SSE4.2
            unsafe {
                Renderer::cull_spheres_sse(&cx, &cy, &cz, &radii, &planes, &mut sse_out);
            }
            assert_eq!(sse_out, scalar_out, "SSE4.2 剔除结果与标量逐位不一致");
        }
        #[cfg(target_arch = "aarch64")]
        if std::arch::is_aarch64_feature_detected!("neon") {
            let mut neon_out = Vec::new();
            // safety: 已运行时检测 NEON（AArch64 基线特性）
            unsafe {
                Renderer::cull_spheres_neon(&cx, &cy, &cz, &radii, &planes, &mut neon_out);
            }
            assert_eq!(neon_out, scalar_out, "NEON 剔除结果与标量逐位不一致");
        }
        // 非 x86_64 或无双 AVX2：标量结果本身就是正确语义，无需对照
        assert!(!scalar_out.is_empty() || scalar_out.is_empty());
    }

    #[test]
    fn simd_cull_tail_batches_handled() {
        // 长度非 8 的倍数：覆盖尾部标量路径
        let mut planes = [[0f32; 4]; 6];
        for p in &mut planes {
            p[0] = 0.0;
            p[1] = 0.0;
            p[2] = 1.0;
            p[3] = 0.0;
        }
        let n = 13; // 1 个 AVX2 批 + 5 个尾部
        let mut cx = Vec::with_capacity(n);
        let mut cy = Vec::with_capacity(n);
        let mut cz = Vec::with_capacity(n);
        let mut radii = Vec::with_capacity(n);
        for i in 0..n {
            cx.push((i as f32) - 6.0);
            cy.push(0.0);
            cz.push((i as f32) - 6.0);
            radii.push(1.0);
        }
        let mut scalar_out = Vec::new();
        Renderer::cull_spheres_scalar(&cx, &cy, &cz, &radii, &planes, &mut scalar_out);

        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx2") {
            let mut avx_out = Vec::new();
            unsafe {
                Renderer::cull_spheres_avx2(&cx, &cy, &cz, &radii, &planes, &mut avx_out);
            }
            assert_eq!(avx_out, scalar_out);
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx512f") {
            let mut avx512_out = Vec::new();
            unsafe {
                Renderer::cull_spheres_avx512(&cx, &cy, &cz, &radii, &planes, &mut avx512_out);
            }
            assert_eq!(avx512_out, scalar_out);
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx") {
            let mut avx_out = Vec::new();
            unsafe {
                Renderer::cull_spheres_avx(&cx, &cy, &cz, &radii, &planes, &mut avx_out);
            }
            assert_eq!(avx_out, scalar_out);
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("sse4.2") {
            let mut sse_out = Vec::new();
            unsafe {
                Renderer::cull_spheres_sse(&cx, &cy, &cz, &radii, &planes, &mut sse_out);
            }
            assert_eq!(sse_out, scalar_out);
        }
        #[cfg(target_arch = "aarch64")]
        if std::arch::is_aarch64_feature_detected!("neon") {
            let mut neon_out = Vec::new();
            // safety: 已运行时检测 NEON（AArch64 基线特性）
            unsafe {
                Renderer::cull_spheres_neon(&cx, &cy, &cz, &radii, &planes, &mut neon_out);
            }
            assert_eq!(neon_out, scalar_out, "NEON 剔除结果与标量逐位不一致");
        }
    }

    #[test]
    fn simd_morph_matches_scalar() {
        // 覆盖各档批量倍数 + 尾部：33（AVX-512 两批+1 尾部）与 65536（整场地形网格）全量对比
        for n in [33usize, 65536usize] {
            let mut rng = Rng(0x51AD_0007 ^ n as u64);
            let mut base = Vec::with_capacity(n);
            let mut coarse = Vec::with_capacity(n);
            for _ in 0..n {
                base.push(rng.next_f32() * 40.0 - 20.0);
                coarse.push(rng.next_f32() * 40.0 - 20.0);
            }
            let blend = rng.next_f32();
            let mut scalar_out = vec![0.0f32; n];
            Renderer::morph_heights_scalar(&base, &coarse, blend, &mut scalar_out);

            let mut out = vec![0.0f32; n];
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx512f") && crate::engine::cpu::avx512_enabled() {
                // safety: 已运行时检测 AVX-512 且未被型号过滤
                unsafe {
                    Renderer::morph_heights_avx512(&base, &coarse, blend, &mut out);
                }
                assert_eq!(out, scalar_out, "AVX-512 morph 与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx2") {
                // safety: 已运行时检测 AVX2
                unsafe {
                    Renderer::morph_heights_avx2(&base, &coarse, blend, &mut out);
                }
                assert_eq!(out, scalar_out, "AVX2 morph 与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx") {
                // safety: 已运行时检测 AVX
                unsafe {
                    Renderer::morph_heights_avx(&base, &coarse, blend, &mut out);
                }
                assert_eq!(out, scalar_out, "AVX morph 与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("sse4.2") {
                // safety: 已运行时检测 SSE4.2
                unsafe {
                    Renderer::morph_heights_sse(&base, &coarse, blend, &mut out);
                }
                assert_eq!(out, scalar_out, "SSE4.2 morph 与标量逐位不一致 (n={})", n);
            }
            out.fill(0.0);
            #[cfg(target_arch = "aarch64")]
            if std::arch::is_aarch64_feature_detected!("neon") {
                // safety: NEON 在 AArch64 是基线特性（此处仍运行时确认）
                unsafe {
                    Renderer::morph_heights_neon(&base, &coarse, blend, &mut out);
                }
                assert_eq!(out, scalar_out, "NEON morph 与标量逐位不一致 (n={})", n);
            }
            // dispatch 选路（当前机器实际启用档位）也必须与标量逐位一致
            out.fill(0.0);
            Renderer::morph_heights_dispatch(&base, &coarse, blend, &mut out);
            assert_eq!(out, scalar_out, "dispatch morph 与标量逐位不一致 (n={})", n);
        }
    }

    /// 指令级微基准（隔离进程、无渲染并发）：65536 实例剔除 + 65536 点 morph，各档 vs 标量。
    /// 运行：`cargo test --release simd_cull_microbench -- --nocapture --test-threads=1`
    #[test]
    fn simd_cull_microbench() {
        let mut rng = Rng(0xC011_2026);
        let mut planes = [[0f32; 4]; 6];
        for p in &mut planes {
            let (nx, ny, nz) = (
                rng.next_f32() * 2.0 - 1.0,
                rng.next_f32() * 2.0 - 1.0,
                rng.next_f32() * 2.0 - 1.0,
            );
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            p[0] = nx / len;
            p[1] = ny / len;
            p[2] = nz / len;
            p[3] = rng.next_f32() * 4.0 - 2.0;
        }
        let n = 65536usize;
        let mut cx = Vec::with_capacity(n);
        let mut cy = Vec::with_capacity(n);
        let mut cz = Vec::with_capacity(n);
        let mut radii = Vec::with_capacity(n);
        for _ in 0..n {
            cx.push(rng.next_f32() * 200.0 - 100.0);
            cy.push(rng.next_f32() * 200.0 - 100.0);
            cz.push(rng.next_f32() * 200.0 - 100.0);
            radii.push(rng.next_f32() * 4.0);
        }
        let mut base = Vec::with_capacity(n);
        let mut coarse = Vec::with_capacity(n);
        for _ in 0..n {
            base.push(rng.next_f32() * 40.0 - 20.0);
            coarse.push(rng.next_f32() * 40.0 - 20.0);
        }
        let blend = 0.5f32;

        let mut cull_paths: Vec<(&'static str, Box<dyn Fn(&mut Vec<u32>)>)> = Vec::new();
        let mut morph_paths: Vec<(&'static str, Box<dyn Fn(&mut [f32])>)> = Vec::new();
        macro_rules! add_paths {
            ($name:expr, $cull:ident, $morph:ident) => {{
                let (cx, cy, cz, radii, planes, base, coarse) =
                    (cx.clone(), cy.clone(), cz.clone(), radii.clone(), planes, base.clone(), coarse.clone());
                cull_paths.push((
                    $name,
                    Box::new(move |out: &mut Vec<u32>| {
                        // safety: 已由调用方按硬件能力注册
                        unsafe {
                            Renderer::$cull(&cx, &cy, &cz, &radii, &planes, out);
                        }
                    }),
                ));
                morph_paths.push((
                    $name,
                    Box::new(move |out: &mut [f32]| {
                        // safety: 已由调用方按硬件能力注册
                        unsafe {
                            Renderer::$morph(&base, &coarse, blend, out);
                        }
                    }),
                ));
            }};
        }
        #[cfg(target_arch = "x86_64")]
        {
            if std::is_x86_feature_detected!("avx512f") {
                add_paths!("avx512", cull_spheres_avx512, morph_heights_avx512);
            }
            if std::is_x86_feature_detected!("avx2") {
                add_paths!("avx2", cull_spheres_avx2, morph_heights_avx2);
            }
            if std::is_x86_feature_detected!("avx") {
                add_paths!("avx", cull_spheres_avx, morph_heights_avx);
            }
            if std::is_x86_feature_detected!("sse4.2") {
                add_paths!("sse4.2", cull_spheres_sse, morph_heights_sse);
            }
        }
        {
            let (cx, cy, cz, radii, planes, base, coarse) =
                (cx.clone(), cy.clone(), cz.clone(), radii.clone(), planes, base.clone(), coarse.clone());
            cull_paths.push((
                "scalar",
                Box::new(move |out: &mut Vec<u32>| {
                    Renderer::cull_spheres_scalar(&cx, &cy, &cz, &radii, &planes, out);
                }),
            ));
            morph_paths.push((
                "scalar",
                Box::new(move |out: &mut [f32]| {
                    Renderer::morph_heights_scalar(&base, &coarse, blend, out);
                }),
            ));
        }

        let rounds = 200u32;
        let mut cull_out: Vec<Vec<u32>> = cull_paths.iter().map(|_| Vec::new()).collect();
        let mut morph_out: Vec<Vec<f32>> = morph_paths.iter().map(|_| vec![0.0f32; n]).collect();
        let bench = |paths: &[(&'static str, Box<dyn Fn(&mut Vec<u32>)>)],
                     outs: &mut [Vec<u32>]| -> Vec<u64> {
            let mut us = vec![0u64; paths.len()];
            for (i, (_, f)) in paths.iter().enumerate() {
                f(&mut outs[i]);
            }
            for (i, (_, f)) in paths.iter().enumerate() {
                let t0 = std::time::Instant::now();
                for _ in 0..rounds {
                    f(&mut outs[i]);
                    std::hint::black_box(&outs[i]);
                }
                us[i] = t0.elapsed().as_micros() as u64 / rounds as u64;
            }
            us
        };
        // 剔除基准
        let cull_us = bench(&cull_paths, &mut cull_out);
        let scalar_cull = *cull_us.last().unwrap();
        // morph 基准
        let mut morph_us = vec![0u64; morph_paths.len()];
        for (i, (_, f)) in morph_paths.iter().enumerate() {
            f(&mut morph_out[i]);
        }
        for (i, (_, f)) in morph_paths.iter().enumerate() {
            let t0 = std::time::Instant::now();
            for _ in 0..rounds {
                f(&mut morph_out[i]);
                std::hint::black_box(&morph_out[i]);
            }
            morph_us[i] = t0.elapsed().as_micros() as u64 / rounds as u64;
        }
        let scalar_morph = *morph_us.last().unwrap();
        // 逐位一致性（与各自标量对照）
        for (i, (name, _)) in cull_paths.iter().enumerate() {
            assert_eq!(&cull_out[i], cull_out.last().unwrap(), "{} 剔除与标量不一致", name);
        }
        for (i, (name, _)) in morph_paths.iter().enumerate() {
            assert_eq!(&morph_out[i], morph_out.last().unwrap(), "{} morph 与标量不一致", name);
        }
        println!("\n== cull SIMD 微基准（{} 实例 × {} 轮，release，单线程） ==", n, rounds);
        println!("{:<8}{:>14}{:>10}", "path", "us/round", "speedup");
        for (i, (name, _)) in cull_paths.iter().enumerate() {
            println!(
                "{:<8}{:>14}{:>9.2}x",
                name,
                cull_us[i],
                scalar_cull as f64 / cull_us[i].max(1) as f64
            );
        }
        println!("\n== morph SIMD 微基准（{} 点 × {} 轮，release，单线程） ==", n, rounds);
        println!("{:<8}{:>14}{:>10}", "path", "us/round", "speedup");
        for (i, (name, _)) in morph_paths.iter().enumerate() {
            println!(
                "{:<8}{:>14}{:>9.2}x",
                name,
                morph_us[i],
                scalar_morph as f64 / morph_us[i].max(1) as f64
            );
        }
    }
}

// ============================================================
// NPC 士兵可视化单元测试
// ============================================================

#[cfg(test)]
mod npc_visual_tests {
    use super::*;

    /// 读取列主序 model 数组的平移分量（model[12..15]）
    fn translation(m: &InstanceData) -> [f32; 3] {
        [m.model[12], m.model[13], m.model[14]]
    }

    /// 三几何分组：盒 9（脚×2/骨盆/胸廓/背心/头/头盔/枪身/枪托）+ 圆柱 8（四肢）= 17。
    /// **改 `soldier_part_matrices` 的身体计划就要同步改这里** —— 它是身体计划的守卫。
    #[test]
    fn soldier_parts_count_and_tint() {
        let tint = [0.2, 0.6, 0.9, 1.0];
        let (box_parts, cyl_parts, sph_parts) =
            Renderer::soldier_part_matrices([0.0, 0.0, 0.0], 0.0, tint, 0.0, false, false);
        assert_eq!(
            box_parts.len(),
            10,
            "盒体段应为 10（脚×2/骨盆/胸廓/背心/头/头盔/枪身/枪托/背包）"
        );
        assert_eq!(cyl_parts.len(), 8, "圆柱段应为 8（四肢×8）");
        assert_eq!(sph_parts.len(), 0, "不再有球体段：球头已被「方块头 + 头盔壳」取代");
        assert_eq!(box_parts.len() + cyl_parts.len() + sph_parts.len(), 18);
        // 段数预算守卫：压力模式 255 人 × 每组 3072 ⇒ 每组每人最多 12 段。
        // 没有这条断言，下次给士兵加装备就会在 255 人时静默截断（超出的段直接不画）。
        // 2026-09-12 加了背包（盒 9→10）：10×255 = 2550/3072 = 83%，仍留 17% 余量。
        // **上限 12 段**：再加盒段前先看这条断言会不会红。
        const NPC_HEADS: usize = 255;
        const PER_HEAD_MAX: usize = MAX_NPC_INSTANCES as usize / NPC_HEADS; // = 12
        assert_eq!(PER_HEAD_MAX, 12, "每组每人段数上限变了，下面的余量估算要重算");
        assert!(
            box_parts.len() <= PER_HEAD_MAX,
            "盒体段超预算：{} > {PER_HEAD_MAX}（{NPC_HEADS} 人时会静默截断）",
            box_parts.len()
        );
        assert!(
            cyl_parts.len() <= PER_HEAD_MAX,
            "圆柱段超预算：{} > {PER_HEAD_MAX}",
            cyl_parts.len()
        );
        // 逐段明暗（2026-09-12 第④条）：`tint` 不再是同一个值，而是**队色 × 本段系数**。
        // 旧断言是"所有段 tint == 队色"，它锁定的正是"整个人是一块均匀饱和色"那个缺陷
        // （实机放大图上就是一团橙色塑料）。改成锁四条不变量：
        //   ① alpha 不变；② 各通道不越界（∈ [0, 队色]）；
        //   ③ 色相比例不变（只许等比缩放，否则阵营色语义会漂移）；
        //   ④ 至少一段保持**完整队色**（远距离认阵营），且确实存在层次（不能全等）。
        let mut saw_full = false;
        let mut saw_variation = false;
        for p in box_parts.iter().chain(cyl_parts.iter()).chain(sph_parts.iter()) {
            assert_eq!(p.tint[3], tint[3], "段色 alpha 必须保持队色");
            for c in 0..3 {
                assert!(
                    p.tint[c] >= 0.0 && p.tint[c] <= tint[c] + 1e-6,
                    "段色越界：{:?} vs 队色 {:?}",
                    p.tint,
                    tint
                );
                // 比例不变：以通道 0 交叉相乘，避免除零
                let lhs = p.tint[c] * tint[0];
                let rhs = tint[c] * p.tint[0];
                assert!(
                    (lhs - rhs).abs() < 1e-5,
                    "段色改变了色相比例：{:?} vs 队色 {:?}",
                    p.tint,
                    tint
                );
            }
            if (p.tint[0] - tint[0]).abs() < 1e-6 {
                saw_full = true;
            } else {
                saw_variation = true;
            }
        }
        assert!(saw_full, "至少要有一段用完整队色，否则远距离认不出阵营");
        assert!(saw_variation, "逐段明暗必须有层次：全部相等就回到了'一块塑料'");
    }

    #[test]
    fn soldier_torso_height() {
        let (box_parts, _, _) =
            Renderer::soldier_part_matrices([0.0, 0.0, 0.0], 0.0, [1.0; 4], 0.0, false, false);
        // 盒体组第 4 段 = 胸廓（枢轴 y=1.26 + 段心 -0.01），yaw=0 时平移 y = 1.25
        let t = translation(&box_parts[3]);
        assert!(
            (t[1] - 1.25).abs() < 1e-3,
            "胸廓 y 应为 1.25，实际 {}",
            t[1]
        );
        assert!(t[0].abs() < 1e-3);
        assert!((t[2] + 0.01).abs() < 1e-3, "胸廓 z 应为 -0.01，实际 {}", t[2]);
    }

    /// 身体计划的**退化缩放守卫**：逐段检查实例矩阵三个基向量的长度。
    ///
    /// 存在的理由（2026-09-12 第 28 轮）：把游戏截图放大 3× 后发现士兵渲染成
    /// "一堆杂乱红方块 + 几条细长红色尖刺"。细到近乎 1px 的几何只可能来自**某一轴
    /// 缩放接近 0**。这条测试直接在矩阵上量，不必看图猜、也不必起游戏。
    ///
    /// `model` 列主序：列 i 占 `4i..4i+3`，所以三个基向量长度 = 三条轴的缩放。
    #[test]
    fn soldier_parts_have_no_degenerate_scale() {
        let (boxes, cyls, sphs) =
            Renderer::soldier_part_matrices([0.0; 3], 0.0, [1.0; 4], 0.0, false, false);
        let axis_len = |m: &[f32; 16], col: usize| -> f32 {
            let o = col * 4;
            (m[o] * m[o] + m[o + 1] * m[o + 1] + m[o + 2] * m[o + 2]).sqrt()
        };
        let mut bad: Vec<String> = Vec::new();
        for (i, p) in boxes.iter().enumerate() {
            let (sx, sy, sz) = (
                axis_len(&p.model, 0),
                axis_len(&p.model, 1),
                axis_len(&p.model, 2),
            );
            println!(
                "盒[{i}] scale=({sx:.3},{sy:.3},{sz:.3}) t=({:.2},{:.2},{:.2})",
                p.model[12], p.model[13], p.model[14]
            );
            if sx < 0.01 || sy < 0.01 || sz < 0.01 {
                bad.push(format!("盒[{i}]=({sx:.4},{sy:.4},{sz:.4})"));
            }
        }
        for (i, p) in cyls.iter().enumerate() {
            let (sx, sy, sz) = (
                axis_len(&p.model, 0),
                axis_len(&p.model, 1),
                axis_len(&p.model, 2),
            );
            println!(
                "柱[{i}] scale=({sx:.3},{sy:.3},{sz:.3}) t=({:.2},{:.2},{:.2})",
                p.model[12], p.model[13], p.model[14]
            );
            if sx < 0.01 || sy < 0.01 || sz < 0.01 {
                bad.push(format!("柱[{i}]=({sx:.4},{sy:.4},{sz:.4})"));
            }
        }
        for p in sphs.iter() {
            let _ = p;
        }
        assert!(
            bad.is_empty(),
            "存在退化缩放（某轴 ≈ 0，渲染出来就是尖刺）：{}",
            bad.join(", ")
        );
    }

    #[test]
    fn soldier_gun_rotates_with_yaw() {
        let (base, _, _) =
            Renderer::soldier_part_matrices([0.0, 0.0, 0.0], 0.0, [1.0; 4], 0.0, false, false);
        let (turned, _, _) = Renderer::soldier_part_matrices(
            [0.0, 0.0, 0.0],
            std::f32::consts::FRAC_PI_2,
            [1.0; 4],
            0.0,
            false,
            false,
        );
        // 盒体组第 8 段 = 枪身，局部 (x=+0.16, y=+1.18, z=+0.36)。
        // **不变式：枪指 +Z、并随 yaw 一起转。** 带横向偏移后，90° 把局部 (+0.16,+0.36)
        // 映到 (+0.36,-0.16) —— 所以 z 不再归零；旧表枪在 x=0，"z 应归零"当时成立纯属巧合，
        // 拿它当判据会把一个正确的旋转判错。
        let g0 = translation(&base[7]);
        let g90 = translation(&turned[7]);
        assert!((g0[0] - 0.16).abs() < 1e-3, "yaw=0 枪 x={}", g0[0]);
        assert!(
            (g0[2] - 0.36).abs() < 1e-3,
            "yaw=0 枪应伸向 +Z，z={}",
            g0[2]
        );
        assert!(
            (g90[0] - 0.36).abs() < 1e-3,
            "yaw=90° 枪应转到 +X，x={}",
            g90[0]
        );
        assert!(
            (g90[2] + 0.16).abs() < 1e-3,
            "yaw=90° 枪 z 应为 -0.16，z={}",
            g90[2]
        );
    }

    #[test]
    fn soldier_pos_translation_applies() {
        let (box_parts, _, _) = Renderer::soldier_part_matrices(
            [10.0, 2.0, -3.0],
            0.0,
            [1.0; 4],
            0.0,
            false,
            false,
        );
        // 盒体组第 8 段 = 枪身：平移 = pos + 局部 (0.16, 1.18, 0.36)
        let t = translation(&box_parts[7]);
        assert!((t[0] - 10.16).abs() < 1e-3, "x={}", t[0]);
        // 胸廓（盒体组第 4 段）：平移 = pos + (0, 1.25, -0.01)
        let tc = translation(&box_parts[3]);
        assert!((tc[1] - 3.25).abs() < 1e-3, "胸廓 y={}", tc[1]);
    }

    /// 尸体 15 段（14 段人体 + 横置枪），三几何分组，tint 保留
    #[test]
    fn dead_body_15_parts_with_tint() {
        let (box_parts, cyl_parts, sph_parts) =
            Renderer::dead_part_matrices([1.0, 0.0, 2.0], 0.0, [0.9, 0.1, 0.1, 1.0]);
        assert_eq!(box_parts.len(), 4, "尸体盒体段应为 4（脚×2/颈/枪）");
        assert_eq!(cyl_parts.len(), 10, "尸体圆柱段应为 10（四肢×8/骨盆/胸）");
        assert_eq!(sph_parts.len(), 1, "尸体球体段应为 1");
        assert_eq!(box_parts.len() + cyl_parts.len() + sph_parts.len(), 15);
        for p in box_parts.iter().chain(cyl_parts.iter()).chain(sph_parts.iter()) {
            assert_eq!(p.tint, [0.9, 0.1, 0.1, 1.0]);
        }
    }
}

/// `assets/mesh.spv` 的 **Workgroup 显式布局**回归守卫（2026-09-15）。
///
/// ## 守的是什么
///
/// `MeshShadingEXT` 要求 SPIR-V >= 1.4，而 `Offset` / `ArrayStride` 这类**显式布局装饰**
/// 在 SPIR-V ≤ 1.3 允许、**1.4 起对非 `Block` 类型禁止**。naga 30 的 SPIR-V 写入器
/// （`src/back/spv/writer.rs` 的 `decorate_struct_member`）**无条件**写 `Offset`，
/// 于是网格着色器一度是 7 个 `.spv` 里**唯一**过不了严格校验的那个：
///
/// ```text
/// spirv-val --target-env vulkan1.3 assets/mesh.spv
/// [VUID-StandaloneSpirv-None-10684] the Workgroup storage class has a explicit layout
/// from the Offset decoration
/// ```
///
/// `build.rs::strip_workgroup_explicit_layout` 在写出前把它去掉。这条测试**独立复算**
/// 一遍"从 Workgroup 变量出发的类型可达闭包"，所以下面三种情况都会让它变红：
/// ① 有人把 build.rs 里那次调用删了；② naga 升级后又多写了别的显式布局装饰；
/// ③ 网格着色器 WGSL 改出新形态的 Workgroup 类型。
///
/// ## 为什么"去掉"对 Workgroup 是无损的
///
/// Workgroup 内存**主机侧永远不碰**（`renderer.rs` 一级的实例/光照 buffer 都是
/// Uniform/StorageBuffer，那些带 `Block`、装饰**必须原样保留**），着色器访问一律走
/// `OpAccessChain` 的**成员索引**，字节偏移由驱动按 std430 自行推导 —— 只要同一个
/// 着色器内部一致，偏移取多少都不影响结果。反过来，动到带 `Block` 的类型上就是
/// **缓冲布局错位**，所以这条测试也顺带断言了那几个 `Block` 类型仍有 `Offset`。
#[cfg(test)]
mod workgroup_layout_tests {
    use std::collections::{HashMap, HashSet};

    const OP_TYPE_ARRAY: u32 = 28;
    const OP_TYPE_RUNTIME_ARRAY: u32 = 29;
    const OP_TYPE_STRUCT: u32 = 30;
    const OP_TYPE_POINTER: u32 = 32;
    const OP_VARIABLE: u32 = 59;
    const OP_DECORATE: u32 = 71;
    const OP_MEMBER_DECORATE: u32 = 72;
    const DECORATION_BLOCK: u32 = 2;
    const DECORATION_OFFSET: u32 = 35;
    const STORAGE_WORKGROUP: u32 = 4;
    /// `RowMajor` / `ColMajor` / `ArrayStride` / `MatrixStride` / `Offset`
    const EXPLICIT_LAYOUT: [u32; 5] = [4, 5, 6, 7, 35];

    fn words_of_mesh_spv() -> Vec<u32> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mesh.spv");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("读 {} 失败: {e}", path.display()));
        assert_eq!(bytes.len() % 4, 0, "SPIR-V 字节数必须是 4 的倍数");
        bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    struct Scan {
        /// 从 Workgroup 变量出发可达的类型
        workgroup_reachable: HashSet<u32>,
        /// 可达类型上的显式布局装饰（`(类型, 装饰)`）
        workgroup_layout: Vec<(u32, u32)>,
        /// 带 `Block` 装饰的类型
        block_types: HashSet<u32>,
        /// 带 `Block` 的类型的 `Offset` 装饰条数
        block_offsets: usize,
    }

    fn scan(words: &[u32]) -> Scan {
        assert_eq!(words[0], 0x0723_0203, "不是 SPIR-V 字流（魔数不符）");
        let mut structs: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut arrays: HashMap<u32, u32> = HashMap::new();
        let mut pointers: HashMap<u32, (u32, u32)> = HashMap::new();
        let mut seeds: Vec<u32> = Vec::new();
        let mut block_types: HashSet<u32> = HashSet::new();
        let mut decorations: Vec<(u32, u32, Option<u32>)> = Vec::new(); // (目标, 装饰, 成员)

        let mut i = 5usize;
        while i < words.len() {
            let count = (words[i] >> 16) as usize;
            let op = words[i] & 0xFFFF;
            let w = &words[i..i + count];
            match op {
                OP_TYPE_STRUCT => {
                    structs.insert(w[1], w[2..].to_vec());
                }
                OP_TYPE_ARRAY | OP_TYPE_RUNTIME_ARRAY => {
                    arrays.insert(w[1], w[2]);
                }
                OP_TYPE_POINTER => {
                    pointers.insert(w[1], (w[2], w[3]));
                }
                OP_VARIABLE if w[3] == STORAGE_WORKGROUP => {
                    if let Some(&(storage, pointee)) = pointers.get(&w[1]) {
                        if storage == STORAGE_WORKGROUP {
                            seeds.push(pointee);
                        }
                    }
                }
                OP_DECORATE => {
                    if w[2] == DECORATION_BLOCK {
                        block_types.insert(w[1]);
                    }
                    decorations.push((w[1], w[2], None));
                }
                OP_MEMBER_DECORATE => decorations.push((w[1], w[3], Some(w[2]))),
                _ => {}
            }
            i += count;
        }

        let mut reachable: HashSet<u32> = HashSet::new();
        let mut stack = seeds;
        while let Some(ty) = stack.pop() {
            if !reachable.insert(ty) {
                continue;
            }
            if let Some(members) = structs.get(&ty) {
                stack.extend(members.iter().copied());
            }
            if let Some(&elem) = arrays.get(&ty) {
                stack.push(elem);
            }
        }

        let mut workgroup_layout = Vec::new();
        let mut block_offsets = 0usize;
        for (target, decoration, _member) in decorations {
            if reachable.contains(&target) && EXPLICIT_LAYOUT.contains(&decoration) {
                workgroup_layout.push((target, decoration));
            }
            if block_types.contains(&target) && decoration == DECORATION_OFFSET {
                block_offsets += 1;
            }
        }
        Scan { workgroup_reachable: reachable, workgroup_layout, block_types, block_offsets }
    }

    /// 主语：网格着色器不得带任何 Workgroup 显式布局装饰（否则严格 `spirv-val` 拒载）。
    #[test]
    fn mesh_spirv_has_no_workgroup_explicit_layout() {
        let words = words_of_mesh_spv();
        let s = scan(&words);
        assert!(
            !s.workgroup_reachable.is_empty(),
            "mesh.spv 里找不到 Workgroup 变量 —— 网格着色器结构变了，这条守卫已失效，必须重写"
        );
        assert!(
            s.workgroup_layout.is_empty(),
            "mesh.spv 的 Workgroup 类型上仍有 {} 条显式布局装饰（类型/装饰：{:?}）。\n\
             修法：确认 build.rs::compile_wgsl_mesh 仍调用 strip_workgroup_explicit_layout；\n\
             若是 naga 新增了别的装饰种类，把它加进 build.rs 的 EXPLICIT_LAYOUT。\n\
             验证命令：spirv-val --target-env vulkan1.3 assets/mesh.spv",
            s.workgroup_layout.len(),
            s.workgroup_layout
        );
    }

    /// 反面：去掉装饰**只**能碰 Workgroup 可达类型。带 `Block` 的 Uniform / StorageBuffer /
    /// PushConstant 一旦被动，主机侧按同一份偏移写入的数据全部错位（静默、不报 VUID）。
    #[test]
    fn block_types_keep_their_offsets() {
        let words = words_of_mesh_spv();
        let s = scan(&words);
        assert!(
            !s.block_types.is_empty(),
            "mesh.spv 里找不到 Block 类型 —— 这条反向守卫已失效"
        );
        assert!(
            s.block_offsets > 0,
            "带 Block 的类型（{:?}）的 Offset 装饰被去掉了 —— 缓冲布局会错位",
            s.block_types
        );
        for ty in &s.block_types {
            assert!(
                !s.workgroup_reachable.contains(ty),
                "Block 类型 {ty} 同时是 Workgroup 可达的 —— 去掉规则会误伤它，必须先改 build.rs 的取舍"
            );
        }
    }
}

/// 水平面绕序回归守卫（2026-09-19）。
///
/// 本管线 `FrontFace::CLOCKWISE` + 着色器 Y 翻转：水平面**从上方可见 ⇔ (x,z) 有向面积 > 0**，
/// 以地面 quad 为参照。立方体顶/底面与 mesh 圆柱盖曾按相反约定绕序 ⇒ 顶面从上方恒被
/// 背面剔除，每个 marker 盒子实际是"顶面开口的盒子"：底面埋地的（喷泉池沿/水面、花坛、
/// 路缘石、碑座下两级）从开口露出地面，读作"坑"——追了两天的池子"坑"与柱头"管口"全是它。
/// 悬空盒子的"顶面"其实一直是透过开口看到的**底面**（平着色 + 法线翻向让它无从分辨）。
#[cfg(test)]
mod horizontal_winding_tests {
    use super::{Renderer, GROUND_INDICES, GROUND_VERTS, INDICES, VERTICES};

    /// 三角形在 (x,z) 平面的有向面积（正 = 与地面 quad 同绕序 = 从上方可见）。
    fn area_xz(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
        (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0])
    }

    #[test]
    fn ground_reference_and_cube_horizontal_faces_follow_it() {
        let g = |i: usize| GROUND_VERTS[GROUND_INDICES[i] as usize].pos;
        assert!(
            area_xz(g(0), g(1), g(2)) > 0.0,
            "地面 quad 自身绕序变了 —— 这条守卫失效，按新约定重写"
        );
        let v = |i: usize| VERTICES[INDICES[i] as usize].pos;
        for t in (24..30).step_by(3) {
            assert!(
                area_xz(v(t), v(t + 1), v(t + 2)) > 0.0,
                "立方体顶面 INDICES[{t}..] 与地面反绕 → 从上方被剔除（开口盒 bug 复发）"
            );
        }
        for t in (30..36).step_by(3) {
            assert!(
                area_xz(v(t), v(t + 1), v(t + 2)) < 0.0,
                "立方体底面 INDICES[{t}..] 从上方可见 → 盒子会被看穿"
            );
        }
    }

    #[test]
    fn cylinder_caps_follow_the_ground_winding() {
        let (verts, indices) = Renderer::cylinder_mesh_data();
        let pos = |i: u32| verts[i as usize].pos;
        // 每段 12 个索引：6 侧壁 + 3 上盖 + 3 下盖
        let segs = (indices.len() / 12) as u32;
        for i in 0..segs {
            let t = (i * 12 + 6) as usize;
            assert!(
                area_xz(pos(indices[t]), pos(indices[t + 1]), pos(indices[t + 2])) > 0.0,
                "圆柱上盖第 {i} 片从上方被剔除（柱头会读成管口）"
            );
            let b = (i * 12 + 9) as usize;
            assert!(
                area_xz(pos(indices[b]), pos(indices[b + 1]), pos(indices[b + 2])) < 0.0,
                "圆柱下盖第 {i} 片从上方可见（空心会被看穿）"
            );
        }
    }

    #[test]
    fn mesh_shader_horizontal_winding_matches_cpu() {
        let src = include_str!("../../build.rs");
        for pat in [
            "vec3<u32>(16u, 18u, 17u), vec3<u32>(16u, 19u, 18u)",
            "vec3<u32>(20u, 22u, 21u), vec3<u32>(20u, 23u, 22u)",
            "vec3<u32>(48u, i + 24u, ((i + 1u) % 24u) + 24u)",
            "vec3<u32>(49u, ((i + 1u) % 24u), i)",
        ] {
            assert!(
                src.contains(pat),
                "mesh 着色的水平面绕序与 CPU 不一致，缺 `{pat}`（两条路径必须同约定）"
            );
        }
    }
}

#[cfg(test)]
mod pt_prop_attrs_tests {
    use super::pt_bake_prop_attrs;

    fn dec_n(w: u32, shift: u32) -> f32 {
        (((w >> shift) & 0xFF) as f32 - 127.0) / 127.0
    }
    fn dec_c(w: u32, shift: u32) -> f32 {
        ((w >> shift) & 0xFF) as f32 / 255.0
    }
    fn v(pos: [f32; 3], col: [f32; 3]) -> [f32; 11] {
        let mut a = [0.0f32; 11];
        a[0..3].copy_from_slice(&pos);
        a[8..11].copy_from_slice(&col);
        a
    }

    #[test]
    fn quantization_roundtrip_stays_within_u8_bounds() {
        // 任意朝向三角：法线往返误差 ≤ 半格（1/127·0.5 容差放宽到 1/127），色 ≤ 1/255
        let verts = [
            v([0.0, 0.0, 0.0], [0.10, 0.50, 0.90]),
            v([1.0, 0.3, 0.0], [0.20, 0.55, 0.85]),
            v([0.2, 1.0, 0.7], [0.30, 0.45, 0.80]),
        ];
        let idx = [0u32, 1, 2];
        let a = pt_bake_prop_attrs(&verts, &idx);
        assert_eq!(a.len(), 2);
        let raw = {
            let e1 = [1.0f32, 0.3, 0.0];
            let e2 = [0.2f32, 1.0, 0.7];
            let n = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            [n[0] / l, n[1] / l, n[2] / l]
        };
        for (k, want) in raw.iter().enumerate() {
            let got = dec_n(a[0], (k as u32) * 8);
            assert!(
                (got - want).abs() <= 1.0 / 127.0 + 1e-6,
                "法线轴 {k} 往返误差越界: got {got} want {want}"
            );
        }
        for (k, want) in [(0u32, 0.20f32), (8, 0.5), (16, 0.85)] {
            let got = dec_c(a[1], k);
            assert!(
                (got - (want * 255.0).round() / 255.0).abs() < 1e-6,
                "色均值量化不是最近格点: shift {k} got {got}"
            );
        }
    }

    #[test]
    fn degenerate_triangle_falls_back_to_up_normal() {
        let verts = [
            v([0.0, 0.0, 0.0], [0.5, 0.5, 0.5]),
            v([2.0, 0.0, 0.0], [0.5, 0.5, 0.5]),
            v([1.0, 0.0, 0.0], [0.5, 0.5, 0.5]),
        ];
        let a = pt_bake_prop_attrs(&verts, &[0, 1, 2]);
        assert_eq!(dec_n(a[0], 0), 0.0);
        assert_eq!(dec_n(a[0], 8), 1.0);
        assert_eq!(dec_n(a[0], 16), 0.0);
    }

    #[test]
    fn tail_indices_below_one_full_triangle_are_dropped() {
        // 7 个索引 = 2 整角 + 1 尾：尾被丢弃 ⇒ 4 个字；
        // 与 prop_index_count(=7) 的等式把关会因此拒绝道具进 BLAS（宁缺不漏）
        let verts = [
            v([0.0, 0.0, 0.0], [0.4, 0.4, 0.4]),
            v([1.0, 0.0, 0.0], [0.4, 0.4, 0.4]),
            v([0.0, 1.0, 0.0], [0.4, 0.4, 0.4]),
            v([0.0, 0.0, 1.0], [0.4, 0.4, 0.4]),
        ];
        let a = pt_bake_prop_attrs(&verts, &[0, 1, 2, 0, 1, 3, 0]);
        assert_eq!(a.len(), 4);
        assert_ne!(a.len() as u32 / 2 * 3, 7, "等式把关对残缺索引必须不成立");
    }

    #[test]
    fn axis_aligned_faces_get_exact_normals() {
        // 水平面（+Y）与竖直面（+Z）的量化法线必须精确落在 ±1/0 格点上
        let up = [
            v([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
            v([1.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
            v([0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        ];
        let a = pt_bake_prop_attrs(&up, &[0, 2, 1]);
        assert_eq!(dec_n(a[0], 8), 1.0, "水平面法线必须是 +Y");
        assert_eq!(dec_n(a[0], 0), 0.0);
        assert_eq!(dec_n(a[0], 16), 0.0);
    }
}

#[cfg(test)]
mod vk_failure_path_tests {
    //! Vulkan **失败路径**的守卫（2026-09-22 复查新增）。
    //!
    //! 本仓这一年 GPU 侧的事故，修法全都是 `log` + 降级；而失败路径上有两种写法
    //! 会把"偶发的一次分配/映射失败"升级成更坏的状态：
    //! 1. 把 `expect` / `unwrap` 接在 `map_memory` / `create_buffer` / `allocate_memory`
    //!    这类调用后面 —— 直接 panic（切枪那一枪正好走这条路 = 整个进程没了）；
    //! 2. 用 `if let Ok(..)` 接 —— 失败被吞掉，后面的代码继续用**未初始化的显存**
    //!    （画错或设备消失，且不报错）。
    //!
    //! 这两条**没法用普通单测触发**（本机 `map_memory` 不会失败），所以退一步：
    //! 加源码检查把它们挡住。它是**检查，不是证明** —— 只管这两种写法。
    //! 判据：修之前这两条检查各自报出真实位置（枪模重映射两处 / 士兵索引映射一处），
    //! 修完转绿。
    //!
    //! ⚠️ 正文写在 `mod` 之内（`//!` 而不是 `///`）：外层的文档注释在 `mod` 行**之前**，
    //! 会被自己的扫描算进去 —— 那段文字里就写着这两个模式，自指 ⇒ 永远红。

    /// Vulkan 调用关键字（与真实写法逐字一致）
    const CALLS: [&str; 8] = [
        ".map_memory(",
        ".create_buffer(",
        ".allocate_memory(",
        ".bind_buffer_memory(",
        ".create_image(",
        ".create_image_view(",
        ".create_swapchain(",
        ".create_command_pool(",
    ];

    /// 只把**代码行**算进扫描：注释里为了解释这个坑，本来就要写出这两个模式，
    /// 不排除的话"解释它的注释"会自己踩线（第一次就是这么红的）。
    fn is_comment(line: &str) -> bool {
        let t = line.trim_start();
        t.starts_with("//") || t.starts_with("/*") || t.starts_with('*')
    }

    /// 第 `idx` 行（含）往前 `window` 行内是否出现过 Vulkan 调用；返回相隔行数
    fn vk_call_within(lines: &[&str], idx: usize, window: usize) -> Option<usize> {
        (0..=window).find(|&back| {
            idx.checked_sub(back)
                .and_then(|j| lines.get(j))
                .is_some_and(|l| !is_comment(l) && CALLS.iter().any(|c| l.contains(c)))
        })
    }

    /// 判据：`renderer.rs` 里对 Vulkan 调用的结果不许 `.expect()` / `.unwrap()`。
    /// 现状（2026-09-22 修完后）为 0 处；修前会红在两处枪模 `map_memory`（5353 / 5365 行）。
    #[test]
    fn no_expect_or_unwrap_on_vulkan_calls() {
        // ⚠️ 只扫**生产代码**：`include_str!` 会把本测试模块自身也读进来，
        // 而它正文里就写着 `.expect(` 这几个字（自指 ⇒ 这条检查永远红）。
        let full = include_str!("renderer.rs");
        let src = full.split("mod vk_failure_path_tests").next().unwrap_or("");
        let lines: Vec<&str> = src.lines().collect();
        // 先证明这条检查真的扫到了东西（否则文件被搬走/改名时会静默恒真）
        assert!(
            lines.iter().any(|l| l.contains(".map_memory(")),
            "检查失效：源码里一个 map_memory 都没扫到"
        );
        let mut bad = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if is_comment(line) {
                continue;
            }
            if !(line.contains(".expect(") || line.contains(".unwrap()")) {
                continue;
            }
            if let Some(back) = vk_call_within(&lines, i, 8) {
                bad.push(format!(
                    "第 {} 行（Vulkan 调用在 {} 行之前）: {}",
                    i + 1,
                    i + 1 - back,
                    line.trim()
                ));
            }
        }
        assert!(
            bad.is_empty(),
            "Vulkan 失败路径必须 log + 降级，不许 panic：\n{}",
            bad.join("\n")
        );
    }

    /// 判据：Vulkan 调用的失败不许被 `if let Ok(..)` **静默吞掉**。
    /// 修前会红在士兵网格的索引映射（`if let Ok(ip) = ...map_memory(...)`）——
    /// 那一处失败时索引一个都没写，函数却继续把 `soldier_index_count` 设成
    /// `indices.len()`，draw call 读未初始化显存。
    #[test]
    fn no_if_let_ok_swallowing_vulkan_calls() {
        let full = include_str!("renderer.rs");
        let src = full.split("mod vk_failure_path_tests").next().unwrap_or("");
        let lines: Vec<&str> = src.lines().collect();
        assert!(
            lines.iter().any(|l| l.contains(".map_memory(")),
            "检查失效：源码里一个 map_memory 都没扫到"
        );
        let mut bad = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if is_comment(line) {
                continue;
            }
            if !line.contains("if let Ok(") {
                continue;
            }
            if let Some(back) = vk_call_within(&lines, i, 8) {
                bad.push(format!(
                    "第 {} 行（Vulkan 调用在 {} 行之前）: {}",
                    i + 1,
                    i + 1 - back,
                    line.trim()
                ));
            }
        }
        assert!(
            bad.is_empty(),
            "Vulkan 调用的失败不许用 if let Ok 吞掉（要 log + 降级）：\n{}",
            bad.join("\n")
        );
    }
}

/// `queue_present` 结果分类的判据（2026-09-22 复查新增）。
///
/// 背景：呈现路径此前只认两种"需要重建交换链"的结果，**其余 `Err` 落空** ——
/// 于是 `ERROR_SURFACE_LOST_KHR` / `ERROR_DEVICE_LOST` 被当成呈现成功。
/// 这个分类是纯函数，所以这条真的能用单测钉住（不像上面的源码守卫）。
#[cfg(test)]
mod present_result_tests {
    use super::{classify_present, PresentOutcome};
    use super::{classify_acquire_err, AcquireOutcome};
    use super::{ACQUIRE_STALL_FALLBACK, ACQUIRE_STALL_MAX, ACQUIRE_TIMEOUT_NS};
    use super::FENCE_WAIT_TIMEOUT_NS;
    use super::{fence_stall_due, FENCE_STALL_MAX};
    use ash::vk;

    #[test]
    fn only_ok_false_counts_as_presented() {
        assert_eq!(classify_present(Ok(false)), PresentOutcome::Presented);
    }

    #[test]
    fn suboptimal_and_out_of_date_ask_for_a_new_swapchain() {
        assert_eq!(classify_present(Ok(true)), PresentOutcome::RecreateSwapchain);
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_OUT_OF_DATE_KHR)),
            PresentOutcome::RecreateSwapchain
        );
    }

    /// 这两条就是"改了 2026-09-22 之前会红"的判据：旧写法把它们当成功了。
    #[test]
    fn surface_lost_and_device_lost_are_failures() {
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_SURFACE_LOST_KHR)),
            PresentOutcome::Failed
        );
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_DEVICE_LOST)),
            PresentOutcome::Failed
        );
    }

    /// 🔴 **等待必须有上界**（2026-09-25 真机教训）：独显 + `defense_line` + IMMEDIATE 那次
    /// "游戏死了"的真身是主循环用 `u64::MAX` 无限等 acquire ⇒ 日志停住、无 panic、无 VUID。
    /// 这条断言把"不许再出现无限等待"钉死（改回 `u64::MAX` 立刻红）。
    #[test]
    fn swapchain_waits_are_bounded() {
        assert!(
            ACQUIRE_TIMEOUT_NS > 0 && ACQUIRE_TIMEOUT_NS <= 2_000_000_000,
            "acquire 超时必须有限且 ≤2s，实际 {}",
            ACQUIRE_TIMEOUT_NS
        );
        assert!(
            FENCE_WAIT_TIMEOUT_NS > 0 && FENCE_WAIT_TIMEOUT_NS <= 10_000_000_000,
            "围栏超时必须有限且 ≤10s，实际 {}",
            FENCE_WAIT_TIMEOUT_NS
        );
        assert!(
            ACQUIRE_STALL_FALLBACK >= 1 && ACQUIRE_STALL_FALLBACK < ACQUIRE_STALL_MAX,
            "降级阈值必须在放弃阈值之前（{} < {}）",
            ACQUIRE_STALL_FALLBACK,
            ACQUIRE_STALL_MAX
        );
    }

    /// 围栏超时的升级判据：前两次只报错（可能只是某一帧特别久），第三次才判定卡死。
    #[test]
    fn fence_stall_escalates_after_three_timeouts() {
        assert!(!fence_stall_due(0));
        assert!(!fence_stall_due(1));
        assert!(!fence_stall_due(FENCE_STALL_MAX - 1));
        assert!(fence_stall_due(FENCE_STALL_MAX));
        assert!(fence_stall_due(FENCE_STALL_MAX + 5));
    }

    /// acquire 的错误分类：只有"这轮没图像"能重试；OUT_OF_DATE/SURFACE_LOST 要重建；
    /// DEVICE_LOST 之类必须当失败（旧写法把它们全都静默丢进 `?` 的 map 里）。
    #[test]
    fn acquire_errors_are_classified() {
        assert_eq!(
            classify_acquire_err(vk::Result::TIMEOUT),
            AcquireOutcome::Retry
        );
        assert_eq!(
            classify_acquire_err(vk::Result::NOT_READY),
            AcquireOutcome::Retry
        );
        assert_eq!(
            classify_acquire_err(vk::Result::ERROR_OUT_OF_DATE_KHR),
            AcquireOutcome::RecreateSwapchain
        );
        assert_eq!(
            classify_acquire_err(vk::Result::ERROR_SURFACE_LOST_KHR),
            AcquireOutcome::RecreateSwapchain
        );
        assert_eq!(
            classify_acquire_err(vk::Result::ERROR_DEVICE_LOST),
            AcquireOutcome::Failed
        );
    }
}

/// 并行剔除**段数上限**的判据（2026-09-22 复查新增）。
///
/// `cull_and_upload` 的两张前缀和表是栈上定长 `[u32; CULL_MAX_SEGMENTS]`，
/// 段数必须 ≤ 表长。旧写法是裸的 `pool.workers() + 1`，只有一句
/// `debug_assert!(nw <= 64)` 兜着 —— 而 **release 里 `debug_assert` 不存在**
/// ⇒ 64 个 worker 以上的机器（64C/128T 起）每帧 `index out of bounds`。
#[cfg(test)]
mod cull_segment_tests {
    use super::{cull_segment_count, CULL_MAX_SEGMENTS};

    #[test]
    fn segment_count_is_workers_plus_one_within_limit() {
        // 本机这一类（16C 级）与旧公式逐值一致 ⇒ 改动是空操作
        assert_eq!(cull_segment_count(0), 1, "0 个 worker 也要有 1 段（调用线程自己跑）");
        assert_eq!(cull_segment_count(3), 4);
        assert_eq!(cull_segment_count(31), 32);
        // 边界：刚好填满表
        assert_eq!(cull_segment_count(CULL_MAX_SEGMENTS - 1), CULL_MAX_SEGMENTS);
        // 🔴 这一条是"修之前会红"的判据：旧公式在这里给 65 / 1001
        assert_eq!(cull_segment_count(CULL_MAX_SEGMENTS), CULL_MAX_SEGMENTS);
        assert_eq!(cull_segment_count(1000), CULL_MAX_SEGMENTS);
    }

    /// 上限本身：低于 64 只会让"每核一段"的机器少用几个核（慢一点，结果不变），
    /// 但不该被顺手调小 —— 顺带锁住它的量级。
    #[test]
    fn limit_covers_common_topologies() {
        assert!(
            CULL_MAX_SEGMENTS >= 64,
            "段数上限被调小了：常见 32C64T 拓扑会退化成段数不足（只是少并行，不会算错）"
        );
    }
}

/// `RV3D_GPU`（物理设备选择）的判据（2026-09-23 加）。
///
/// 起因：本机是双 GPU 笔记本，验证时 dGPU 可能被占（用户在跑 AI）⇒ 需要一个"强制走核显"的开关。
/// 这两条纯函数把"怎么解析"与"怎么挑"分开，于是**不需要真显卡就能单测**。
#[cfg(test)]
mod gpu_pick_tests {
    use super::{parse_gpu_preference, pick_physical_device, GpuPreference};
    use ash::vk;

    fn dgpu() -> (vk::PhysicalDeviceType, String) {
        (
            vk::PhysicalDeviceType::DISCRETE_GPU,
            "NVIDIA GeForce RTX 5060 Laptop GPU".to_string(),
        )
    }
    fn igpu() -> (vk::PhysicalDeviceType, String) {
        (
            vk::PhysicalDeviceType::INTEGRATED_GPU,
            "AMD Radeon 610M (integrated)".to_string(),
        )
    }

    #[test]
    fn parse_gpu_preference_maps_known_aliases() {
        assert_eq!(parse_gpu_preference(None), GpuPreference::Auto);
        assert_eq!(parse_gpu_preference(Some("")), GpuPreference::Auto);
        assert_eq!(parse_gpu_preference(Some("   ")), GpuPreference::Auto, "空白 = 未设");
        assert_eq!(parse_gpu_preference(Some("igpu")), GpuPreference::Integrated);
        assert_eq!(parse_gpu_preference(Some("IGPU")), GpuPreference::Integrated);
        assert_eq!(
            parse_gpu_preference(Some("integrated")),
            GpuPreference::Integrated
        );
        assert_eq!(parse_gpu_preference(Some("dgpu")), GpuPreference::Discrete);
        assert_eq!(
            parse_gpu_preference(Some("discrete")),
            GpuPreference::Discrete
        );
        // 其它值 = 名字子串，统一转小写
        assert_eq!(
            parse_gpu_preference(Some("  Radeon ")),
            GpuPreference::Name("radeon".to_string())
        );
    }

    #[test]
    fn pick_physical_device_honours_preference() {
        let both = [dgpu(), igpu()];
        // 🔴 默认必须是独显（历史行为，不许因为加了开关而改变）
        assert_eq!(pick_physical_device(&both, &GpuPreference::Auto), Some(0));
        assert_eq!(pick_physical_device(&both, &GpuPreference::Discrete), Some(0));
        assert_eq!(pick_physical_device(&both, &GpuPreference::Integrated), Some(1));
        assert_eq!(
            pick_physical_device(&both, &GpuPreference::Name("radeon".into())),
            Some(1)
        );
        assert_eq!(
            pick_physical_device(&both, &GpuPreference::Name("nvidia".into())),
            Some(0)
        );
        // 匹配不到 ⇒ None（调用方据此**报错退出**，绝不静默回退到独显）
        assert_eq!(
            pick_physical_device(&both, &GpuPreference::Name("intel".into())),
            None
        );
        // 只有集显的机器上要独显 ⇒ 也是 None
        assert_eq!(
            pick_physical_device(&[igpu()], &GpuPreference::Discrete),
            None
        );
        // 顺序无关：集显在前的列表里 Auto 仍选独显
        let flipped = [igpu(), dgpu()];
        assert_eq!(pick_physical_device(&flipped, &GpuPreference::Auto), Some(1));
    }
}
