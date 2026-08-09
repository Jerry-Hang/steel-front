//! Vulkan 渲染器模块
//!
//! 使用 ash 0.38 初始化 Vulkan，渲染一个旋转的带纹理立方体。
//! 包含完整的 Vulkan 管线生命周期管理。
//! 已接入 MVP Uniform Buffer（model/view/proj）与深度缓冲。

use std::ffi::CStr;
use std::time::Instant;
use std::fs::File;
use ash::{
    ext::debug_utils::Instance as DebugUtils,
    khr::{surface::Instance as Surface, swapchain::Device as Swapchain},
    util, vk, Device, Entry, Instance,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;
use super::lighting::{LightUniform, LIGHT_UBO_BINDING};

// ============================================================
// 数据类型
// ============================================================

/// Camera Uniform 数据（view/proj 两个 4x4 矩阵 + lod_params，144 字节）
#[repr(C)]
#[derive(Copy, Clone)]
struct CameraUniform {
    view: glam::Mat4,
    proj: glam::Mat4,
    /// (terrain_lod_high_end, fade_start, fade_end, terrain_lod_med_end)
    /// x/w：地形网格 LOD 切换距离（shader 不读这两个分量，仅 CPU 侧语义扩展）
    /// y/z：实例远档十字 quad 地面淡出区间（shader 读取，语义保持不变）
    lod_params: [f32; 4],
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

/// 距离 LOD 阈值：相机到实例中心距离 < 120 用近档立方体，否则远档十字 quad
const LOD_DISTANCE: f32 = 120.0;
/// 远档十字 quad 地面距离淡出区间（地平线处自然消失）
/// FADE_END=900 保证任何可达机位（|x|,|z|<=600）最近场点距离 <=486 < 900，
/// 场外不再“实例全灭”；远角 1210 > 900 仍自然淡出（地平线无硬边）。
const FADE_START: f32 = 400.0;
const FADE_END: f32 = 900.0;

// ============================================================
// 地形常量（世界 512×512，与实例场同域）
// ============================================================
const TERRAIN_VERTS: usize = 257;
const TERRAIN_CELLS: usize = 256;
const TERRAIN_HALF: f32 = 255.0;
const TERRAIN_UV_SCALE: f32 = 32.0; // uv 铺 0..16 重复采样

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
const MAX_MARKER_INSTANCES: u32 = 64;
/// marker 在实例 buffer 中的起始 slot：跳过 0..=INSTANCE_COUNT。
///
/// slot 65536 是地形 identity（shader 硬编码 TERRAIN_INSTANCE_INDEX=65536 读取，
/// cull_and_upload 只写 0..visible-1，永不触碰），marker 从 65537 起，互不干扰。
const MARKER_SLOT_BASE: u32 = INSTANCE_COUNT + 1;

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
// 地形高度（全图拍平 y=0）
// ============================================================

/// Hermite 平滑插值（LOD morph 过渡系数复用）
fn smooth_t(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// 地形高度：全图恒定 0（地形顶点与实例 Y 共用同一函数）
fn terrain_height(_x: f32, _z: f32) -> f32 {
    0.0
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
    framebuffers: Vec<vk::Framebuffer>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    image_available_semaphores: Vec<vk::Semaphore>,
    render_finished_semaphores: Vec<vk::Semaphore>,
    in_flight_fences: Vec<vk::Fence>,
    current_frame: usize,
    max_frames_in_flight: usize,
    first_frame_done: bool,
    /// 上一帧 render() 总耗时（微秒，性能日志用）
    last_frame_us: u64,
    vertex_buffer: vk::Buffer,
    vertex_buffer_memory: vk::DeviceMemory,
    index_buffer: vk::Buffer,
    index_buffer_memory: vk::DeviceMemory,
    /// 远档 LOD 十字 quad 几何（独立 vertex/index buffer）
    far_vertex_buffer: vk::Buffer,
    far_vertex_buffer_memory: vk::DeviceMemory,
    far_index_buffer: vk::Buffer,
    far_index_buffer_memory: vk::DeviceMemory,
    /// 地形 LOD 网格（索引 0/1/2 = 高/中/低密度；顶点缓冲 HOST_VISIBLE 供 morph 每帧更新）
    terrain_lods: Vec<TerrainLodMesh>,
    /// 每帧一份 instance buffer（双缓冲，避免 CPU 写与上一帧 GPU 读竞态）
    instance_buffers: Vec<vk::Buffer>,
    instance_buffers_memory: Vec<vk::DeviceMemory>,
    /// 每帧对应的持久映射指针
    instance_mapped: Vec<*mut std::ffi::c_void>,
    /// 全量实例（CPU 侧保留，每帧剔除后压缩上传）
    instances: Vec<InstanceData>,
    /// 剔除后可见实例索引（4B/个，上传时直接从 instances 拷贝 80B）
    culled: Vec<u32>,
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
    // ---- HUD 覆盖层（自包含：独立 pipeline / 独立顶点缓冲，不侵入主 pass）----
    hud_pipeline: vk::Pipeline,
    hud_pipeline_layout: vk::PipelineLayout,
    hud_vertex_buffer: vk::Buffer,
    hud_vertex_buffer_memory: vk::DeviceMemory,
    hud_mapped: *mut std::ffi::c_void,
    hud_vertex_count: u32,
    hud_capacity_quads: u32,
    /// 上一帧渲染统计（供 HUD / 日志）
    last_near_count: u32,
    last_far_count: u32,
    last_terrain_lod_name: &'static str,
    /// 当前光照 uniform（每帧由 set_lights 更新，render 时写入帧 slot）
    light_data: LightUniform,
    /// 当前画质预设（默认 Medium = 现有行为；纯 CPU 侧参数）
    quality: QualityPreset,
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

impl Renderer {
    pub fn new(window: &Window) -> Result<Self, String> {
        let mut renderer = Self::init_instance(window)?;
        renderer.init_swapchain()?;
        renderer.init_render_pass()?;
        renderer.init_command_pool()?;
        renderer.create_instance_buffer()?;
        renderer.init_depth_resources()?;
        renderer.init_descriptors()?;       // ← 新增
        renderer.init_pipeline()?;
        renderer.init_hud()?;
        renderer.init_framebuffers()?;
        renderer.init_texture()?;
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
        let mut required_extensions: Vec<*const i8> = window_extensions.to_vec();
        required_extensions.push(c"VK_EXT_debug_utils".as_ptr());
        let ext_names = required_extensions.as_slice();

        let layer_names = [c"VK_LAYER_KHRONOS_validation"];
        let layers: Vec<*const i8> = layer_names.iter().map(|l| l.as_ptr()).collect();

        let layer_properties = unsafe {
            entry
                .enumerate_instance_layer_properties()
                .map_err(|e| format!("无法枚举实例层属性: {}", e))?
        };
        let has_validation = layer_properties.iter().any(|prop| {
            let name = unsafe { CStr::from_ptr(prop.layer_name.as_ptr()) };
            name.to_bytes_with_nul() == b"VK_LAYER_KHRONOS_validation\0"
        });
        if has_validation {
            log::info!("验证层可用，已启用");
        } else {
            log::warn!("验证层不可用，将不使用");
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
        let device_extensions = [swapchain_ext_name.as_ptr()];
        let physical_device_features = vk::PhysicalDeviceFeatures::default();

        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extensions)
            .enabled_features(&physical_device_features);

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
            framebuffers: Vec::new(),
            command_pool: vk::CommandPool::null(),
            command_buffers: Vec::new(),
            image_available_semaphores: Vec::new(),
            render_finished_semaphores: Vec::new(),
            in_flight_fences: Vec::new(),
            current_frame: 0,
            max_frames_in_flight: 2,
            first_frame_done: false,
            last_frame_us: 0,
            vertex_buffer: vk::Buffer::null(),
            vertex_buffer_memory: vk::DeviceMemory::null(),
            index_buffer: vk::Buffer::null(),
            index_buffer_memory: vk::DeviceMemory::null(),
            far_vertex_buffer: vk::Buffer::null(),
            far_vertex_buffer_memory: vk::DeviceMemory::null(),
            far_index_buffer: vk::Buffer::null(),
            far_index_buffer_memory: vk::DeviceMemory::null(),
            terrain_lods: Vec::new(),
            instance_buffers: Vec::new(),
            instance_buffers_memory: Vec::new(),
            instance_mapped: Vec::new(),
            instances: Vec::new(),
            culled: Vec::with_capacity(INSTANCE_COUNT as usize),
            instance_radii: Vec::with_capacity(INSTANCE_COUNT as usize),
            instance_center_x: Vec::with_capacity(INSTANCE_COUNT as usize),
            instance_center_y: Vec::with_capacity(INSTANCE_COUNT as usize),
            instance_center_z: Vec::with_capacity(INSTANCE_COUNT as usize),
            markers: Vec::new(),
            last_marker_near: 0,
            last_marker_far: 0,
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
            hud_pipeline: vk::Pipeline::null(),
            hud_pipeline_layout: vk::PipelineLayout::null(),
            hud_vertex_buffer: vk::Buffer::null(),
            hud_vertex_buffer_memory: vk::DeviceMemory::null(),
            hud_mapped: std::ptr::null_mut(),
            hud_vertex_count: 0,
            hud_capacity_quads: 4096,
            last_near_count: 0,
            last_far_count: 0,
            last_terrain_lod_name: "high",
            light_data: LightUniform::default(),
            quality: QualityPreset::DEFAULT,
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
            _ => vk::PresentModeKHR::MAILBOX,
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
                        .expect("创建图像视图失败")
                }
            })
            .collect();

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
        let color_attachment = vk::AttachmentDescription::default()
            .format(self.swapchain_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

        let color_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let color_attachment_refs = [color_attachment_ref];

        // 深度附件（D32_SFLOAT）
        let depth_attachment = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let depth_attachment_ref = vk::AttachmentReference::default()
            .attachment(1)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment_refs)
            .depth_stencil_attachment(&depth_attachment_ref);
        let subpasses = [subpass];
        let attachments = [color_attachment, depth_attachment];

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
                .samples(vk::SampleCountFlags::TYPE_1)
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
        // 描述：binding=0, 类型=UNIFORM_BUFFER, 阶段=VERTEX
        let ubo_layout_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
        // 纹理采样（贴图 binding=1，采样器 binding=3，均只在 Fragment 阶段使用）
        let sampled_image_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 实例 storage buffer（binding=2，Vertex 阶段读取）
        let storage_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
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
        let bindings = [
            ubo_layout_binding,
            sampled_image_binding,
            storage_binding,
            sampler_binding,
            light_ubo_binding,
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
                .descriptor_count((max_frames * 2) as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(max_frames as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(max_frames as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(max_frames as u32),
        ];

        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets(max_frames as u32);

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
                // 范围必须覆盖 marker 区（INSTANCE_COUNT+1+MAX_MARKER_INSTANCES），
                // 否则 shader 读 marker slot 会触发 VUID 越界校验
                .range(
                    std::mem::size_of::<InstanceData>() as u64
                        * (INSTANCE_COUNT as u64 + 1 + MAX_MARKER_INSTANCES as u64),
                );
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

    fn init_pipeline(&mut self) -> Result<(), String> {
        let vs_spirv = load_spirv("assets/triangle.vert.spv")?;
        let fs_spirv = load_spirv("assets/triangle.frag.spv")?;
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
                .offset(0),
            // location 1: color vec3
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::size_of::<[f32; 3]>() as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset((std::mem::size_of::<[f32; 3]>() * 2) as u32),
        ];

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
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

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

        let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
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

        unsafe {
            self.device.destroy_shader_module(vs_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }

        self.create_vertex_buffer()?;
        self.create_index_buffer()?;
        self.create_far_geometry()?;
        self.create_terrain_lods()?;
        log::info!("图形管线创建完成");
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

        // 独立 pipeline layout：无描述符
        let hud_layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&[])
            .push_constant_ranges(&[]);
        self.hud_pipeline_layout = unsafe {
            self.device
                .create_pipeline_layout(&hud_layout_info, None)
                .map_err(|e| format!("创建 HUD 管线布局失败: {}", e))?
        };

        let hud_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&hud_vertex_input)
            .input_assembly_state(&hud_input_assembly)
            .viewport_state(&hud_viewport_state)
            .rasterization_state(&hud_rasterizer)
            .multisample_state(&hud_multisampling)
            .depth_stencil_state(&hud_depth_stencil)
            .color_blend_state(&hud_blend_state)
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

    /// 上一帧统计：near/far 可见实例数与地形 LOD 名（供 HUD / 日志）
    pub fn last_stats(&self) -> (u32, u32, &'static str) {
        (
            self.last_near_count,
            self.last_far_count,
            self.last_terrain_lod_name,
        )
    }

    /// 更新光照 uniform（每帧渲染前调用；默认全零 = 光照关闭）
    pub fn set_lights(&mut self, lights: &LightUniform) {
        self.light_data = *lights;
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
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_requirements.size)
            .memory_type_index(memory_type);
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
        let staging_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(staging_reqs.size)
            .memory_type_index(staging_type);
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
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(memory_type);
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
                    let y = heights[iz * w + ix];
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
                            terrain_coarse_height(x, z, coarse, coarse_cells)
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

    /// 每帧按 morph 进度 t 更新当前 LOD 网格顶点高度：
    /// h = 细高度 + t × (下一级曲面插值高度 − 细高度)。t=1 时几何与下一级完全重合，
    /// 因此切换级别无 popping。仅在过渡带内（0<t<1）执行，整块重传顶点缓冲。
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
        for i in 0..n {
            mesh.verts[i].pos[1] = base[i] + (coarse[i] - base[i]) * blend;
        }
        let bytes = n * std::mem::size_of::<Vertex>();
        unsafe {
            std::ptr::copy_nonoverlapping(
                mesh.verts.as_ptr() as *const u8,
                mesh.vertex_mapped as *mut u8,
                bytes,
            );
        }
    }

    /// 生成 256×256 网格实例；按 frame-in-flight 数量双缓冲
    /// （每帧一份 HOST_VISIBLE|HOST_COHERENT buffer，剔除后压缩上传到当前帧 slot）
    fn create_instance_buffer(&mut self) -> Result<(), String> {
        debug_assert!(
            std::mem::size_of::<InstanceData>() == 80,
            "InstanceData 必须对齐 std430 步长 80 字节"
        );

        // 256×256 网格：间距 2.0、以原点为中心、y=0 平面（场地 512×512）。
        // 实例 Y 采样地形高度（同一 terrain_height()），底部下沉 0.05 防 z-fighting。
        self.instances = Vec::with_capacity(INSTANCE_COUNT as usize);
        for iz in 0..GRID_SIZE {
            for ix in 0..GRID_SIZE {
                let x = (ix as f32 - (GRID_SIZE as f32 - 1.0) * 0.5) * 2.0;
                let z = (iz as f32 - (GRID_SIZE as f32 - 1.0) * 0.5) * 2.0;
                let y = terrain_height(x, z) - 0.05;
                let model = glam::Mat4::from_translation(glam::Vec3::new(x, y, z));
                // 半径 = 0.5 * max(三轴列长度)；预算一次供剔除查表（纯平移时恒 0.5）
                let r = 0.5 * model
                    .x_axis
                    .length()
                    .max(model.y_axis.length())
                    .max(model.z_axis.length());
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

        // 末尾保留 1 个 slot 存 identity 实例（地形 draw 用，仅创建时写入一次），
        // 再追加 MAX_MARKER_INSTANCES 个 slot 给世界障碍 marker（见 MARKER_SLOT_BASE）
        let buffer_elems = (INSTANCE_COUNT + 1 + MAX_MARKER_INSTANCES) as u64;
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
            // 写入 identity 实例（地形 draw 读取，永不覆盖）
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &identity as *const InstanceData as *const u8,
                    mapped as *mut u8,
                    std::mem::size_of::<InstanceData>(),
                );
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
        // 近/远档分界距离随画质预设变化（Medium 与原 LOD_DISTANCE 一致）
        let near_sq = quality_params(self.quality).instance_lod_distance;
        let near_sq = near_sq * near_sq;
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

    /// 每帧视锥剔除 + 距离 LOD 分档：
    /// 可见实例按 [近档(d<LOD_DISTANCE)][远档] 连续压缩上传到当前帧 slot，
    /// 返回 (near, far) 两个档的实例数（近档在前，远档紧跟其后）。
    fn cull_and_upload(
        &mut self,
        view: glam::Mat4,
        proj: glam::Mat4,
        cam_pos: glam::Vec3,
    ) -> (u32, u32) {
        let planes = Self::extract_frustum_planes(view, proj);
        self.culled.clear();

        // 单遍剔除：半径查表（创建时预算），可见实例只存索引（4B）避免 80B 中转拷贝。
        // ★ AVX-512 加速已启用（本机 Zen4 实测走 16 实例/批路径，见下方 cull_spheres_avx512）：
        //   选路统一走 cpu::avx512_enabled()——硬件不支持、RV3D_DISABLE_AVX512=1、
        //   Intel 11 代（能效差）与 12 代起（大小核）都会自动禁用并回退到下一档。
        // x86_64 分级选路：AVX-512（16 实例/批）> AVX2（8）> AVX（8）> SSE4.2（4）> 标量，
        // 各级路径与标量逐位一致（非 FMA 累加顺序相同），老处理器自动回退。
        #[cfg(target_arch = "x86_64")]
        {
            if crate::engine::cpu::avx512_enabled() {
                // safety: 上面已运行时检测 AVX-512，CPU 支持才进入该分支
                unsafe {
                    Self::cull_spheres_avx512(
                        &self.instance_center_x,
                        &self.instance_center_y,
                        &self.instance_center_z,
                        &self.instance_radii,
                        &planes,
                        &mut self.culled,
                    );
                }
            } else if std::is_x86_feature_detected!("avx2") {
                // safety: 上面已运行时检测 AVX2，CPU 支持才进入该分支
                unsafe {
                    Self::cull_spheres_avx2(
                        &self.instance_center_x,
                        &self.instance_center_y,
                        &self.instance_center_z,
                        &self.instance_radii,
                        &planes,
                        &mut self.culled,
                    );
                }
            } else if std::is_x86_feature_detected!("avx") {
                // safety: 上面已运行时检测 AVX
                unsafe {
                    Self::cull_spheres_avx(
                        &self.instance_center_x,
                        &self.instance_center_y,
                        &self.instance_center_z,
                        &self.instance_radii,
                        &planes,
                        &mut self.culled,
                    );
                }
            } else if std::is_x86_feature_detected!("sse4.2") {
                // safety: 上面已运行时检测 SSE4.2
                unsafe {
                    Self::cull_spheres_sse(
                        &self.instance_center_x,
                        &self.instance_center_y,
                        &self.instance_center_z,
                        &self.instance_radii,
                        &planes,
                        &mut self.culled,
                    );
                }
            } else {
                Self::cull_spheres_scalar(
                    &self.instance_center_x,
                    &self.instance_center_y,
                    &self.instance_center_z,
                    &self.instance_radii,
                    &planes,
                    &mut self.culled,
                );
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self::cull_spheres_scalar(
                &self.instance_center_x,
                &self.instance_center_y,
                &self.instance_center_z,
                &self.instance_radii,
                &planes,
                &mut self.culled,
            );
        }

        let stride = std::mem::size_of::<InstanceData>();
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return (0, 0),
        };

        // 单遍分档上传：近档写前段、远档紧跟其后（偏移 = near + far），距离²只算一次
        let near_sq = quality_params(self.quality).instance_lod_distance;
        let near_sq = near_sq * near_sq;
        let mut near_count = 0u32;
        let mut far_count = 0u32;
        for &idx in &self.culled {
            let inst = &self.instances[idx as usize];
            let dx = inst.model[12] - cam_pos.x;
            let dy = inst.model[13] - cam_pos.y;
            let dz = inst.model[14] - cam_pos.z;
            if dx * dx + dy * dy + dz * dz < near_sq {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add((near_count as usize) * stride),
                        stride,
                    );
                }
                near_count += 1;
            } else {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        inst as *const InstanceData as *const u8,
                        slot.add(((near_count as usize) + (far_count as usize)) * stride),
                        stride,
                    );
                }
                far_count += 1;
            }
        }
        (near_count, far_count)
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

    /// 加载 assets/textures/test.png 并创建纹理资源
    fn init_texture(&mut self) -> Result<(), String> {
        // 从文件加载真实贴图（image crate）。
        // image crate 行序为自上而下，与 Vulkan 默认 UV 原点一致，无需翻转。
        let texture_path = "assets/textures/test.png";
        let img = image::open(texture_path)
            .map_err(|e| format!("加载纹理图片失败 '{}': {}", texture_path, e))?
            .to_rgba8();
        let width = img.width();
        let height = img.height();
        let pixels = img.as_raw().clone();
        let image_size = (width * height * 4) as u64;

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
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
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
                .level_count(1)
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

            // TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL
            let barrier_to_read = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(subresource_range)
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ);
            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier_to_read],
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
                    .level_count(1)
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
            .anisotropy_enable(false)
            .max_anisotropy(1.0)
            .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
            .unnormalized_coordinates(false)
            .min_lod(0.0)
            .max_lod(0.0);
        self.texture_sampler = unsafe {
            self.device
                .create_sampler(&sampler_info, None)
                .map_err(|e| format!("创建纹理 Sampler 失败: {}", e))?
        };

        log::info!(
            "纹理初始化完成: {}x{}（来自 {}）",
            width,
            height,
            texture_path
        );
        Ok(())
    }

    /// 把纹理 Image View 和 Sampler 写入每个 DescriptorSet（binding 1 / 2）
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

            let writes = [sampled_image_write, sampler_write];
            unsafe {
                self.device.update_descriptor_sets(&writes, &[]);
            }
        }
        Ok(())
    }

    fn init_framebuffers(&mut self) -> Result<(), String> {
        self.framebuffers = self
            .swapchain_image_views
            .iter()
            .enumerate()
            .map(|(i, &image_view)| {
                let attachments = [image_view, self.depth_image_views[i]];
                let framebuffer_create_info = vk::FramebufferCreateInfo::default()
                    .render_pass(self.render_pass)
                    .attachments(&attachments)
                    .width(self.swapchain_extent.width)
                    .height(self.swapchain_extent.height)
                    .layers(1);
                unsafe {
                    self.device
                        .create_framebuffer(&framebuffer_create_info, None)
                        .expect("创建帧缓冲失败")
                }
            })
            .collect();
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

        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.1, 0.1, 0.15, 1.0],
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

        // 近档 draw call：立方体几何，实例区从 0 开始
        if near_count > 0 {
            let vertex_buffers = [self.vertex_buffer];
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
                    self.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    INDICES.len() as u32,
                    near_count,
                    0,
                    0,
                    0,
                );
            }
        }

        // 远档 draw call：十字双 quad 几何，实例区偏移 = near_count（[近档][远档] 连续排布）
        if far_count > 0 {
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
            self.device
                .end_command_buffer(command_buffer)
                .map_err(|e| format!("结束命令缓冲失败: {}", e))?;
        }
        Ok(())
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
        let (near_count, far_count) = self.cull_and_upload(view, proj, cam_pos);
        // ---- 世界障碍 marker：独立槽位上传（见 MARKER_SLOT_BASE），计数供 draw call 使用 ----
        let (marker_near, marker_far) = self.upload_markers(cam_pos);
        self.last_marker_near = marker_near;
        self.last_marker_far = marker_far;
        let cull_us = cull_start.elapsed().as_micros() as u64;
        self.last_near_count = near_count;
        self.last_far_count = far_count;

        // ---- 地形网格 LOD：按相机到地形中心地面距离选级，过渡带内 morph 高度 ----
        let terrain_dist = (cam_pos.x * cam_pos.x + cam_pos.z * cam_pos.z).sqrt();
        let quality = quality_params(self.quality);
        let (terrain_lod, terrain_blend) = terrain_lod_blend_with_params(terrain_dist, quality);
        self.last_terrain_lod_name = terrain_lod.name();
        let t0 = Instant::now();
        self.update_terrain_lod_morph(terrain_lod, terrain_blend);
        let terrain_lod_index = terrain_lod as usize;
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
                "visible={}/{} near={} far={} fps={:.1} frame_us={} cull_us={} terrain_us={} wait_fence_us={} acquire_us={} record_us={} submit_us={} present_us={} terrain_lod={} blend={:.3} quality={}",
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
                self.quality().label()
            );
            self.frame_count = 0;
            self.perf_window_start = Instant::now();
            self.last_perf_log = Instant::now();
        }

        // ---- 每帧把 view/proj 写进 Uniform Buffer（按 frame-in-flight 多份）----
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
        let light_ubo = self.light_data;
        if let Some(&ptr) = self.light_uniform_mapped.get(self.current_frame) {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &light_ubo as *const _ as *const u8,
                    ptr as *mut u8,
                    std::mem::size_of::<LightUniform>(),
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

        if !self.first_frame_done {
            self.first_frame_done = true;
            println!("=== RENDERER OK ===");
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
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device.destroy_pipeline_layout(self.pipeline_layout, None);
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
        // 非 x86_64 或无双 AVX2：标量结果本身就是正确语义，无需对照
        assert!(!scalar_out.is_empty() || scalar_out.is_empty());
    }

    #[test]
    fn simd_cull_tail_batches_handled() {
        // 长度非 8 的倍数：覆盖尾部标量路径
        let mut rng = Rng(0xC0FF_EE);
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
    }
}
