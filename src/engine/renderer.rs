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
pub(crate) struct Vertex {
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
    /// 🧊 `(u, v, glass)`：u/v = 该顶点在**屏幕上的归一化位置**（磨砂玻璃按它采样
    /// 那张降采样模糊图）；`glass` = ui.rs 的 `Quad::glass` 标志（0/1）。
    /// 非玻璃 quad 照样写 uv（不额外分支），片元按 `glass` 决定用不用它。
    uv_glass: [f32; 3],
}
// HUD 覆盖层自己的步长契约（与主管线的 `Vertex` 无关）：`hud.vert.spv` 按
// `pos vec2 + color vec4 + uv_glass vec3` 取属性，步长由这里推导 ⇒ 改动同样必须在这里被挡住。
const _: () = assert!(
    std::mem::size_of::<HudVertex>() == 36,
    "HudVertex 必须是 36B（pos vec2 + color vec4 + uv_glass vec3）：HUD 着色器按此布局取属性"
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

/// `RV3D_NO_GROUND_TEX=1` 时关掉地面微细节层（`light_data.flags.w` 保持 0）。
/// A/B 诊断门，与 `RV3D_NO_SHADOW` 同一套惯例。**读一次缓存住**：本函数在每帧构建
/// 光照 UBO 的路径上，不该每帧 `getenv`。
fn no_ground_detail_tex() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RV3D_NO_GROUND_TEX").as_deref() == Ok("1"))
}

/// 动态阴影图的绑定号（片元第二张阴影图，见 `shadow_dyn_image` 字段注释）。
/// 10 是本 set layout 里 ground_detail(9) 之后的第一个空位；**必须与 build.rs WGSL 的
/// `@group(0) @binding(10) var shadow_dyn_map` 同步**。
const SHADOW_DYN_BINDING: u32 = 10;

/// 🧊 磨砂玻璃背景模糊图的尺寸（固定，不随交换链变化 —— 理由见 `init_menu_blur`）。
/// 320×200 ≈ 交换链的 1/8，一次线性 blit 就得到 8×8 盒式平均，正是"毛玻璃"需要的低频。
const MENU_BLUR_W: u32 = 320;
const MENU_BLUR_H: u32 = 200;

// ============================================================
// 地形常量（世界 512×512，与实例场同域）
// ============================================================
const TERRAIN_VERTS: usize = 129;
const TERRAIN_CELLS: usize = 128;
const TERRAIN_HALF: f32 = 255.0;
const TERRAIN_UV_SCALE: f32 = 32.0; // uv 铺 0..16 重复采样
/// 地形网格渲染下沉量：地面平铺 quad 抬到 +0.05、地形网格整体下沉 0.35，
/// 两层地面在深度上拉开 0.4m，杜绝远距离深度精度不足导致的 z-fighting 闪烁
/// （旧版实例场与地形几乎共面，远档顶面被深度测试剔除、只剩侧壁可见）。
const TERRAIN_RENDER_SINK: f32 = 0.35;
/// 程序化地形平坦半径（米）：覆盖中央 60×60 安全区、障碍环带 58–130m 与两军接火区
/// 🔴 2026-10-02 起改 `pub`：被 `map.rs` 的地图守卫引用（据点若落在平坦区外，
/// 固定 y 的占领底盘会被丘陵埋掉或悬空）。
pub const TERRAIN_FLAT_RADIUS: f32 = 230.0; // 城市占地 ±215 需平地（2026-08-21 城市地图）
/// 平坦区外丘陵最大抬升（米，平滑抬升 × 噪声幅值，恒 ≤ 本常量）
const TERRAIN_HILL_AMPLITUDE: f32 = 15.0;
/// 丘陵抬升过渡带宽（米）：半径 140 → 320 内 smoothstep 从 0 升到满幅（起点斜率 0）
const TERRAIN_HILL_RAMP: f32 = 130.0;
/// 值噪声格距（米）：格距越大丘陵越平缓（低频滚动丘陵，LOD morph 无突兀）
const TERRAIN_HILL_CELL: f32 = 128.0;

// ---- 地形网格 LOD（3 级密度：高 129² / 中 65² / 低 33² 顶点）----
/// 各级每边格数（128 / 64 / 32），顶点数 = 格数 + 1，格间距 = 512 / 格数。
/// 粗网格顶点恰为细网格顶点子集（间距 4.0 / 8.0 / 16.0，起点同为 -255）。
///
/// 🔴 **2026-09-26 由 256/128/64 降到 128/64/32**：帧预算地图显示地形网格占 **8.4%** 帧时间
/// （`RV3D_NO_TERRAIN=1` 实测 144.7 → 156.8），而 LOD 是**按"相机到地图中心的距离"**选的 ——
/// 玩家出生在地图正中 ⇒ 恒选最细那一级 ⇒ **为一片完全平坦的城市（`terrain_height` 半径 140m
/// 内恒为 0）铺 131k 个 2m 三角形**。降到 4m 网格后：BASE +4.6%、全关底噪 184.6 → 200.4。
/// 代价用数衡量（不是"看着差不多"）：最细一级的插值误差 `256 格 0.007m / 128 格 0.029m /
/// 64 格 0.110m`（丘陵 ≤15m），阈值 0.10m 钉在
/// `terrain_finest_grid_interpolation_error_stays_within_budget` 里。
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
pub(crate) enum TerrainLod {
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

/// 截图读回时主机侧等待**围栏**的超时（纳秒，2 秒足够完成一帧渲染）。
/// 两处都用它：等本帧渲染完成（`in_flight_fences[slot]`）与等拷贝命令完成（截图自己的围栏）。
const SCREENSHOT_WAIT_TIMEOUT_NS: u64 = 2_000_000_000;

/// 交换链图像获取的超时（纳秒）。**绝不可以用 `u64::MAX`**：呈现引擎不给图像时
/// （隐藏/被遮挡窗口 + IMMEDIATE 是已知诱因）主循环会静默卡死，外面只看到"日志停住"。
const ACQUIRE_TIMEOUT_NS: u64 = 1_000_000_000;

/// 检视围栏等待超时（纳秒）。5 秒足够任何一帧；超时说明 GPU 侧出了问题，要留下日志。
const FENCE_WAIT_TIMEOUT_NS: u64 = 5_000_000_000;

/// 单次**呈现**耗时超过多少算「卡住」（微秒）。
///
/// 取值理由：正常呈现实测 Linux 43–101µs、Windows 101–373µs
/// ⇒ 1 秒留了**三个数量级**余量，不会把"某帧慢了一下"误判成卡死。
const PRESENT_STALL_US: u64 = 1_000_000;

/// 连续呈现卡顿到第几次就降级到 mailbox（与 `ACQUIRE_STALL_FALLBACK` 同语义）。
const PRESENT_STALL_FALLBACK: u32 = 3;

/// 呈现卡顿的处置（纯函数，可单测）。语义与 `FrameAction` / `PresentOutcome` 一致：
/// **分类归纯函数，动作归调用方**，这样三条分支都能在没 GPU 的机器上钉住。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresentStall {
    /// 正常 —— 调用方把连续计数**清零**（漏了它，一次偶发卡顿会累积成"连续三次"）
    Ok,
    /// 偶发一次：值得留一条日志（否则就是静默），但**不动行为**
    Warn,
    /// 连续多次：该按 acquire 那套方式降级了
    Degrade,
}

/// `(本次呈现耗时, 含本次在内的连续卡顿次数)` → 处置。判据
/// `present_stall_classifies_and_clears`。
fn present_stall(present_us: u64, consecutive: u32) -> PresentStall {
    if present_us < PRESENT_STALL_US {
        PresentStall::Ok
    } else if consecutive < PRESENT_STALL_FALLBACK {
        PresentStall::Warn
    } else {
        PresentStall::Degrade
    }
}

/// 连续卡顿计数的推进（纯函数）。**正常帧必须清零** ——
/// 漏了清零，几次**偶发**长卡顿会累积成"连续三次"从而误降级。
///
/// 🔴 单独抽出来的理由是**让测试真的钉住调用方**：状态机若写在测试里的局部闭包里，
/// 真实调用方忘了清零时测试照样绿（那只是把意图抄了一遍，不是判据）。
/// 抽成纯函数后，线上与测试跑的是同一份逻辑。
fn next_stall_count(current: u32, present_us: u64) -> u32 {
    match present_stall(present_us, current.saturating_add(1)) {
        PresentStall::Ok => 0,
        _ => current.saturating_add(1),
    }
}

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

/// 一帧**走完呈现之后**的处置（纯函数，可单测）。判据见 `frame_action` 的文档。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameAction {
    /// 本帧正常结束（已呈现、无需重建）
    Presented,
    /// 本帧**已呈现**，随后重建交换链
    RecreateAfterPresent,
    /// 设备级失败（SURFACE_LOST / DEVICE_LOST …）：交回上层
    Fail,
}

