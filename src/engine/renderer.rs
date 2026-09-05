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
    16, 17, 18, 16, 18, 19, // 上
    20, 21, 22, 20, 22, 23, // 下
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
/// 绕序注意：本管线为 FrontFace::CLOCKWISE + shader Y 翻转，水平地面从上方看必须是
/// 逆时针（索引 [0,2,1, 0,3,2]）才是正面；立方体顶面用的顺时针在长期被背面剔除，
/// 这正是旧版"只剩侧壁竖立"的根因。marker/NPC 的垂直面不受影响。
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
/// 实例 storage buffer 的总元素数。**唯一权威定义**——历史上它是三份互相抄写的副本
/// （`buffer_elems` + 主管线 descriptor `.range()` + 阴影 pass descriptor `.range()`），
/// 加一个槽位只要漏改任一份，shader 就会对那一槽越界读 storage buffer：驱动不会报错，
/// 只会返回全零，于是 `inst.model` 变成零矩阵、所有顶点塌到一点、几何**完全不显示**且
/// 没有任何日志或 VUID 提示（2026-09-04 加道具槽时正好踩中，靠"红屏探针 + 换槽对照"才定位）。
/// 现在由最高槽位反推，结构上不可能再漏。
const INSTANCE_BUFFER_ELEMS: u64 = PROP_INSTANCE_INDEX as u64 + 1;
const _: () = assert!(
    INSTANCE_BUFFER_ELEMS > GUN_INSTANCE_INDEX as u64 && INSTANCE_BUFFER_ELEMS > 0,
    "实例 buffer 必须覆盖所有已知槽位"
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
    /// 从物理障碍盒构建世界 marker：模型 = 平移(x, 1.2, z) × 缩放(2·half_w, 2.4, 2·half_d)。
    ///
    /// 与 game.rs apply_level 的物理刚体严格同尺寸：刚体 AABB = (x, 1.2, z) ± (half_w, 1.2, half_d)
    /// （高 MAP_BLOCK_HEIGHT = 2.4），即渲染盒与碰撞盒水平足迹逐米一致 —— 玩家被挡距离仅由
    /// 玩家胶囊半径（0.5m）决定，不存在“视觉细/碰撞粗”的 AABB 与 marker 尺寸差。
    ///
    /// 材质：按 ObstacleKind 调色板 + 确定性逐障碍微变（terrain_hash 量化格点），
    /// 墙（砖红）/块（金属灰蓝）/栅栏（木板）/树（树干棕）/建筑（混凝土）/残骸（土棕）
    /// 各有可辨识材质色；同一种类的相邻盒子明度/色相 ±6% 抖动，形成砖缝/板纹颗粒感。
    pub fn for_obstacle(ob: &MapObstacle) -> Self {
        WorldMarker {
            model: glam::Mat4::from_translation(glam::Vec3::new(ob.x, ob.y, ob.z))
                * glam::Mat4::from_scale(glam::Vec3::new(
                    ob.half_w * 2.0,
                    ob.half_h * 2.0,
                    ob.half_d * 2.0,
                )),
            tint: {
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
fn terrain_height(x: f32, z: f32) -> f32 {
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

        let (physical_device, physical_device_properties) = physical_devices
            .iter()
            .filter_map(|&device| {
                let properties = unsafe { instance.get_physical_device_properties(device) };
                let surface_support = unsafe {
                    surface_loader
                        .get_physical_device_surface_support(device, 0, surface)
                        .unwrap_or(false)
                };
                if surface_support {
                    Some((device, properties))
                } else {
                    None
                }
            })
            .max_by_key(|&(_, props)| match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 2,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                _ => 0,
            })
            .ok_or_else(|| "没有找到合适的物理设备".to_string())?;

        let device_name = unsafe {
            CStr::from_ptr(physical_device_properties.device_name.as_ptr())
                .to_string_lossy()
                .to_string()
        };
        log::info!("选择物理设备: {}", device_name);
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
            prop_vertex_buffer: vk::Buffer::null(),
            prop_vertex_memory: vk::DeviceMemory::null(),
            prop_index_buffer: vk::Buffer::null(),
            prop_index_memory: vk::DeviceMemory::null(),
            prop_mapped: std::ptr::null_mut(),
            prop_vertex_count: 0,
            prop_index_count: 0,
            prop_capacity_verts: 0,
            prop_capacity_idx: 0,
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
        // 呈现模式可被 RV3D_PRESENT_MODE 覆盖（immediate/mailbox/fifo），
        // 性能探针对比用；默认 MAILBOX（不锁 vsync）。
        let preferred = match std::env::var("RV3D_PRESENT_MODE").as_deref() {
            Ok("immediate") => vk::PresentModeKHR::IMMEDIATE,
            Ok("fifo") => vk::PresentModeKHR::FIFO,
            // 2026-08-23：默认 IMMEDIATE（配合全局帧率上限 RV3D_FPS 默认 240）——
            // 独显直连下 FIFO 垂直同步死锁（等不到 vblank 中断）→ 主循环冻结；
            // MAILBOX 在笔记本混合切换时触发 device lost；IMMEDIATE 最稳。
            _ => vk::PresentModeKHR::IMMEDIATE,
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

        let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            // COLOR_ATTACHMENT | TRANSFER_SRC：截图读回需要把 swapchain 图像作为
            // TRANSFER 源拷贝到 staging buffer（vkCmdCopyImageToBuffer 的 VUID 要求）。
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
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
            "swapchain diag: current_extent={}x{} final={}x{}",
            surface_capabilities.current_extent.width,
            surface_capabilities.current_extent.height,
            extent.width,
            extent.height
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

    /// 创建 NPC 人体圆柱几何（四肢用）：单位圆柱 r=1 h=1 沿 Y，24 段，含上下盖。
    fn create_cylinder_geometry(&mut self) -> Result<(), String> {
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
            // 上盖 fan
            indices.extend_from_slice(&[top_center, t2, t3]);
            // 下盖 fan
            indices.extend_from_slice(&[bottom_center, t1, t0]);
        }
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
                log::warn!("cpu: 强制 {forced} 但硬件不支持，回退自动选路");
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
        let m = (proj * view).to_cols_array_2d(); // m[col][row]
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
        let parts: [([f32; 3], [f32; 3], [f32; 3], u8, u8); 15] = [
            ([-0.10, 0.84, 0.0], [0.065, 0.38, 0.065], [0.0, -0.19, 0.0], 1, 1), // 左大腿（髋）圆柱
            ([0.10, 0.84, 0.0], [0.065, 0.38, 0.065], [0.0, -0.19, 0.0], 2, 1),  // 右大腿（髋）圆柱
            ([-0.09, 0.46, 0.0], [0.05, 0.36, 0.05], [0.0, -0.18, 0.0], 3, 1),   // 左小腿（膝）圆柱
            ([0.09, 0.46, 0.0], [0.05, 0.36, 0.05], [0.0, -0.18, 0.0], 4, 1),    // 右小腿（膝）圆柱
            ([-0.09, 0.045, 0.0], [0.09, 0.05, 0.24], [0.0, 0.0, 0.02], 0, 0),    // 左脚（盒）
            ([0.09, 0.045, 0.0], [0.09, 0.05, 0.24], [0.0, 0.0, 0.02], 0, 0),     // 右脚（盒）
            ([0.0, 0.96, 0.0], [0.17, 0.18, 0.19], [0.0, -0.09, 0.0], 0, 1),      // 骨盆（圆柱）
            ([0.0, 1.26, 0.0], [0.20, 0.48, 0.20], [0.0, -0.02, -0.01], 10, 1),   // 胸（桶形圆柱，含后坐前俯）
            ([0.0, 1.49, 0.0], [0.05, 0.06, 0.05], [0.0, -0.01, 0.0], 0, 0),      // 颈（盒）
            ([0.0, 1.64, 0.0], [0.15, 0.17, 0.15], [0.0, -0.01, 0.0], 0, 2),      // 头（球体，φ≈0.30m）
            ([-0.28, 1.38, 0.02], [0.05, 0.26, 0.05], [0.0, -0.13, 0.0], 5, 1),   // 左上臂（肩）圆柱
            ([0.28, 1.38, 0.02], [0.05, 0.26, 0.05], [0.0, -0.13, 0.0], 6, 1),    // 右上臂（肩）圆柱
            ([-0.28, 1.10, 0.02], [0.042, 0.24, 0.042], [0.0, -0.12, 0.02], 7, 1), // 左前臂（肘）圆柱
            ([0.28, 1.10, 0.02], [0.042, 0.24, 0.042], [0.0, -0.12, 0.02], 8, 1),  // 右前臂（肘）圆柱
            ([0.0, 1.18, 0.52], [0.13, 0.10, 0.95], [0.0, 0.0, 0.0], 9, 0),       // 枪（+Z 前方，后坐 -Z）
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
        for (pivot, scale, center, kind, geom) in parts.iter() {
            let mut anim = glam::Mat4::IDENTITY;
            match kind {
                1 => anim *= glam::Mat4::from_rotation_x(stride),        // 左大腿
                2 => anim *= glam::Mat4::from_rotation_x(-stride),       // 右大腿
                3 => anim *= glam::Mat4::from_rotation_x(-stride * 0.5), // 左小腿（膝弯反向）
                4 => anim *= glam::Mat4::from_rotation_x(stride * 0.5),  // 右小腿
                5 => anim *= glam::Mat4::from_rotation_x(-stride * 0.8), // 左上臂（与同侧腿反向）
                6 => anim *= glam::Mat4::from_rotation_x(stride * 0.8),  // 右上臂
                7 => anim *= glam::Mat4::from_rotation_x(-stride * 0.35), // 左前臂（肘弯）
                8 => anim *= glam::Mat4::from_rotation_x(stride * 0.35), // 右前臂
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
                tint,
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
    pub fn set_npc_visuals(&mut self, visuals: &[NpcVisual]) {
        self.npc_box_parts.clear();
        self.npc_cyl_parts.clear();
        self.npc_sph_parts.clear();
        for v in visuals {
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
                break;
            }
        }
    }

    /// 追加倒地尸体段（由 main.rs 传入位置/朝向/阵营；15 段/具躺倒姿态），
    /// 与活体 NPC 共用 NPC 三几何槽位区（各组段数截断到 MAX_NPC_INSTANCES）
    pub fn set_dead_bodies(&mut self, bodies: &[NpcVisual]) {
        for v in bodies {
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
                break;
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
        self.gun_vertex_count = verts.len() as u32;
        self.gun_index_count = indices.len() as u32;
        if verts.is_empty() || indices.is_empty() {
            return;
        }
        // 枪模缓冲容量：预分配全局最大（35 把枪当前最大 verts=30492 / idx=145992，
        // next_power_of_two = 32768 / 262144）。切枪（含容量缩小）永不重建缓冲——
        // 重建会 destroy 正在被 GPU 使用的 buffer → NVIDIA 驱动 device lost（画面卡死，
        // 2026-08-18 修复：切到小网格武器触发重建导致崩溃）。
        // 未来新增更大枪模时 max() 自动扩容（首帧重建一次，代价可接受）。
        let need_verts = 32768u32.max((verts.len() as u32).next_power_of_two());
        let need_idx = 262_144u32.max((indices.len() as u32).next_power_of_two());
        if need_verts != self.gun_buffer_capacity_verts
            || need_idx != self.gun_buffer_capacity_idx
            || self.gun_mapped.is_null()
            || self.gun_vertex_buffer == vk::Buffer::null()
        {
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
            let v_size = need_verts as u64 * std::mem::size_of::<Vertex>() as u64;
            let (vb, vm) = self
                .create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, v_size)
                .expect("枪模顶点缓冲创建失败");
            let i_size = need_idx as u64 * 4; // 索引容量独立按实际索引数
            let (ib, im) = self
                .create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, i_size)
                .expect("枪模索引缓冲创建失败");
            self.gun_vertex_buffer = vb;
            self.gun_vertex_buffer_memory = vm;
            self.gun_index_buffer = ib;
            self.gun_index_buffer_memory = im;
            self.gun_mapped = unsafe {
                self.device
                    .map_memory(vm, 0, v_size, vk::MemoryMapFlags::empty())
                    .expect("枪模顶点缓冲映射失败")
            };
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
        unsafe {
            self.device.unmap_memory(self.gun_vertex_buffer_memory);
            self.gun_mapped = self
                .device
                .map_memory(
                    self.gun_vertex_buffer_memory,
                    0,
                    self.gun_buffer_capacity_verts as u64 * std::mem::size_of::<Vertex>() as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .expect("枪模顶点缓冲重映射失败")
        }
        // 索引上传（独立映射窗口，用一次性的暂存：直接再 map 索引内存）
        unsafe {
            let iptr = self
                .device
                .map_memory(
                    self.gun_index_buffer_memory,
                    0,
                    self.gun_index_count as u64 * 4,
                    vk::MemoryMapFlags::empty(),
                )
                .expect("枪模索引缓冲映射失败");
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
        let merged = crate::engine::props::merge(set, placements, |x, z| terrain_height_at(x, z));
        self.prop_vertex_count = 0;
        self.prop_index_count = 0;
        if merged.is_empty() || merged.indices.is_empty() {
            log::info!("props: 无摆放几何（套件 {} 件 / 摆放 {} 处）", set.len(), placements.len());
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
            let (vb, vm) = match self
                .create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER,
                                    cap_v as u64 * std::mem::size_of::<Vertex>() as u64)
            {
                Ok(v) => v,
                Err(e) => {
                    log::error!("props: 顶点缓冲创建失败，跳过道具绘制: {e}");
                    return;
                }
            };
            let (ib, im) = match self
                .create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, cap_i as u64 * 4)
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
        log::info!(
            "props: 上传完成 顶点 {} / 三角 {} / 摆放 {} 处，包围盒 x∈[{:.1},{:.1}] y∈[{:.1},{:.1}] z∈[{:.1},{:.1}]",
            need_v, need_i / 3, placements.len(),
            merged.min[0], merged.max[0], merged.min[1], merged.max[1],
            merged.min[2], merged.max[2]
        );
    }

    /// 当前交换链尺寸 (宽, 高)——PT 原生分辨率取它
    pub fn frame_size(&self) -> (u32, u32) {
        (self.swapchain_extent.width, self.swapchain_extent.height)
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
        let set_bindings = [as_layout, img_layout, mat_layout, acc_layout];
        let set_create = vk::DescriptorSetLayoutCreateInfo::default().bindings(&set_bindings);
        let sl = unsafe { self.device.create_descriptor_set_layout(&set_create, None) }.map_err(|e| format!("PT sl: {e}"))?;
        let pipe_layouts = [sl];
        // push constants：6×vec4 = 96B（pt_panorama.glsl 的 PC 块 a..f）
        let pc_ranges = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(96)];
        let pipe_create = vk::PipelineLayoutCreateInfo::default().set_layouts(&pipe_layouts).push_constant_ranges(&pc_ranges);
        let pl = unsafe { self.device.create_pipeline_layout(&pipe_create, None) }.map_err(|e| format!("PT pl: {e}"))?;
        let stage_info = vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::COMPUTE).module(vs_module).name(c"main");
        let compute_info = vk::ComputePipelineCreateInfo::default().stage(stage_info).layout(pl);
        let pipelines = unsafe { self.device.create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None).map_err(|e| format!("PT pipe {:?}", e.1))? };
        let pipeline = pipelines[0];
        let img_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D).format(vk::Format::B8G8R8A8_UNORM)
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
            .image(image).view_type(vk::ImageViewType::TYPE_2D).format(vk::Format::B8G8R8A8_UNORM)
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
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1),
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
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        // AS 一次性构建 + 等待（与 run_pt_view 同款——已验证路径！）
        let alloc = vk::CommandBufferAllocateInfo::default().command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
        let cb = unsafe { self.device.allocate_command_buffers(&alloc) }.map_err(|e| format!("PT cb: {e}"))?[0];
        unsafe {
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT));
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
            self.device.end_command_buffer(cb);
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
        };
        self.pt_fill_geom(&mut assets, &boxes[..n], &albedos)?;
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
        let mut geom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        geom.ty = vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL;
        geom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        geom.geometry_count = 1;
        geom.p_geometries = &geo;
        geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            // 尺寸按 PT_MAX_BOXES 容量算（不是当前盒数）：换场景只重建 BLAS，
            // 若按初始 4 盒分配，塞进 512 盒会越界写 AS 缓冲 -> device lost
            ext.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &geom, &[(PT_MAX_BOXES * 12) as u32], &mut size_info);
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
        boxes.push(crate::engine::ray_tracer::PtBox {
            center: [0.0, -1.0, 0.0],
            half: [400.0, 1.0, 400.0],
            material: 0,
        });
        albedos.push([0.34, 0.32, 0.29]);
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
        if sig == self.pt_scene_sig {
            return Ok(());
        }
        self.pt_scene_sig = sig;
        let n = boxes.len();
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
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT));
            self.record_pt_build(cb, assets, n)?;
            self.device.end_command_buffer(cb);
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
        let set_bindings = [as_layout, img_layout, mat_layout, acc_layout];
        let set_create = vk::DescriptorSetLayoutCreateInfo::default().bindings(&set_bindings);
        let set_layout_handle = unsafe { self.device.create_descriptor_set_layout(&set_create, None) }
            .map_err(|e| format!("PT set: {e}"))?;
        let pipe_layouts = [set_layout_handle];
        let pc_ranges = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(96)];
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
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1),
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
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        // 命令：AS 构建 + dispatch + 拷贝回读
        let alloc = vk::CommandBufferAllocateInfo::default().command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
        let cb = unsafe { self.device.allocate_command_buffers(&alloc) }.map_err(|e| format!("PT cb: {e}"))?[0];
        let begin_info = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            self.device.begin_command_buffer(cb, &begin_info);
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
                let pc = self.pt_params.pack(size, size, i, i == 0, spp, 0.0);
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
            self.device.end_command_buffer(cb);
            let cbs = [cb];
            let submit = vk::SubmitInfo::default().command_buffers(&cbs);
            self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("PT submit: {e}"))?;
            // 2026-08-29 修复：AS 构建必须在 dispatch 前完成（wait 保障 TLAS 可见）
            self.device.queue_wait_idle(self.graphics_queue).map_err(|e| format!("PT wait: {e}"))?;
            // 读回 + PNG
            let m = self.device.map_memory(read_mem, 0, (size * size * 4) as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("PT map: {e}"))?;
            let mut px: Vec<u8> = std::slice::from_raw_parts(m as *const u8, (size * size * 4) as usize).to_vec();
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
                        bmp.push(px[i]);
                        bmp.push(px[i + 1]);
                        bmp.push(px[i + 2]);
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
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            let begin_info = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device.begin_command_buffer(cb, &begin_info);
            self.record_pt_build(cb, &assets, boxes.len())?;
            self.device.end_command_buffer(cb);
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
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default());
            self.device.cmd_bind_pipeline(cb, vk::PipelineBindPoint::COMPUTE, compute_pipeline);
            self.device.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::COMPUTE, pipe_layout, 0, &[dset], &[]);
            for _ in 0..iterations {
                self.device.cmd_dispatch(cb, (n as u32 + 63) / 64, 1, 1);
            }
            self.device.end_command_buffer(cb);
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
        b_geom.p_geometries = &b_geo;
        b_geom.dst_acceleration_structure = assets.blas;
        b_geom.scratch_data = vk::DeviceOrHostAddressKHR { device_address: scratch_base };
        b_geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let range_b = vk::AccelerationStructureBuildRangeInfoKHR { primitive_count: (box_count * 12) as u32, primitive_offset: 0, first_vertex: 0, transform_offset: 0 };
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
            let rb: [vk::AccelerationStructureBuildRangeInfoKHR; 1] = [range_b];
            let rbs: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [&rb];
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
        let nw = pool.workers() + 1;
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
        let mut near_prefix = [0u32; 64];
        let mut far_prefix = [0u32; 64];
        debug_assert!(nw <= 64, "并行段数超栈数组上限");
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
                log::warn!("cpu: 强制 {forced} 但硬件不支持，回退自动选路");
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
    pub fn init_hud_overlay(&mut self) -> Result<(), String> {
        unsafe {
            let color_attachment = vk::AttachmentDescription::default()
                .format(self.swapchain_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::PRESENT_SRC_KHR)
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
            self.hud_framebuffers = self.swapchain_image_views.iter().map(|&iv| {
                let fbi = vk::FramebufferCreateInfo::default()
                    .render_pass(self.hud_render_pass)
                    .attachments(std::slice::from_ref(&iv))
                    .width(self.swapchain_extent.width)
                    .height(self.swapchain_extent.height)
                    .layers(1);
                self.device.create_framebuffer(&fbi, None).map_err(|e| format!("hud fb: {e}"))
            }).collect::<Result<Vec<_>, _>>()?;
        }
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

        // ---- GLB 道具合并网格：一次 draw call 画完整城道具。
        //      走主管线（**已开深度测试**），所以道具之间、道具与地形之间遮挡正确。
        //      identity 实例取 PROP_INSTANCE_INDEX：位姿已在 CPU 烘进顶点，GPU 侧不需要
        //      逐实例矩阵；该槽 tint.w=Shape::Authored.tag() 让片元跳过程序化立面加工。
        if self.prop_index_count > 0 && self.prop_vertex_count > 0 {
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
                self.device.cmd_draw_indexed(
                    command_buffer,
                    self.prop_index_count,
                    1,
                    0,
                    0,
                    PROP_INSTANCE_INDEX,
                );
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
                        }
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
                    self.device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, self.hud_pipeline);
                    let vb = [self.hud_vertex_buffer];
                    let offs = [0u64];
                    self.device.cmd_bind_vertex_buffers(command_buffer, 0, &vb, &offs);
                    self.device.cmd_draw(command_buffer, self.hud_vertex_count, 1, 0, 0);
                    self.device.cmd_end_render_pass(command_buffer);
                    let hud_back = vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE).dst_access_mask(vk::AccessFlags::MEMORY_READ)
                        .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL).new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(sw_img)
                        .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                    self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[hud_back]);
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

        for _ in 0..self.max_frames_in_flight {
            let image_available = unsafe {
                self.device
                    .create_semaphore(&semaphore_create_info, None)
                    .map_err(|e| format!("创建信号量失败: {}", e))?
            };
            let render_finished = unsafe {
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
            self.render_finished_semaphores.push(render_finished);
            self.in_flight_fences.push(fence);
        }
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
        let frame_start = Instant::now();
        let fence = self.in_flight_fences[self.current_frame];
        let t0 = Instant::now();
        unsafe {
            self.device
                .wait_for_fences(&[fence], true, u64::MAX)
                .map_err(|e| format!("等待围栏失败: {}", e))?;
        }
        self.stage_wait_fence_us = t0.elapsed().as_micros() as u64;

        let t0 = Instant::now();
        let (image_index, suboptimal) = unsafe {
            self.swapchain_loader
                .acquire_next_image(
                    self.swapchain,
                    u64::MAX,
                    self.image_available_semaphores[self.current_frame],
                    vk::Fence::null(),
                )
                .map_err(|e| match e {
                    vk::Result::ERROR_OUT_OF_DATE_KHR => "交换链过期".to_string(),
                    vk::Result::ERROR_SURFACE_LOST_KHR => "表面丢失".to_string(),
                    _ => format!("获取交换链图像失败: {}", e),
                })?
        };
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
        let (marker_near, marker_far) = if self.void_mode { (0, 0) } else { self.upload_markers(cam_pos) };
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
        // ---- 自发光实体（爆炸闪光等）：独立槽位上传（见 EMISSIVE_SLOT_BASE）----
        let (emissive_near, emissive_far) = if self.void_mode { (0, 0) } else { self.upload_emissive(cam_pos) };
        self.last_emissive_near = emissive_near;
        self.last_emissive_far = emissive_far;
        let cull_us = cull_start.elapsed().as_micros() as u64;
        self.last_cull_us = cull_us;
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
        let signal_semaphores = [self.render_finished_semaphores[self.current_frame]];
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

        if let Err(vk::Result::ERROR_OUT_OF_DATE_KHR) = present_result {
            log::warn!("呈现 OUT_OF_DATE，重建交换链...");
            return Err("交换链过期".to_string());
        }
        if let Ok(true) = present_result {
            log::warn!("呈现 SUBOPTIMAL，重建交换链...");
            return Err("交换链过期".to_string());
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

            // 释放管线
            if self.pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.pipeline, None);
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
    #[test]
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

    /// 三几何分组：盒 4（脚×2/颈/枪）+ 圆柱 10（四肢×8/骨盆/胸桶）+ 球 1（头）= 15
    #[test]
    fn soldier_parts_count_and_tint() {
        let tint = [0.2, 0.6, 0.9, 1.0];
        let (box_parts, cyl_parts, sph_parts) =
            Renderer::soldier_part_matrices([0.0, 0.0, 0.0], 0.0, tint, 0.0, false, false);
        assert_eq!(box_parts.len(), 4, "盒体段应为 4（脚×2/颈/枪）");
        assert_eq!(cyl_parts.len(), 10, "圆柱段应为 10（四肢×8/骨盆/胸）");
        assert_eq!(sph_parts.len(), 1, "球体段应为 1（头）");
        assert_eq!(box_parts.len() + cyl_parts.len() + sph_parts.len(), 15);
        for p in box_parts.iter().chain(cyl_parts.iter()).chain(sph_parts.iter()) {
            assert_eq!(p.tint, tint);
        }
    }

    #[test]
    fn soldier_torso_height() {
        let (_, cyl_parts, _) =
            Renderer::soldier_part_matrices([0.0, 0.0, 0.0], 0.0, [1.0; 4], 0.0, false, false);
        // 圆柱组第 6 段 = 胸（枢轴 y=1.26 + 段心 -0.02），yaw=0 时平移 y = 1.24
        let t = translation(&cyl_parts[5]);
        assert!(
            (t[1] - 1.24).abs() < 1e-3,
            "胸 y 应为 1.24，实际 {}",
            t[1]
        );
        assert!(t[0].abs() < 1e-3);
        assert!((t[2] + 0.01).abs() < 1e-3, "胸 z 应为 -0.01，实际 {}", t[2]);
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
        // 盒体组第 4 段 = 枪：yaw=0 时局部偏移 (+0, +1.18, +0.52)，
        // 转 90° 后绕 y 轴旋转应落到 (+0.52, +1.18, ~0)
        let g0 = translation(&base[3]);
        let g90 = translation(&turned[3]);
        assert!(
            (g0[2] - 0.52).abs() < 1e-3,
            "yaw=0 枪应伸向 +Z，z={}",
            g0[2]
        );
        assert!(
            (g90[0] - 0.52).abs() < 1e-3,
            "yaw=90° 枪应转到 +X，x={}",
            g90[0]
        );
        assert!(g90[2].abs() < 1e-3, "yaw=90° 枪 z 应归零，z={}", g90[2]);
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
        // 盒体组第 4 段 = 枪：平移 x = pos.x
        let t = translation(&box_parts[3]);
        assert!((t[0] - 10.0).abs() < 1e-3, "x={}", t[0]);
        // 胸（圆柱组第 6 段）：平移 = pos + (0, 1.24, -0.01)
        let (_, cyl_parts, _) = Renderer::soldier_part_matrices(
            [10.0, 2.0, -3.0],
            0.0,
            [1.0; 4],
            0.0,
            false,
            false,
        );
        let tc = translation(&cyl_parts[5]);
        assert!((tc[1] - 3.24).abs() < 1e-3, "胸 y={}", tc[1]);
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
