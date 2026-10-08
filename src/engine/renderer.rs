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
// 子模块里的 `super::procedural::…` 需要这条再导出：那段代码原来就在 renderer.rs（`super` 指 engine），
// 下沉一层后 `super` 变成 renderer。再导出让**搬走的字节保持原样** ——
// 判据 tools/refactor_move_check.py 因此不需要为它加归一化。
pub(crate) use crate::engine::procedural;

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

// 子模块（见 docs/refactor-plan.md）
mod textures;

// 子模块（见 docs/refactor-plan.md）
mod shadow;

// 子模块（见 docs/refactor-plan.md）
mod pipelines;

// 子模块（见 docs/refactor-plan.md）
mod descriptors;

// 子模块（见 docs/refactor-plan.md）
mod device;

// 子模块（见 docs/refactor-plan.md）
mod swapchain;

// 子模块（见 docs/refactor-plan.md）
mod frame;

#[cfg(test)]
mod tests_support;
#[cfg(test)]
mod tests_geom;
#[cfg(test)]
mod tests_gpu;
#[cfg(test)]
mod tests_vk;