/// 纯函数：`(acquire 是否 SUBOPTIMAL, present 的结果)` → 本帧处置。
///
/// 🔴 2026-09-25 深夜复查补：**成功 acquire 之后、present 之前，不许再 return**。
///
/// `acquire_next_image` 成功 ⇒ `image_available_semaphores[current_frame]` 已被 signal，
/// 而这张图像只有走完 present 才会被交还；那个信号量是**渲染器生命周期对象**
/// （`init_sync_objects` 只建一次，`recreate_swapchain` 不会重建它），而 `current_frame`
/// 也只在整帧走完时才前进 ⇒ 成功 acquire 之后提前 return，会让**下一帧拿一个仍 signaled
/// 的二值信号量去 acquire** —— 未定义行为（`VUID-vkAcquireNextImageKHR-semaphore-01286`：
/// semaphore 必须 unsignaled；相关条 `-01779`：不得有未完成的 signal/wait）。本机默认不开
/// 验证层（`RV3D_VALIDATION=1` 才开）⇒ 这类问题**完全静默**，本仓历史上最贵的一类 bug 同形。
///
/// 旧写法把"acquire 说 suboptimal"与"present 说 suboptimal"混成了同一件事：前者直接
/// `return Err("交换链过期")` **丢掉了已经拿到手的图像**。现在两件事分开 ——
/// acquire 的 suboptimal 只是一个**登记**（本帧照常 record/submit/present），
/// 重建统一发生在 present 之后（与 present 自己返回 SUBOPTIMAL / OUT_OF_DATE 合流）。
///
/// 签名本身承载这条不变式：**present 结果必须作为参数传进来**，
/// 也就是"想返回 `RecreateAfterPresent` 就必须先 present 过"。
fn frame_action(acquire_suboptimal: bool, present: PresentOutcome) -> FrameAction {
    match present {
        PresentOutcome::Failed => FrameAction::Fail,
        PresentOutcome::RecreateSwapchain => FrameAction::RecreateAfterPresent,
        PresentOutcome::Presented if acquire_suboptimal => FrameAction::RecreateAfterPresent,
        PresentOutcome::Presented => FrameAction::Presented,
    }
}

/// 这一帧要不要**跳过渲染**（纯函数，可单测）。
///
/// 两种降级都必须跳过：① `gpu_stalled`（围栏连续超时 ⇒ GPU 侧已卡死，再提交也只是白等
/// 5 秒）；② `swapchain_broken`（**重建交换链中途失败** ⇒ `swapchain` 等句柄可能已被
/// `destroy_swapchain` 销毁，再 acquire/提交就是拿空句柄调 Vulkan）。
/// 两者都保持进程与输入响应；后者在下一次重建**成功**时自动清除（尺寸变化 / 5 秒
/// 尺寸自检都会重试）。
fn frame_suppressed(gpu_stalled: bool, swapchain_broken: bool) -> bool {
    gpu_stalled || swapchain_broken
}

/// 这一帧要不要重画阴影图（纯函数，可单测）。见 `Renderer::shadow_every`：隔帧重画，
/// `every = 1` 就是每帧（A/B 对照）；`void_mode`（检视模式）下从不画。
fn shadow_due(frame_seq: u64, every: u32, void_mode: bool) -> bool {
    !void_mode && frame_seq % every.max(1) as u64 == 0
}

/// **静态**阴影图这一帧要不要重画（纯函数，可单测）。
///
/// 静态图只装世界不动的那批投射者（地形/地面场/marker/道具），所以它只需要偶尔重画：
/// `every` 帧一次（默认 30 ≈ 半秒，安全网），`split = false`（A/B 关掉拆分）时不单独画
/// —— 那条路走"单张图、每帧两类投射者一起画"的旧逻辑。`void_mode`（检视模式）下从不画。
fn shadow_static_due(frame_seq: u64, every: u64, void_mode: bool, split: bool) -> bool {
    split && !void_mode && frame_seq % every.max(1) == 0
}

/// surface 未给出 `current_extent`（= `u32::MAX`）时的**兜底**尺寸：必须夹进
/// `[min_image_extent, max_image_extent]`（纯函数，可单测）。
///
/// Vulkan 规定 `current_extent` 未定义时 `imageExtent` 必须落在 surface 给的范围里
/// （`VUID-VkSwapchainCreateInfoKHR-imageExtent-01274`）。旧代码在这一支直接写死 `1280x720` ——
/// 只要某台设备的 `maxImageExtent` 比它小，创建交换链当场违规；而**本机 Win32 surface 恒给出
/// 具体尺寸，所以这条分支在默认配置上永远走不到**（与 §21.23 的 PT blit 写死 2560x1600 是同一类：
/// 默认配置恰好等于那个写死的值，于是缺陷永远被躲过去）。
///
/// 退化输入也要挡住：驱动若报 0 或 `max < min`，宁可退到 `min`（且不小于 1），
/// 也不要提交一个 `imageExtent = 0` 的交换链 —— 那没有兜底路径。
fn clamp_swapchain_extent(
    fallback: vk::Extent2D,
    min: vk::Extent2D,
    max: vk::Extent2D,
) -> vk::Extent2D {
    let lo_w = min.width.max(1);
    let lo_h = min.height.max(1);
    let hi_w = max.width.max(lo_w);
    let hi_h = max.height.max(lo_h);
    vk::Extent2D {
        width: fallback.width.clamp(lo_w, hi_w),
        height: fallback.height.clamp(lo_h, hi_h),
    }
}

/// 决定交换链到底用哪个尺寸 —— **窗口尺寸优先于那个写死的 1280x720**。
///
/// 🔴 **2026-09-28 修（Linux 适配，Wayland 阻断级）**：`currentExtent` 在
/// **Wayland 下是设计上就未定义的**（Mesa 明确填 `{UINT32_MAX, UINT32_MAX}`，见
/// `wsi_common_wayland.c::wsi_wl_surface_get_capabilities`），而在 Win32 / X11
/// 下它**恒等于窗口尺寸** ⇒ 这条兜底分支在 Windows 上**永远走不到**，于是
/// 「兜底 = 写死 1280x720」这个值一路漂到 Linux 才暴露，后果是三重错位：
/// 交换链恒为 1280x720、投影用窗口尺寸、HUD 排版也用窗口尺寸 ⇒ 画面比例错乱、
/// HUD 按窗口排版却画进 720p 视口；而 `Resized` 的重建**走的还是同一段代码**
/// ⇒ **永远不跟随窗口**。
///
/// 这与 §21.23 的「PT blit 写死 2560x1600」是同一类缺陷：默认配置恰好等于那个
/// 写死的值，于是缺陷永远被躲过去。
///
/// 语义（判据 `swapchain_extent_follows_the_window_when_current_extent_is_undefined`）：
/// - `currentExtent.width != u32::MAX`（Win32 / X11）⇒ **原样返回**，
///   与修复前逐字节一致，Windows 侧行为不变；
/// - 未定义（Wayland）⇒ 用**调用方给的窗口物理尺寸**；
/// - 窗口尺寸不可用（0，Wayland 首个 configure 之前）⇒ 才退到 1280x720 兜底。
///
/// 后两条都要夹进 `min/maxImageExtent`（`VUID-VkSwapchainCreateInfoKHR-imageExtent-01274`）。
fn swapchain_extent_choice(
    current: vk::Extent2D,
    window: vk::Extent2D,
    min: vk::Extent2D,
    max: vk::Extent2D,
) -> vk::Extent2D {
    if current.width != u32::MAX {
        return current;
    }
    let fallback = if window.width > 0 && window.height > 0 {
        window
    } else {
        vk::Extent2D {
            width: 1280,
            height: 720,
        }
    };
    clamp_swapchain_extent(fallback, min, max)
}

