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
// 自由函数搬进 helpers.rs 后，外部路径（如 game.rs 的 super::renderer::terrain_height_at）靠这条再导出维持不变。
pub(crate) use helpers::*;
pub(crate) use gpu_layout::*;

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

const _: () = assert!(
    std::mem::size_of::<HudVertex>() == 36,
    "HudVertex 必须是 36B（pos vec2 + color vec4 + uv_glass vec3）：HUD 着色器按此布局取属性"
);





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
pub(crate) struct QualityParams {
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
pub(crate) enum PresentStall {
    /// 正常 —— 调用方把连续计数**清零**（漏了它，一次偶发卡顿会累积成"连续三次"）
    Ok,
    /// 偶发一次：值得留一条日志（否则就是静默），但**不动行为**
    Warn,
    /// 连续多次：该按 acquire 那套方式降级了
    Degrade,
}



/// 连续 acquire 超时到第几次就**降级到 mailbox 自动恢复**（≈3 秒没图像）
const ACQUIRE_STALL_FALLBACK: u32 = 3;

/// 连续 acquire 超时到第几次就放弃这一帧并报错（≈30 秒没图像，日志里要能被看见）
const ACQUIRE_STALL_MAX: u32 = 30;

/// 连续**围栏**超时到第几次就判定"GPU 侧卡死"（3 × 5s = 15s 没有任何一帧完成）
const FENCE_STALL_MAX: u32 = 3;


/// 像素字节序策略（由 swapchain 像素格式决定）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PixelOrder {
    /// 源为 B,G,R,A 字节序（转 RGBA 需交换 R/B）
    Bgra,
    /// 源为 R,G,B,A 字节序（直接拷贝）
    Rgba,
}



/// 物理设备选择偏好（来自 `RV3D_GPU`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GpuPreference {
    /// 不设 `RV3D_GPU`：有窗口表面的设备里**优先独显**（本仓历史行为）
    Auto,
    Discrete,
    Integrated,
    /// 设备名包含该子串（已转小写）
    Name(String),
}




/// `queue_present` 的结果分类（纯函数，可单测）。
///
/// 🔴 2026-09-22 复查补：此前只处理 `Err(ERROR_OUT_OF_DATE_KHR)` 与 `Ok(true)`（SUBOPTIMAL），
/// **其余 `Err` 一律被静默忽略** —— `ERROR_SURFACE_LOST_KHR` / `ERROR_DEVICE_LOST` 会被
/// 当成"这一帧呈现成功"，主循环继续跑（画面已经死了，帧计数与 fps 照走）。
/// 现在只认 `Ok(false)` 为成功；除"重建交换链"两种之外的 Err 一律升级为错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresentOutcome {
    /// `Ok(false)`：真·呈现成功
    Presented,
    /// OUT_OF_DATE / SUBOPTIMAL：交换链需要重建（这是**成功的一类**，不是失败）
    RecreateSwapchain,
    /// 其它 Err（SURFACE_LOST / DEVICE_LOST / …）：不能当成功
    Failed,
}


/// `acquire_next_image` 的**错误**结果分类（纯函数，可单测）。
///
/// 🔴 2026-09-25 复查补：此前 acquire 用的是 `timeout = u64::MAX`，于是"呈现引擎一直不给图像"
/// 会让主循环**静默卡死**在 acquire 里 —— 日志停住、无 panic、无 VUID、无 `has been lost`，
/// 从外面看就是"游戏死了"（当晚独显 + `defense_line` + IMMEDIATE 的 TDR 就是这个形态）。
/// 现在超时有限（`ACQUIRE_TIMEOUT_NS`），超时会计数、记日志，并最终降级到 mailbox 自动恢复。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AcquireOutcome {
    /// 这一轮没有图像：`TIMEOUT` / `NOT_READY`，可以重试
    Retry,
    /// 交换链要重建（OUT_OF_DATE / SURFACE_LOST）
    RecreateSwapchain,
    /// 不可恢复（DEVICE_LOST 等）
    Failed,
}


/// 一帧**走完呈现之后**的处置（纯函数，可单测）。判据见 `frame_action` 的文档。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameAction {
    /// 本帧正常结束（已呈现、无需重建）
    Presented,
    /// 本帧**已呈现**，随后重建交换链
    RecreateAfterPresent,
    /// 设备级失败（SURFACE_LOST / DEVICE_LOST …）：交回上层
    Fail,
}








/// 交换链重建失败后**多久才允许再试一次**（秒）。见 `should_retry_swapchain`。
const RECREATE_RETRY_MIN_SECS: f32 = 1.0;









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






impl Renderer {















































































































}

// ============================================================
// Drop：释放所有资源
// ============================================================


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

// 子模块（见 docs/refactor-plan.md）
mod drop;

// 子模块（见 docs/refactor-plan.md）
mod helpers;

// 子模块（见 docs/refactor-plan.md）
mod gpu_layout;

#[cfg(test)]
mod tests_support;
#[cfg(test)]
mod tests_geom;
#[cfg(test)]
mod tests_gpu;
#[cfg(test)]
mod tests_vk;