/// 设备扩展选择：**缺扩展要降级，不要让 `create_device` 直接失败**。
///
/// 🔴 **2026-09-29 修**。原代码在 `VK_EXT_mesh_shader` 可用时**无条件**追加 5 个光追扩展，
/// 而 `enumerate_device_extension_properties` 的结果只在**事后**用来打一行 warn ——
/// 于是"设备缺任一光追扩展"的后果不是降级，而是 **`create_device` 失败 = 游戏起不来**，
/// 而 PT 本来就是**默认关**的，根本不值得为它挡住启动。
/// 同一段还有第二处：`PhysicalDeviceRayQueryFeaturesKHR` 等**特性结构无条件挂进 pNext**，
/// 即使对应扩展没启用 —— 那本身就是无效用法。
///
/// 语义（判据 `device_extensions_degrade_instead_of_failing`）：
/// - `required`：逐个按实际支持情况启用；缺的**如实报出来**（是否致命由调用方决定，
///   本函数不替它决定 —— 这里只负责"别把可选的东西当必需"）；
/// - `all_or_nothing`：**全有或全无**的一组。
///   🔴 光追那 5 个必须同进同退：只启用一半时，特性链与后续代码路径都假设它们齐全，
///   半套就是未定义行为 —— 比"整组不用"危险得多。
///   ⇒ 缺任何一个就**整组不启用**，并把缺的那些报出来。
fn pick_device_extensions<'a>(
    available: &[String],
    required: &[&'a str],
    all_or_nothing: &[&'a str],
) -> (Vec<&'a str>, Vec<&'a str>) {
    let has = |n: &str| available.iter().any(|a| a == n);
    let mut enabled: Vec<&'a str> = required.iter().copied().filter(|n| has(n)).collect();
    let mut missing: Vec<&'a str> = required.iter().copied().filter(|n| !has(n)).collect();
    let absent: Vec<&'a str> = all_or_nothing.iter().copied().filter(|n| !has(n)).collect();
    if absent.is_empty() {
        enabled.extend(all_or_nothing.iter().copied());
    } else {
        // 整组不启用：把缺的那些报出去（**已存在的那几个也算"没启用"**，
        // 因为调用方要的是"这一组能不能用"，不是"哪几个名字恰好存在"）
        missing.extend(absent);
    }
    (enabled, missing)
}

/// 交换链重建失败后**多久才允许再试一次**（秒）。见 `should_retry_swapchain`。
const RECREATE_RETRY_MIN_SECS: f32 = 1.0;

/// 现在值不值得再试一次交换链重建（纯逻辑，可单测；"距上次尝试多久"由调用方折成秒）。
///
/// - 设备丢失 ⇒ **永远不值得**：本引擎没有重建设备的路径，重试只会失败。实测（2026-09-26）
///   设备被打掉之后，`main.rs` 的尺寸自检**每帧**重试重建，12 秒跑 1961 轮、刷 5900 行
///   错误日志，而进程看着还活着 —— "看起来在跑、其实一帧都画不出来"正是本仓最反对的静默。
/// - 上一次刚失败（< `RECREATE_RETRY_MIN_SECS`）⇒ 先不试（限流，别刷屏）；
/// - 其余 ⇒ 值得（重建成功即自动恢复 `swapchain_broken`）。
fn should_retry_swapchain(device_lost: bool, swapchain_broken: bool, secs_since_attempt: f32) -> bool {
    if device_lost {
        return false;
    }
    !(swapchain_broken && secs_since_attempt < RECREATE_RETRY_MIN_SECS)
}

/// 这条错误串是不是"设备丢了"（纯函数，可单测）。
///
/// 引擎各层的错误类型都是 `String`（只做前缀拼接），`VK_ERROR_DEVICE_LOST` 到这一层
/// 只剩 ash 的 Display 文本 `The logical device has been lost.` ⇒ 只能按子串判。
/// 判据独立成函数是为了**只有一处**定义"什么算不可恢复"。
pub fn is_device_lost_error(msg: &str) -> bool {
    msg.contains("device has been lost") || msg.contains("DEVICE_LOST")
}

/// `device_wait_idle` 失败时该打的日志（纯函数，可单测）：`None` = 这次不打。
///
/// 🔴 2026-09-26 复查发现全仓 6 处 `let _ = self.device.device_wait_idle();`
/// （`set_first_person_gun_mesh` / `set_props` ×2 / `set_shadow_props` /
/// `pt_set_scene_markers` / `Drop`）—— 全都是"**等空闲 → 销毁/重建在飞资源**"的关键路径，
/// 而**等待失败与等待成功在日志里长得一模一样**：后面那句 `destroy_buffer` 到底安不安全，
/// 排查的人手里没有任何证据。而这条命令最可能返回的错误正好是 `VK_ERROR_DEVICE_LOST`
/// ——本仓最贵的一类故障（铁律 B「设备丢失 = 不可恢复」）。
///
/// 消息**必须点名错误**：否则 `device lost` 与 `out of host memory` 在日志里分不清，
/// 而这两者的处置完全相反（前者不可恢复、后者可以少要点资源重试）。
/// 用闩而不是每次都记：这几条路径在换枪 / 重载 / 进出 PT 时反复跑。
///
/// 判据 = `wait_idle_failure_is_named_and_reported_once` +
/// `device_wait_idle_errors_are_never_silently_dropped`。
pub fn wait_idle_failure_message(err: vk::Result, already_warned: bool) -> Option<String> {
    if already_warned {
        return None;
    }
    Some(format!(
        "gpu: device_wait_idle 失败（{err:?}）—— 紧接着的 destroy/create 都发生在**尚未确认空闲**的\
         设备上；若为 device lost 则不可恢复，见铁律 B"
    ))
}

/// 道具缓冲要不要（重新）创建 —— 纯逻辑，可单测。
///
/// 判据是 `need > capacity`，**不是** `need != capacity`：写成 `!=` 时"道具变少"也会重建，
/// 而重建等于 destroy 正在被在飞帧与 PT BLAS 引用的缓冲。枪模那条路为此付过代价
/// （2026-09-15 实测按一下 2 = 整台设备消失，见 `set_first_person_gun_mesh` 的注释）。
fn prop_buffer_growth_needed(
    need_v: u32,
    cap_v: u32,
    need_i: u32,
    cap_i: u32,
    mapped_ok: bool,
) -> bool {
    !mapped_ok || need_v > cap_v || need_i > cap_i
}

/// 启动时要不要构建 PT 常驻资源（纯函数，可单测）。
///
/// 🔴 2026-09-25 修：`RV3D_PT_LIVE=1` 自称"强制开"，但它**只**改 `pt_live_enabled`，
/// 而 PT 真正出画还要求 `pt_resident.is_some()`（判据见 `render()` 里那句
/// `if self.pt_live_enabled && self.pt_resident.is_some()`），常驻资源却只在
/// `config.pt_enable == true` 时构建 ⇒ **只设环境变量时 PT 一帧都跑不出来**，
/// 外面只看到"开关写着开了、画面没变"。那次"PT 验证"因此什么也没验到（§21.17）。
///
/// 现在三态一致：`1` 强制开（含常驻资源）、`0` 强制关（连资源都不建，省显存）、
/// 未设时跟随配置。
pub fn pt_resident_needed(configured: bool, live_env: Option<&str>) -> bool {
    match live_env {
        Some("0") => false,
        Some("1") => true,
        _ => configured,
    }
}

/// PT 实时渲染分辨率（纯函数，可单测）：`(窗口宽, 窗口高, RV3D_PT_SIZE)` → `(w, h)`。
///
/// 三态：`RV3D_PT_SIZE` 合法（128..=4096 且 8 的倍数）时**等比**缩放到该宽度；
/// 未设或非法时跟随窗口；两者都对齐到 8 的倍数（`%8==0` 是 PT 图像的硬要求），
/// 且窗口退化为 0 时也不会返回 0（驱动不接受 0 尺寸）。
///
/// 🔴 2026-09-25 修：注释一直写着"单值覆盖（**等比**）"，但实现只改了宽、高取窗口高，
/// 于是 `RV3D_PT_SIZE=512` 在 2560x1600 的窗口上得到 **512x1600** 的压扁图 ——
/// PT 参照帧与功耗 A/B 都因此失去可比性（判据 = 本函数上方那条单测）。
pub fn pt_render_extent(win_w: u32, win_h: u32, size_env: Option<u32>) -> (u32, u32) {
    let snap = |v: u32| (v & !7).max(8);
    let (w, h) = (snap(win_w.max(64)), snap(win_h.max(64)));
    match size_env {
        Some(s) if (128..=4096).contains(&s) && s % 8 == 0 => {
            // 等比：高 = 窗口高 × (s / 窗口宽)，再对齐 8
            let scaled = (h as f32 * (s as f32 / w as f32)).round() as u32;
            (s, snap(scaled.max(8)))
        }
        _ => (w, h),
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
/// 必须与 build.rs 的 `EMISSIVE_INSTANCE_BASE`（`NPC_INSTANCE_BASE + 9216`，即
/// **三个 NPC 几何区各 3072**）同步 —— 这条注释 2026-09-26 之前写的是 `+ 3072`，
/// 照它改容量会算错 9216-3072=6144 个槽。判据 = `instance_slot_layout_tests::
/// gun_slot_layout_is_pinned` + `marker_band_does_not_bleed_into_npc_band`。
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
pub(crate) struct InstanceData {
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
                // 🔴 Block 必须用**橙色**，不能用纯绿：mesh 着色器的树冠兜底判据是
                //    `is_foliage(tint) = g > r && g > b * 1.4`（`build.rs:1116`），
                //    纯绿 [0,1,0] 两条全中 ⇒ 本开关一开，所有 Shape::Legacy 的方块
                //    会被改画成二十面体（`build.rs:1380` 的 is_tree），
                //    于是"让几何自报家门"的诊断图**自己造出一个四尖星伪影**（§27.5）。
                //    橙色 g=0.55 < r=1.0 ⇒ 这条判据永远为假，且与其余五色一眼可区分。
                static DEBUG_KIND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                if *DEBUG_KIND.get_or_init(|| std::env::var("RV3D_DEBUG_KIND").is_ok()) {
                    let c = match ob.kind {
                        ObstacleKind::Wall => [1.0, 0.0, 0.0],      // 红
                        ObstacleKind::Block => [1.0, 0.55, 0.0],    // 橙（见上：绿色会触发 is_tree）
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

    debug_utils: Option<DebugUtils>,

    debug_messenger: Option<vk::DebugUtilsMessengerEXT>,
    surface_loader: Surface,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,

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
    /// **窗口的物理尺寸**（`Window::inner_size()`），由 `new()` 播种、`Resized` 更新。
    ///
    /// 为什么需要它（2026-09-28，Linux 适配）：Wayland 下
    /// `VkSurfaceCapabilitiesKHR::currentExtent` 是**未定义**的（`UINT32_MAX`），
    /// 交换链尺寸必须由应用自己按窗口尺寸算 —— 没有这个字段就只能写死一个常数，
    /// 结果就是「交换链恒 1280x720、永远不跟随窗口」（见 `swapchain_extent_choice`）。
    /// 这个字段就是那条路唯一缺的输入。
    window_extent: vk::Extent2D,
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
    /// 连续围栏超时判定 GPU 卡死后的降级开关（见 `render()` 开头）
    gpu_stalled: bool,
    /// 重建交换链**中途失败**后的降级开关（见 `recreate_swapchain`）：失败时句柄可能
    /// 已被销毁 ⇒ 在恢复前不许再提交帧。下一次重建成功即清除。
    swapchain_broken: bool,
    /// 阴影 pass 的重画间隔（帧）：`RV3D_SHADOW_EVERY`，默认 **2**（隔帧）。
    ///
    /// 依据（2026-09-26 帧预算地图，`perf_run -NoShadow` 的 A/B）：阴影 pass 占约 **32%**
    /// 帧时间，而**每帧真的会动的只有 NPC 的箱子** —— 太阳方向静止、道具与地形是静态几何
    /// ⇒ 隔帧重画只让 NPC 的影子旧一帧（10ms @100fps，肉眼不可见），代价换来约一半的阴影开销。
    /// **1 = 每帧重画**（与旧行为逐帧一致，用于 A/B 判定）。
    shadow_every: u32,
    /// 阴影是否拆成**静态图 + 动态图**两张（默认开；`RV3D_NO_SHADOW_SPLIT=1` 回到旧行为做 A/B）。
    /// 依据见 `shadow_dyn_image` 字段注释。
    shadow_split: bool,
    /// 静态阴影图的重画间隔（帧）：`RV3D_SHADOW_STATIC_EVERY`，默认 **30**。
    /// 世界不动时静态图内容就不变，所以它只需要偶尔重画（安全网：万一静态内容变了 —— 换关、
    /// 道具重传、太阳转动 —— 最多 30 帧后自动修正）。
    shadow_static_every: u64,
    /// 本帧要不要重画静态阴影图（由 `render()` 用 `shadow_static_due` 算好，录制里只读）
    shadow_static_frame: bool,
    /// 本帧要不要画阴影（由 `render()` 用 `shadow_due` 算好，`record_command_buffer` 只读）
    shadow_frame: bool,
    /// `render()` 的单调帧序号（⚠️ 与在飞槽位 `current_frame` 是两回事，别混）
    frame_seq: u64,
    /// 设备丢失（`VK_ERROR_DEVICE_LOST`）= **不可恢复**：一旦置位就不再提交、不再重建。
    /// 2026-09-26 实测代价：PT blit 的越界目标范围把设备打掉之后，尺寸自检**每帧**重试重建，
    /// 12 秒里跑了 1961 轮、刷了 5900 行错误日志，而进程看着还活着（一帧都画不出来）。
    /// 判据 = `device_lost_stops_rebuilding` / `swapchain_recovery_allowed` 的单测。
    device_lost: bool,
    /// 上一次交换链重建尝试的时刻（失败后限流用，见 `swapchain_recovery_allowed`）
    last_recreate_attempt: Instant,
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
    /// 连续呈现卡顿的帧数（见 `present_stall`）。正常帧必须清零 ——
    /// 漏了清零，几次**偶发**长卡顿会累积成"连续三次"从而误触发降级。
    present_stall_frames: u32,
    depth_images: Vec<vk::Image>,
    depth_images_memory: Vec<vk::DeviceMemory>,
    depth_image_views: Vec<vk::ImageView>,
    // ---- 阴影贴图（2026-08-11：depth-only pass 渲光空间深度，主 pass 3x3 PCF）----
    shadow_image: vk::Image,
    shadow_image_memory: vk::DeviceMemory,
    shadow_image_view: vk::ImageView,
    shadow_sampler: vk::Sampler,
    /// 🧊 磨砂玻璃的背景模糊图（固定 `MENU_BLUR_W × MENU_BLUR_H`，见 `init_menu_blur`）
    menu_blur_image: vk::Image,
    menu_blur_memory: vk::DeviceMemory,
    menu_blur_view: vk::ImageView,
    menu_blur_sampler: vk::Sampler,
    /// 🧊 HUD 玻璃描述符集（绑定 `glass_tex` / `glass_smp`，全局唯一一份）
    hud_glass_set_layout: vk::DescriptorSetLayout,
    hud_glass_pool: vk::DescriptorPool,
    hud_glass_set: vk::DescriptorSet,
    /// 本帧 HUD 里有没有磨砂玻璃 quad（由 `set_hud_quads` 算；真 ⇒ 走"模糊 + overlay 画 HUD"）
    hud_has_glass: bool,
    /// 🧊 磨砂玻璃总开关（`RV3D_MENU_GLASS=0` 关，用于同机位 A/B 与回退）
    menu_glass_enabled: bool,
    shadow_render_pass: vk::RenderPass,
    shadow_framebuffer: vk::Framebuffer,
    /// **动态**阴影图（第二张，与静态图同格式同尺寸）：只画每帧会动的投射者（NPC/士兵）。
    ///
    /// 为什么要两张（2026-09-26 实测，见 §21.27(e)/§21.38）：成本地图显示阴影 pass ≈18% 帧时间，
    /// 而把**静态**投射者（地形/地面场/marker/道具）单独关掉就能拿回几乎全部（`skip_static` 与
    /// `-NoShadow` 同档），只关动态投射者（NPC/士兵）则几乎不变 ⇒ 静态几何才是那 18% 的来源。
    /// 于是：静态图**隔一段时间才重画**（世界不动时内容就不变），动态图每帧重画（只画 NPC，
    /// 三角形数极少）⇒ 静态几何从每帧重画里彻底拿掉，而 NPC 影子反而**比原来更实时**
    /// （原来是整张图隔帧重画）。
    shadow_dyn_image: vk::Image,
    shadow_dyn_image_memory: vk::DeviceMemory,
    shadow_dyn_image_view: vk::ImageView,
    shadow_dyn_framebuffer: vk::Framebuffer,
    /// SHADER_READ_ONLY_OPTIMAL —— 拆分后它可能隔着几十帧才重画，中间一直被主 pass 采样）。
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
    /// `device_wait_idle` 失败告警的一次性闩（2026-09-26 复查）。
    ///
    /// 理由同 `npc_cap_warned`：6 处"等空闲再销毁在飞资源"的关键路径会反复跑
    /// （换枪 / 道具重载 / PT 场景重建），每次失败都记会刷屏。
    /// 见 `wait_idle_failure_message`。
    wait_idle_warned: bool,
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
    /// 本帧相机位置（与 `frame_frustum` 同一处填）。用途：`RV3D_PROP_STATS=1` 的距离直方图
    /// —— 它回答"按距离剔除还有多少可剔"，而 `record_command_buffer` 里拿不到 `view`。
    frame_cam_pos: glam::Vec3,
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
        // 🧊 磨砂玻璃的背景图 / 采样器 / 描述符集：**必须早于 `init_hud`**（HUD 的
        // pipeline layout 引用 `hud_glass_set_layout`）。
        renderer.init_menu_blur()?;
        renderer.init_hud_glass_set()?;
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
                    .map_err(|e| format!("创建调试报告器失败: {e}"))?
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
        // 设备**实际支持**的扩展名（只枚举一次；下面 mesh 与光追两组都基于它判定）。
        // 旧代码把这次枚举关在 mesh 那个块里，于是光追那组只能"先无条件请求、事后打日志"——
        // 那正是"缺扩展就起不来"的来源。
        let device_ext_names: Vec<String> = unsafe {
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
        // ---- 可选网格着色器路径：检测 VK_EXT_mesh_shader（仿 gpu_caps.rs 枚举模式）。
        //      设备没有 VK_EXT_mesh_shader 时 mesh_enabled=false，设备创建与旧代码逐字节一致。
        //      （历史上这条是在 WSLg/dzn 上实测出来的缺失；那段环境已作废，但回退路径照旧有效。）
        //      支持时：扩展加入 enabled_extension_names，并把
        //      PhysicalDeviceMeshShaderFeaturesEXT(mesh_shader=true) 挂到 pNext 链
        //      （task_shader 不启用：本设计为纯 mesh 阶段，无 task 阶段）。
        let mesh_shader_available = {
            if device_ext_names.iter().any(|n| n == "VK_EXT_mesh_shader") {
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

        // 光追扩展组：**全有或全无**（理由见 `pick_device_extensions` 的文档）。
        // 只有真的全齐、且 mesh 路径可用时才启用 —— PT 是默认关的可选功能，
        // 缺扩展的正确后果是"本局没有 RT"，不是"游戏起不来"。
        const RT_DEVICE_EXTENSIONS: [&str; 5] = [
            "VK_KHR_buffer_device_address",
            "VK_KHR_deferred_host_operations",
            "VK_KHR_acceleration_structure",
            "VK_KHR_ray_query",
            "VK_KHR_ray_tracing_pipeline",
        ];
        let (rt_enabled, rt_missing) =
            pick_device_extensions(&device_ext_names, &[], &RT_DEVICE_EXTENSIONS);
        let rt_available = mesh_shader_available && rt_missing.is_empty();
        if rt_missing.is_empty() {
            log::info!("device-create: 光追扩展全齐，启用 {:?}", rt_enabled);
        } else {
            log::warn!(
                "device-create: 光追扩展缺 {:?} ⇒ **整组不启用**（半套是未定义行为）；\
                 本局无 RT/PT，其余渲染路径不受影响",
                rt_missing
            );
        }

        // 设备创建：按**实际支持**逐个启用（mesh 可用时追加 mesh；光追组全齐才追加）。
        let mut device_extensions: Vec<RawCString> = vec![swapchain_ext_name.as_ptr()];
        if mesh_shader_available {
            device_extensions.push(mesh_shader_ext_name.as_ptr());
        }
        if rt_available {
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
        // RT 特性链（Ext 启用 ≠ Feature 启用；rayQuery/accelStructure 必须显式 true）。
        // 🔴 **必须与扩展启用同步**：只启用扩展不启用特性 = 功能不可用；
        // 而**扩展没启用却把特性结构挂进 pNext 本身就是无效用法** ——
        // 旧代码无条件挂这三个，是这次一并修掉的第二处。
        let device_create_info = if rt_available {
            device_create_info
                .push_next(&mut as_features)
                .push_next(&mut bda_features)
                .push_next(&mut rq_features)
        } else {
            device_create_info
        };

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
            // 播种窗口物理尺寸（见字段文档：Wayland 的 currentExtent 未定义时，
            // 这是交换链尺寸唯一的真实来源）。此刻窗口可能还没收到首个 configure，
            // 尺寸为 0 ⇒ `swapchain_extent_choice` 会退到兜底值，
            // 随后 `Resized` 会更新本字段并重建。
            window_extent: {
                let s = window.inner_size();
                vk::Extent2D {
                    width: s.width,
                    height: s.height,
                }
            },
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
            swapchain_broken: false,
            // 阴影拆两张（静态图 + 动态图，见 shadow_dyn_image）：静态图偶尔重画、动态图按
            // `shadow_every` 的节奏重画。默认 **2**（与拆分前整图的节奏一致），于是
            // `RV3D_NO_SHADOW_SPLIT=1` 就是逐帧等价的对照组；=1 可让 NPC 影子更实时。
            shadow_every: std::env::var("RV3D_SHADOW_EVERY")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|v| (1..=8).contains(v))
                .unwrap_or(2),
            shadow_split: std::env::var("RV3D_NO_SHADOW_SPLIT").is_err(),
            shadow_static_every: std::env::var("RV3D_SHADOW_STATIC_EVERY")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| (1..=600).contains(v))
                .unwrap_or(30),
            shadow_static_frame: true, // 首帧必须画（此时静态图里还没有任何内容）
            shadow_frame: true, // 首帧（以及启动时那批 dummy 录制）必须画
            frame_seq: 0,
            device_lost: false,
            last_recreate_attempt: Instant::now(),
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
            present_stall_frames: 0,
            depth_images: Vec::new(),
            depth_images_memory: Vec::new(),
            depth_image_views: Vec::new(),
            shadow_image: vk::Image::null(),
            shadow_image_memory: vk::DeviceMemory::null(),
            shadow_image_view: vk::ImageView::null(),
            shadow_sampler: vk::Sampler::null(),
            menu_blur_image: vk::Image::null(),
            menu_blur_memory: vk::DeviceMemory::null(),
            menu_blur_view: vk::ImageView::null(),
            menu_blur_sampler: vk::Sampler::null(),
            hud_glass_set_layout: vk::DescriptorSetLayout::null(),
            hud_glass_pool: vk::DescriptorPool::null(),
            hud_glass_set: vk::DescriptorSet::null(),
            hud_has_glass: false,
            menu_glass_enabled: match std::env::var("RV3D_MENU_GLASS") {
                Ok(v) => !(v == "0" || v.eq_ignore_ascii_case("false")),
                Err(_) => true,
            },
            shadow_render_pass: vk::RenderPass::null(),
            shadow_framebuffer: vk::Framebuffer::null(),
            shadow_dyn_image: vk::Image::null(),
            shadow_dyn_image_memory: vk::DeviceMemory::null(),
            shadow_dyn_image_view: vk::ImageView::null(),
            shadow_dyn_framebuffer: vk::Framebuffer::null(),
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
            wait_idle_warned: false,
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
            frame_cam_pos: glam::Vec3::ZERO,
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

        let extent = swapchain_extent_choice(
            surface_capabilities.current_extent,
            self.window_extent,
            surface_capabilities.min_image_extent,
            surface_capabilities.max_image_extent,
        );

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
        // 动态阴影图（binding=10；build.rs 片元 `shadow_dyn_map`，静态图是 binding 5）
        let shadow_dyn_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(SHADOW_DYN_BINDING)
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
            shadow_dyn_binding,
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
                // binding 1 地面贴图 + binding 5 静态阴影图 + binding 7/8 marker/NPC 皮肤纹理
                // + binding 9 地面微细节层 + binding 10 动态阴影图（缺一个 = 该 set 分配失败
                // → 启动即报错）。加采样图绑定**必须同时改这个数**。
                .descriptor_count((max_frames * 6) as u32),
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
    /// mesh_enabled=false（设备没有 VK_EXT_mesh_shader）时直接返回，不加载 mesh.spv、不创建任何资源。
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
            // 🧊 location 2 = `uv_glass`（vec3，见 `HudVertex`）。**与 overlay 那份必须一致**。
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::size_of::<HudVertex>() as u32 - std::mem::size_of::<[f32; 3]>() as u32),
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

        // 独立 pipeline layout：🧊 只带一个 set（HUD 玻璃：`glass_tex` + `glass_smp`）。
        // 🔴 **两个 HUD 管线共用这一个 layout**（主 pass 的 `hud_pipeline` 与 overlay 的），
        // 而着色器**静态**引用了 binding 0/1 ⇒ 两边都必须带这套 set（运行时 `glass=0` 也照样
        // 需要有合法描述符绑定，见 `set_hud_quads` 与两处 HUD 绘制里的 `cmd_bind_descriptor_sets`）。
        let hud_set_layouts = [self.hud_glass_set_layout];
        let hud_layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&hud_set_layouts)
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
        // 🧊 本帧有没有磨砂玻璃面板：有 ⇒ `record_command_buffer` 会把 HUD 挪到
        // overlay pass、并在它之前把交换链降采样成模糊图（见 `MENU_BLUR_W`）。
        // 判定放在这里（唯一的上传入口），main.rs / ui.rs 都不需要额外接线。
        //
        // 🔴 PT 模式下**一律当没有玻璃**：那条路把 PT 图 blit 上来再叠 HUD，没有"面板背后的
        // 光栅画面"这回事；更要紧的是 PT 的 HUD 绘制与主 pass 的 HUD 绘制互斥，
        // 若这里留真值，主 pass 那份 HUD 会被跳过而 PT 那份又采不到正确的模糊图。
        let glass_on = self.menu_glass_enabled && !self.pt_live_enabled;
        self.hud_has_glass = glass_on && quads.iter().take(count).any(|q| q.glass);
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
            // 屏幕归一化 UV（与 NDC 同一套换算，只是不乘 2 也不减 1）—— 模糊图是整个屏幕的
            // 降采样副本，所以 uv 直接就是"这一点在屏幕上的位置"。
            let (u0, v0) = (q.rect.x / w, q.rect.y / h);
            let (u1, v1) = ((q.rect.x + q.rect.w) / w, (q.rect.y + q.rect.h) / h);
            // PT 模式（或总开关关掉）时把标志位写成 0 —— 顶点格式恒定，只有这个分量在变
            let g = if q.glass && glass_on { 1.0 } else { 0.0 };
            for (px, py, u, v) in [
                (x0, y0, u0, v0),
                (x1, y0, u1, v0),
                (x0, y1, u0, v1),
                (x1, y0, u1, v0),
                (x1, y1, u1, v1),
                (x0, y1, u0, v1),
            ] {
                verts.push(HudVertex {
                    pos: [px, py],
                    color,
                    uv_glass: [u, v, g],
                });
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
        // 动态阴影图是否参与采样（两张图取 max）。放在这里统一置位，免得依赖
        // game.rs 构造 LightUniform 时是否知道"阴影拆了两张"这件事。
        self.light_data.shadow.config.z = if self.shadow_split
            && self.shadow_dyn_image_view != vk::ImageView::null()
        {
            1.0
        } else {
            0.0
        };
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


































    /// 等 GPU 空闲 + **失败留痕**（`wait_idle_failure_message`）。
    ///
    /// 全仓 6 处"等空闲再销毁/重建在飞资源"统一走这里：原先每处都是
    /// `let _ = self.device.device_wait_idle();` —— 等待失败与等待成功在日志里无法区分。
    /// 判据 = `device_wait_idle_errors_are_never_silently_dropped`（源码扫描，改回 `let _ =` 即红）。
    unsafe fn wait_idle_checked(&mut self) {
        let err = match self.device.device_wait_idle() {
            Ok(()) => return,
            Err(e) => e,
        };
        if let Some(msg) = wait_idle_failure_message(err, self.wait_idle_warned) {
            self.wait_idle_warned = true;
            log::warn!("{msg}");
        }
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
            self.wait_idle_checked();
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
            // 🔴 超时必须有限（`u64::MAX` = 无限等 ⇒ 铁律 B 那个"静默卡死"的同一形态；
            // 2026-09-25 复查时这句是**漏网的一处**，判据 `no_unbounded_wait_on_vulkan_calls`）。
            // ⚠️ 超时后**故意不释放**这条命令缓冲：它可能仍在 pending（释放 = 未定义行为），
            // 代价是每次超时漏一条一次性命令缓冲 —— 比 UB 便宜得多。同理那条围栏仍是 pending，
            // 下一次截图 `reset_fences` 会踩 UB；但这一路径只在 GPU 已经卡住时才可达
            // （那种情况下主循环的围栏超时判定会先 `gpu_stalled`，画面本来就不再更新）。
            self.device
                .wait_for_fences(&[fence], true, SCREENSHOT_WAIT_TIMEOUT_NS)
                .map_err(|e| format!("等待截图围栏失败（限时 {}s）: {}", SCREENSHOT_WAIT_TIMEOUT_NS / 1_000_000_000, e))?;
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
    /// 建一张阴影图（image + 显存 + view）。静态图与动态图除用途外完全同构，
    /// 所以创建代码只留这一份（加第二张图时把它从内联收口成函数）。
    fn create_shadow_map_image(&self) -> Result<(vk::Image, vk::DeviceMemory, vk::ImageView), String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;
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
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe {
            self.device
                .create_image(&image_info, None)
                .map_err(|e| format!("创建阴影图 Image 失败: {}", e))?
        };
        let mem_reqs = unsafe { self.device.get_image_memory_requirements(image) };
        let memory_type = self.pick_memory_type(mem_reqs, true)?;
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(memory_type);
        let memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配阴影图 Image 内存失败: {}", e))?
        };
        unsafe {
            self.device
                .bind_image_memory(image, memory, 0)
                .map_err(|e| format!("绑定阴影图 Image 内存失败: {}", e))?;
        }
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
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
        let view = unsafe {
            self.device
                .create_image_view(&view_info, None)
                .map_err(|e| format!("创建阴影图 Image View 失败: {}", e))?
        };
        Ok((image, memory, view))
    }

    /// 建一个阴影 pass 的 framebuffer（单附件 = 那张图的 view）。
    /// 静态图与动态图共用 `shadow_render_pass`，所以只有附件不同。
    fn create_shadow_framebuffer(&self, view: vk::ImageView) -> Result<vk::Framebuffer, String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;
        let attachments = [view];
        let info = vk::FramebufferCreateInfo::default()
            .render_pass(self.shadow_render_pass)
            .attachments(&attachments)
            .width(SHADOW_MAP_SIZE)
            .height(SHADOW_MAP_SIZE)
            .layers(1);
        unsafe {
            self.device
                .create_framebuffer(&info, None)
                .map_err(|e| format!("创建阴影帧缓冲失败: {}", e))
        }
    }

    /// 🧊 磨砂玻璃的"背景模糊图"（2026-09-26，未结案 #19）：一张**固定尺寸**的降采样副本。
    ///
    /// 为什么固定尺寸（不跟交换链走）：换窗口尺寸时**不用重建**它，描述符集也就不用在
    /// 交换链重建路径里重写 —— 这条路上少一个"忘了重写"的机会（对比阴影图：那是固定
    /// 2048²，同样与交换链无关）。代价只是横竖缩放比不同 ⇒ 模糊半径在两个方向上不等，
    /// 对"磨砂"这件事完全不受影响。
    ///
    /// 建立后**立刻清成深灰并转到 `SHADER_READ_ONLY_OPTIMAL`**：描述符从第一帧起就按这个
    /// 布局绑定，而菜单出现之前根本不会有人写它 ⇒ 不清就是"首帧采到未定义内容"
    /// （与 §21.38 两张阴影图必须 init 时先转布局是同一类问题）。
    fn init_menu_blur(&mut self) -> Result<(), String> {
        let format = self.swapchain_format;
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: MENU_BLUR_W,
                height: MENU_BLUR_H,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe {
            self.device
                .create_image(&image_info, None)
                .map_err(|e| format!("创建菜单模糊图失败: {e}"))?
        };
        let reqs = unsafe { self.device.get_image_memory_requirements(image) };
        let memory_type = self.pick_memory_type(reqs, true)?;
        let memory = unsafe {
            self.device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(reqs.size)
                        .memory_type_index(memory_type),
                    None,
                )
                .map_err(|e| format!("分配菜单模糊图内存失败: {e}"))?
        };
        unsafe {
            self.device
                .bind_image_memory(image, memory, 0)
                .map_err(|e| format!("绑定菜单模糊图内存失败: {e}"))?;
        }
        let view = unsafe {
            self.device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .base_mip_level(0)
                                .level_count(1)
                                .base_array_layer(0)
                                .layer_count(1),
                        ),
                    None,
                )
                .map_err(|e| format!("创建菜单模糊图 View 失败: {e}"))?
        };
        let sampler = unsafe {
            self.device
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                    None,
                )
                .map_err(|e| format!("创建菜单模糊图采样器失败: {e}"))?
        };
        self.menu_blur_image = image;
        self.menu_blur_memory = memory;
        self.menu_blur_view = view;
        self.menu_blur_sampler = sampler;

        // 一次性：UNDEFINED → TRANSFER_DST → 清成深灰 → SHADER_READ_ONLY（之后永远可采样）
        let clear = vk::ClearColorValue {
            float32: [0.02, 0.02, 0.03, 1.0],
        };
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        self.run_single_time_commands(|cb| unsafe {
            let to_dst = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::NONE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(range);
            self.device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_dst],
            );
            self.device
                .cmd_clear_color_image(cb, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &clear, &[range]);
            let to_read = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(range);
            self.device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_read],
            );
        })?;
        Ok(())
    }

    /// 🧊 HUD 玻璃描述符集（`glass_tex` + `glass_smp`）：**只有一份**，不在在飞帧之间复制 ——
    /// 图像与采样器建好后永不改动，也就不存在"写一个正在被读的 set"。
    fn init_hud_glass_set(&mut self) -> Result<(), String> {
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        self.hud_glass_set_layout = unsafe {
            self.device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(|e| format!("创建 HUD 玻璃 set layout 失败: {e}"))?
        };
        let pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(1),
        ];
        self.hud_glass_pool = unsafe {
            self.device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .pool_sizes(&pool_sizes)
                        .max_sets(1),
                    None,
                )
                .map_err(|e| format!("创建 HUD 玻璃 pool 失败: {e}"))?
        };
        let layouts = [self.hud_glass_set_layout];
        self.hud_glass_set = unsafe {
            self.device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(self.hud_glass_pool)
                        .set_layouts(&layouts),
                )
                .map_err(|e| format!("分配 HUD 玻璃 set 失败: {e}"))?
        }[0];
        let image_info = [vk::DescriptorImageInfo::default()
            .image_view(self.menu_blur_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let sampler_info = [vk::DescriptorImageInfo::default()
            .sampler(self.menu_blur_sampler)];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.hud_glass_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.hud_glass_set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&sampler_info),
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        Ok(())
    }

    fn init_shadow_resources(&mut self) -> Result<(), String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;

        // ---- 1. 阴影图 Image + 内存 + View（静态图 + 动态图，见字段注释）----
        let (shadow_image, shadow_image_memory, shadow_image_view) =
            self.create_shadow_map_image()?;
        self.shadow_image = shadow_image;
        self.shadow_image_memory = shadow_image_memory;
        self.shadow_image_view = shadow_image_view;
        let (dyn_image, dyn_memory, dyn_view) = self.create_shadow_map_image()?;
        self.shadow_dyn_image = dyn_image;
        self.shadow_dyn_image_memory = dyn_memory;
        self.shadow_dyn_image_view = dyn_view;

        // 两张图创建后**立刻**转成 SHADER_READ_ONLY_OPTIMAL：它们的描述符（binding 5 / 10）
        // 从第一帧起就按这个布局绑定，而每张图**不一定都会在第一帧被渲染**（关掉拆分时动态图
        // 永不渲染；检视模式下两张都不渲染）。不先转布局就采样一个仍停在 UNDEFINED 的图像 =
        // VUID-vkCmdDraw-None-08114（2026-09-26 实测：RV3D_NO_SHADOW_SPLIT=1 下 11 条）。
        // 转完之后的每次阴影 pass 都以 SHADER_READ_ONLY 为旧布局（见 record_shadow_pass）。
        let maps = [self.shadow_image, self.shadow_dyn_image];
        let barrier_range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::DEPTH)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        self.run_single_time_commands(|cb| unsafe {
            let barriers: Vec<vk::ImageMemoryBarrier> = maps
                .iter()
                .map(|&img| {
                    vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::UNDEFINED)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(img)
                        .subresource_range(barrier_range)
                        .src_access_mask(vk::AccessFlags::empty())
                        .dst_access_mask(vk::AccessFlags::SHADER_READ)
                })
                .collect();
            self.device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barriers,
            );
        })?;

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

        // ---- 4. framebuffer（单附件：阴影图 view）——静态图 + 动态图各一个 ----
        self.shadow_framebuffer = self.create_shadow_framebuffer(self.shadow_image_view)?;
        self.shadow_dyn_framebuffer = self.create_shadow_framebuffer(self.shadow_dyn_image_view)?;

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
            // 动态阴影图（binding 10）：片元在静态图之外再采一张，两张取 max（互不覆盖）。
            // 采样器仍走 binding 6 那条 SAMPLER 写入（两张图共用同一个 sampled-depth 采样器）。
            let shadow_dyn_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.shadow_dyn_image_view)
                .sampler(self.shadow_sampler);
            let shadow_dyn_infos = [shadow_dyn_info];
            let shadow_dyn_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(SHADOW_DYN_BINDING)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&shadow_dyn_infos);

            let writes = [
                sampled_image_write,
                sampler_write,
                shadow_map_write,
                shadow_sampler_write,
                marker_skin_write,
                npc_skin_write,
                ground_detail_write,
                shadow_dyn_write,
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
            // 🧊 location 2 = `uv_glass`（vec3）：磨砂玻璃的屏幕 UV + 标志位。
            // ⚠️ **两个 HUD 管线（主 pass 的 `hud_pipeline` 与 overlay 的 `hud_overlay_pipeline`）
            // 必须同时加**：它们共用 `hud_pipeline_layout` / 同一套着色器，只加一处 =
            // 另一处按 24B 步长解读 36B 顶点 = 静默错位的几何。
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::size_of::<HudVertex>() as u32 - std::mem::size_of::<[f32; 3]>() as u32),
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
        // 🔴 数量 = **在飞帧数**，不是交换链图像数：命令缓冲与 `in_flight_fences[slot]`
        // 一对一，`render()` 按 `current_frame` 取（判据见
        // `command_buffer_is_indexed_by_frame_slot_not_by_swapchain_image`）。
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(self.max_frames_in_flight as u32);

        self.command_buffers = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("分配命令缓冲失败: {}", e))?
        };

        // 占位录制（每帧都会重录）：图像下标只用来选 framebuffer，取模防止
        // 交换链图像数 < 在飞帧数时越界。
        let fbs = self.framebuffers.len().max(1);
        for (i, &command_buffer) in self.command_buffers.iter().enumerate() {
            self.record_command_buffer(command_buffer, i % fbs, INSTANCE_COUNT, 0, TerrainLod::High as usize)?;
        }
        Ok(())
    }


    /// 记录阴影 depth-only pass：布局转换（UNDEFINED → DEPTH_STENCIL_ATTACHMENT_OPTIMAL）
    /// → 渲几何到 2048x2048 阴影图 →（DEPTH_STENCIL_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL）。
    ///
    /// 🔴 **一次调用只画一类投射者**（`draw_static` / `draw_dynamic`），目标图由 `image` +
    /// `framebuffer` 指定 —— 默认路径是"静态图 + 动态图"两张（见 `shadow_dyn_image` 字段注释）：
    /// 静态那类隔一阵子画一次，动态那类每帧画。`RV3D_NO_SHADOW_SPLIT=1` 时调用方改成
    /// "单张图、两类一起画"，与拆分前逐帧等价。
    fn record_shadow_pass(
        &self,
        command_buffer: vk::CommandBuffer,
        near_count: u32,
        far_count: u32,
        terrain_lod: usize,
        image: vk::Image,
        framebuffer: vk::Framebuffer,
        draw_static: bool,
        draw_dynamic: bool,
    ) -> Result<(), String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;

        // 🔬 诊断门（只为测量，不影响默认行为）：把阴影 pass 的**静态**投射者与**动态**投射者
        // 分开关掉，用来回答"阴影那 18% 里，静态占多少"。
        // 依据（2026-09-26 实测）：只关静态 ≈ 关掉整个阴影 pass（`skip_static` 与 `-NoShadow`
        // 同档），只关动态几乎不变 ⇒ 静态几何才是阴影开销的来源，这正是拆两张图的理由。
        // OnceLock：env 只读一次，热路径上没有分配。
        static SKIP_STATIC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        static SKIP_DYNAMIC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        // 第三个门：只跳过**地面实例场**（near/far 实例数置 0），保留地形网格/marker/道具。
        // 目的：地面场是 65536 实例的实例化 draw（顶点量级远大于道具的 246k 三角形），
        // 而它本身就是地面 —— 地面给自己投影几乎没有视觉意义。若它真是阴影 pass 的大头，
        // 那"把地面场从阴影 pass 拿掉"就是一行改动，比两张阴影图简单得多。
        static SKIP_GROUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let skip_ground =
            *SKIP_GROUND.get_or_init(|| std::env::var("RV3D_SHADOW_SKIP_GROUND").is_ok());
        let draw_static =
            draw_static && !*SKIP_STATIC.get_or_init(|| std::env::var("RV3D_SHADOW_SKIP_STATIC").is_ok());
        let draw_dynamic = draw_dynamic
            && !*SKIP_DYNAMIC.get_or_init(|| std::env::var("RV3D_SHADOW_SKIP_DYNAMIC").is_ok());
        // 第四/五个门（同一族诊断）：只跳过**地形网格**（257² 网格，High LOD ≈13 万三角形，
        // 量级最大）或只跳过地面实例场，用来分辨"阴影 pass 的顶点量到底花在哪一层地面上"。
        static SKIP_TERRAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let skip_terrain =
            *SKIP_TERRAIN.get_or_init(|| std::env::var("RV3D_SHADOW_SKIP_TERRAIN").is_ok());
        // 静态组：地形 / 地面实例场 / marker / 道具；动态组：NPC 盒柱球 / 士兵 GLB。
        // 用"把实例数置 0"实现跳过（`draw_shadow_range` 对 0 会早退），不动绘制逻辑本身。
        let (near_count, far_count) = if draw_static && !skip_ground {
            (near_count, far_count)
        } else {
            (0, 0)
        };
        let marker_near = if draw_static { self.last_marker_near } else { 0 };
        let marker_far = if draw_static { self.last_marker_far } else { 0 };
        let npc_box_near = if draw_dynamic { self.last_npc_box_near } else { 0 };
        let npc_box_far = if draw_dynamic { self.last_npc_box_far } else { 0 };
        let npc_cyl_near = if draw_dynamic { self.last_npc_cyl_near } else { 0 };
        let npc_cyl_far = if draw_dynamic { self.last_npc_cyl_far } else { 0 };
        let npc_sph_near = if draw_dynamic { self.last_npc_sph_near } else { 0 };
        let npc_sph_far = if draw_dynamic { self.last_npc_sph_far } else { 0 };
        let soldier_drawn = if draw_dynamic { self.soldier_drawn } else { 0 };
        let skip_static = !draw_static;

        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::DEPTH)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);

        // 进入 attachment 布局：旧布局固定 SHADER_READ_ONLY_OPTIMAL —— 两张图在 init 时就被
        // 转到这个布局（见 init_shadow_resources 的一次性 barrier），而拆分之后静态图可能隔着
        // 几十帧才重画，中间那些帧主 pass 一直在采样它（FRAGMENT_SHADER 读）。写之前必须让
        // 上一次的读可见/完成：srcStage=FRAGMENT_SHADER + srcAccess=SHADER_READ。
        let old_layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
        let src_stage = vk::PipelineStageFlags::FRAGMENT_SHADER;
        let src_access = vk::AccessFlags::SHADER_READ;
        let to_attachment = vk::ImageMemoryBarrier::default()
            .old_layout(old_layout)
            .new_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource)
            .src_access_mask(src_access)
            .dst_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE);
        let to_attachment_barriers = [to_attachment];
        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                src_stage,
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
            .framebuffer(framebuffer)
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
        let terrain_mesh = if skip_static || skip_terrain {
            None
        } else {
            self.terrain_lods.get(terrain_lod)
        };
        if let Some(mesh) = terrain_mesh {
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
            marker_near,
            MARKER_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            marker_far,
            MARKER_SLOT_BASE + marker_near,
        )?;
        // NPC 盒体区（躯干/脚/枪；阴影以盒体近似）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            npc_box_near,
            NPC_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            npc_box_far,
            NPC_SLOT_BASE + npc_box_near,
        )?;
        // NPC 圆柱区（四肢；阴影以盒体近似）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            npc_cyl_near,
            NPC_CYL_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            npc_cyl_far,
            NPC_CYL_SLOT_BASE + npc_cyl_near,
        )?;
        // NPC 球体区（头；阴影以盒体近似）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            npc_sph_near,
            NPC_SPH_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            npc_sph_far,
            NPC_SPH_SLOT_BASE + npc_sph_near,
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
            soldier_drawn,
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
        if !skip_static && sh_ib != vk::Buffer::null() {
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
            .image(image)
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
        // 同理 `swapchain_broken`（见 `recreate_swapchain`）：重建**中途**失败时
        // `swapchain` 等句柄可能已经被销毁，再 acquire/提交就是拿空句柄调 Vulkan。
        // `device_lost` 包含 `swapchain_broken`（置位那一拍就是重建失败），这里并列写出来
        // 是为了让"不可恢复 ⇒ 不提交"这条读起来是显式的。
        if frame_suppressed(self.gpu_stalled, self.swapchain_broken || self.device_lost) {
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

        // 🔴 `suboptimal` **只登记，不许在这里 return**（2026-09-25 深夜复查）：
        // acquire 已经成功 ⇒ `image_available_semaphores[current_frame]` 已被 signal，而该信号量
        // 不会随交换链重建而重建 ⇒ 提前 return 会让它留在 signaled 状态被下一帧复用 = UB 且静默。
        // 这一帧照常走完（record/submit/present），重建统一放到 present 之后，判据见 `frame_action`。
        if suboptimal {
            log::warn!("交换链 SUBOPTIMAL（acquire）—— 本帧照常呈现，随后重建交换链");
        }

        unsafe {
            if let Err(e) = self.device.reset_fences(&[fence]) {
                // 同一条不变式（见 `frame_action` 文档）：这里也已经 acquire 成功过，
                // 直接 `?` 会把 `image_available_semaphores[current_frame]` 留在 signaled 状态
                // 被下一帧复用。既然连围栏都重置不了，设备事实上已经不可用 ⇒ 走既有的
                // `gpu_stalled` 降级：`render()` 之后直接返回，那个信号量**永不再被使用**。
                self.gpu_stalled = true;
                log::error!("重置围栏失败（{}）⇒ 判定 GPU 侧不可用，停止渲染循环", e);
                return Err(format!("重置围栏失败: {}", e));
            }
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
        // 相机位置与视锥同处填：`RV3D_PROP_STATS=1` 的距离直方图要用它（见字段注释）
        self.frame_cam_pos = view.inverse().w_axis.truncate();
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
        //
        // 🔴 `RV3D_NO_GROUND_TEX=1` 关掉这一层（A/B 诊断门）。**这个开关本仓早就写在
        // `renderer.rs:6134` 的注释里**（"与 RV3D_NO_SHADOW / RV3D_NO_GROUND_TEX 同一套惯例"），
        // 但**全仓从未实现过它** —— 拿它做 A/B 会得到"两边完全相同"的假结论
        // （§55 判别时就差点这样把 H1 误判为已否证）。现在补上，并读一次缓存住。
        if self.ground_detail_image_view != vk::ImageView::null() && !no_ground_detail_tex() {
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
        // 🔴 **命令缓冲按「在飞帧槽位」取，不按 `image_index`**（2026-09-25 验证层实测，
        // 判据见 `command_buffer_is_indexed_by_frame_slot_not_by_swapchain_image`）：
        // 围栏 `in_flight_fences[current_frame]` 只保证**这个槽位**的上一次提交已完成，
        // 而 `image_index` 与槽位是两套编号 —— 同一张图像可以被连续两帧 acquire
        // （mailbox 下很常见），那时按图像取就会重录一条**仍 pending** 的命令缓冲
        // （VUID-vkBeginCommandBuffer-commandBuffer-00049 / VUID-vkQueueSubmit-pCommandBuffers-00071）。
        // 图像下标只用来选 framebuffer（见 `record_command_buffer` 的入参）。
        let cmd_buffer = self.command_buffers[self.current_frame];
        // 阴影图隔帧重画（`shadow_every`）：本帧画不画在这里定，`record_command_buffer` 只读。
        // ⚠️ 跳帧时阴影图**保持上一帧的内容**（render pass 的 initialLayout=UNDEFINED + CLEAR
        // 只在真画的那一帧发生），主 pass 照常采样 —— 布局上从 DEPTH_STENCIL_ATTACHMENT_OPTIMAL
        // 到采样所需的 SHADER_READ_ONLY_OPTIMAL 之间那道 barrier 在 `record_shadow_pass` 末尾，
        // 跳帧时图像就停在 SHADER_READ_ONLY_OPTIMAL，主 pass 读它是合法状态。
        self.shadow_frame = shadow_due(self.frame_seq, self.shadow_every, self.void_mode);
        // 静态图单独一条节奏（默认 30 帧一次）：它装的是不动的东西，没必要每帧重画。
        self.shadow_static_frame =
            shadow_static_due(self.frame_seq, self.shadow_static_every, self.void_mode, self.shadow_split);
        self.frame_seq = self.frame_seq.wrapping_add(1);
        self.record_command_buffer(
            cmd_buffer,
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
        let cmd_buffers = [cmd_buffer];

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
        //
        // 2026-09-25 补：本帧的处置交给纯函数 `frame_action` —— 它**必须**拿到 present 结果，
        // 于是"成功 acquire 之后先 present 再决定"由签名保证（理由见该函数文档）。
        match frame_action(suboptimal, classify_present(present_result)) {
            FrameAction::Presented => {}
            FrameAction::RecreateAfterPresent => {
                log::warn!("呈现 {:?}，重建交换链...", present_result);
                return Err("交换链过期".to_string());
            }
            FrameAction::Fail => {
                log::error!("呈现失败（{:?}）—— 不能当成成功", present_result);
                return Err(format!("呈现失败: {:?}", present_result));
            }
        }

        // 🔴 **2026-09-29：`queue_present` 是最后一个没有上界的 Vulkan 等待。**
        //
        // acquire 有 1s 超时、围栏有 5s 超时，但 present **没有** ——
        // 而 Wayland 下 FIFO（合成器不提供 `wp_fifo_v1` 时）**就是在 present 里阻塞等
        // frame callback**，窗口隐藏/最小化时那个回调不会来。后果与 acquire 那次同类：
        // 日志停住、无 panic、无 VUID、无 `has been lost`，从外面看就是"游戏死了"。
        //
        // `vkQueuePresentKHR` 的签名里没有超时参数，加不了超时 ⇒ 判据只能是**耗时**：
        // 超阈值就数一次，连续到阈值就按与 acquire **完全相同**的方式降级到 mailbox 并重建。
        // 放在 `frame_action` **之后**是刻意的：呈现结果的处置是硬不变量，
        // 不能被这里的提前 return 跳过。
        let consecutive = next_stall_count(self.present_stall_frames, self.stage_present_us);
        self.present_stall_frames = consecutive;
        match present_stall(self.stage_present_us, consecutive) {
            PresentStall::Ok => {}
            PresentStall::Warn => {
                if consecutive == 1 {
                    log::error!(
                        "单次呈现耗时 {}ms（阈值 {}ms）—— vkQueuePresentKHR 没有超时参数，\
                         卡在这里主循环就停了。Wayland FIFO 等一个不会来的 frame callback\
                         （窗口不可见时）是已知诱因。连续 {} 次后会降级为 MAILBOX 并重建交换链。",
                        self.stage_present_us / 1000,
                        PRESENT_STALL_US / 1000,
                        PRESENT_STALL_FALLBACK
                    );
                }
            }
            PresentStall::Degrade => {
                if self.present_mode_override != Some(vk::PresentModeKHR::MAILBOX) {
                    log::error!(
                        "连续 {} 次呈现卡顿 ⇒ 降级为 MAILBOX 并重建交换链\
                         （mailbox 会丢弃待呈现图像，不像 FIFO 那样等 frame callback）",
                        consecutive
                    );
                    self.present_mode_override = Some(vk::PresentModeKHR::MAILBOX);
                    return Err("交换链过期".to_string());
                }
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

    /// 重建交换链（窗口尺寸变化 / `交换链过期` / 5 秒尺寸自检三条路都调它）。
    /// 🔴 开头**必须** `wait_idle()`：销毁可能仍在被 pending present 等待的信号量与
    /// framebuffer 是未定义行为（见 `resize_render_finished_semaphores` 的文档）。
    ///
    /// 🔴 2026-09-25 复查补：这个函数**先销毁再重建**，中间有 8 个可能失败的步骤，而
    /// 调用方（`main.rs` 三处）以前都 `let _ =` 把错误丢掉 ⇒ 一旦中途失败，渲染器就带着
    /// **半销毁**的状态继续每帧 acquire/提交（拿空句柄调 Vulkan，日志里只有一串含义不明的报错）。
    /// 现在失败一律置 `swapchain_broken` 降级：不再提交帧，等下一次
    /// 重建**成功**时自动恢复。判据 = `frame_suppressed` 的单测。
    ///
    /// 🔴 2026-09-26 再加一层：失败原因若是**设备丢失**，那"下一次成功"永远不会来
    /// （本引擎没有重建设备的路径），置 `device_lost` 之后 `swapchain_recovery_allowed()`
    /// 恒假 ⇒ 调用方不再重试、不再刷日志。见该字段与 `is_device_lost_error` 的文档。
    /// 现在**值不值得**再试一次交换链重建（`main.rs` 三处重建入口的判据）。
    /// 语义与判据全在纯函数 `should_retry_swapchain` 里，这里只是把"距上次多久"喂进去。
    pub fn swapchain_recovery_allowed(&self) -> bool {
        should_retry_swapchain(
            self.device_lost,
            self.swapchain_broken,
            self.last_recreate_attempt.elapsed().as_secs_f32(),
        )
    }

    /// 更新「窗口物理尺寸」（`Window::inner_size()`）。**必须在 `recreate_swapchain()`
    /// 之前调用**，否则 Wayland 下重建出来的交换链还是上一次的尺寸（`currentExtent`
    /// 未定义 ⇒ 尺寸只能来自这里，见 `window_extent` 字段与 `swapchain_extent_choice`）。
    ///
    /// 为什么不让 `init_swapchain` 自己去问窗口：`init_swapchain` 只有 `&mut self`，
    /// 拿不到 `Window`，而 Vulkan 的 surface 创建与窗口生命周期是分开的两件事
    /// （见 `Renderer::new(window: &Window)`）—— 把窗口尺寸作为**显式输入**传进来，
    /// 比让渲染器持有窗口引用更不容易出借用冲突，也让这条依赖在类型上可见。
    pub fn set_window_extent(&mut self, width: u32, height: u32) {
        self.window_extent = vk::Extent2D { width, height };
    }

    pub fn recreate_swapchain(&mut self) -> Result<(), String> {
        self.last_recreate_attempt = Instant::now();
        let r = self.try_recreate_swapchain();
        self.swapchain_broken = r.is_err();
        if let Err(e) = &r {
            if is_device_lost_error(e) {
                // 不可恢复：把结论**讲明白一次**，然后彻底停下来
                self.device_lost = true;
                log::error!(
                    "设备已丢失（VK_ERROR_DEVICE_LOST，不可恢复）：停止提交与交换链重建，需要重启进程。原因：{}",
                    e
                );
            } else {
                log::error!(
                    "重建交换链失败：{} —— 进入降级（不再提交帧；下一次重建成功即恢复）",
                    e
                );
            }
        }
        r
    }

    fn try_recreate_swapchain(&mut self) -> Result<(), String> {
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
        // 同 `init_command_buffers`：数量按**在飞帧数**（命令缓冲与围栏槽位一对一）
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(self.max_frames_in_flight as u32);
        self.command_buffers = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("重新分配命令缓冲失败: {}", e))?
        };
        let fbs = self.framebuffers.len().max(1);
        for (i, &command_buffer) in self.command_buffers.iter().enumerate() {
            self.record_command_buffer(command_buffer, i % fbs, INSTANCE_COUNT, 0, TerrainLod::High as usize)?;
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
            self.wait_idle_checked();

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
            // 🧊 释放 HUD 磨砂玻璃的那套资源（2026-09-26 补）：图像 / 内存 / 视图 / 采样器 /
            // 描述符池 + set layout。**这五样是当天新加的，当时没进释放清单** ——
            // 是 `tools/audit_vk_resources.py` 在它自己上线两小时后抓出来的
            // （"no release call found: 5"，全部指向这几个字段）。
            // 顺序：先释放依赖资源的池/layout，再拆 view/sampler，最后 image + memory。
            if self.hud_glass_pool != vk::DescriptorPool::null() {
                self.device.destroy_descriptor_pool(self.hud_glass_pool, None);
                self.hud_glass_pool = vk::DescriptorPool::null();
                self.hud_glass_set = vk::DescriptorSet::null();
            }
            if self.hud_glass_set_layout != vk::DescriptorSetLayout::null() {
                self.device
                    .destroy_descriptor_set_layout(self.hud_glass_set_layout, None);
                self.hud_glass_set_layout = vk::DescriptorSetLayout::null();
            }
            if self.menu_blur_view != vk::ImageView::null() {
                self.device.destroy_image_view(self.menu_blur_view, None);
                self.menu_blur_view = vk::ImageView::null();
            }
            if self.menu_blur_sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.menu_blur_sampler, None);
                self.menu_blur_sampler = vk::Sampler::null();
            }
            if self.menu_blur_image != vk::Image::null() {
                self.device.destroy_image(self.menu_blur_image, None);
                self.menu_blur_image = vk::Image::null();
            }
            if self.menu_blur_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.menu_blur_memory, None);
                self.menu_blur_memory = vk::DeviceMemory::null();
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
            // 动态阴影图（第二张）：与静态图同一套顺序（framebuffer → view → image → memory）
            if self.shadow_dyn_framebuffer != vk::Framebuffer::null() {
                self.device.destroy_framebuffer(self.shadow_dyn_framebuffer, None);
            }
            if self.shadow_dyn_image_view != vk::ImageView::null() {
                self.device.destroy_image_view(self.shadow_dyn_image_view, None);
            }
            if self.shadow_dyn_image != vk::Image::null() {
                self.device.destroy_image(self.shadow_dyn_image, None);
            }
            if self.shadow_dyn_image_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.shadow_dyn_image_memory, None);
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
            // NPC 近似几何：球（头）与圆柱（四肢）。这两对缓冲由 `create_sphere_geometry` /
            // `create_cylinder_geometry` 在 init 时各建一次，**此前不在任何释放表里**
            // （2026-09-26 用 tools/audit_vk_resources.py 扫出来的）：一次性小泄漏，但退出路径
            // 不完整就会被后来照抄这段的人继续放大。
            for (b, m) in [
                (self.sphere_vertex_buffer, self.sphere_vertex_buffer_memory),
                (self.sphere_index_buffer, self.sphere_index_buffer_memory),
                (self.cylinder_vertex_buffer, self.cylinder_vertex_buffer_memory),
                (self.cylinder_index_buffer, self.cylinder_index_buffer_memory),
            ] {
                if b != vk::Buffer::null() {
                    self.device.destroy_buffer(b, None);
                }
                if m != vk::DeviceMemory::null() {
                    self.device.free_memory(m, None);
                }
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

// 子模块（见 docs/refactor-plan.md）
mod geometry;

// 子模块（见 docs/refactor-plan.md）
mod instances;

// 子模块（见 docs/refactor-plan.md）
mod parts;

// 子模块（见 docs/refactor-plan.md）
mod record;

// 子模块（见 docs/refactor-plan.md）
mod pt_assets;

// 子模块（见 docs/refactor-plan.md）
mod pt_render;

#[cfg(test)]
mod tests_support;
#[cfg(test)]
mod tests_geom;
#[cfg(test)]
mod tests_gpu;
#[cfg(test)]
mod tests_vk;
