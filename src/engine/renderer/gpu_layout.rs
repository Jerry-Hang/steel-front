// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。

/// Camera Uniform 数据（view/proj 两个 4x4 矩阵 + lod_params + 网格着色器扩展字段，256 字节）
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct CameraUniform {
    pub(crate) view: glam::Mat4,
    pub(crate) proj: glam::Mat4,
    /// (terrain_lod_high_end, fade_start, fade_end, terrain_lod_med_end)
    /// x/w：地形网格 LOD 切换距离（shader 不读这两个分量，仅 CPU 侧语义扩展）
    /// y/z：实例远档十字 quad 地面淡出区间（shader 读取，语义保持不变）
    pub(crate) lod_params: [f32; 4],
    /// 视锥 6 平面（Gribb–Hartmann，法线朝内、归一化）。仅网格着色器读取；
    /// 传统顶点着色器声明的 ViewProj 只读前 144 字节，本扩展字段对其透明。
    pub(crate) planes: [[f32; 4]; 6],
    /// xyz = 相机世界位置，w = 近档距离²（几何 LOD 切换阈值）。仅网格着色器读取。
    pub(crate) cam_pos: [f32; 4],
}

// 光照 Uniform 类型与布局由 lighting 模块统一维护（`lighting::LightUniform`，352 字节）。
// 默认全零 = 光照关闭：片元着色器走原「纹理+顶点颜色 50% 混合」路径，向后兼容。
/// 立方体顶点数据
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub(crate) struct Vertex {
    pub(crate) pos: [f32; 3],
    pub(crate) color: [f32; 3],
    pub(crate) uv: [f32; 2],
}
// 步长契约：主管线把它当 `stride` 用（`size_of::<Vertex>()`），而**着色器把
// `location 0/1/2 = pos/color/uv` 与 32B 步长当成既成事实**（`build.rs` 的 mesh 路径
// 与 `铁律 B` 的 "stride=32" 都这么写）。加字段/加填充会让属性整体错位，
// 而 Vulkan 与驱动**都不报错**（只是画面默默变错）⇒ 在这里把它钉死。
/// HUD 覆盖层顶点：屏幕空间 NDC 位置（Y 已翻转）+ RGBA 颜色
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct HudVertex {
    pub(crate) pos: [f32; 2],
    pub(crate) color: [f32; 4],
    /// 🧊 `(u, v, glass)`：u/v = 该顶点在**屏幕上的归一化位置**（磨砂玻璃按它采样
    /// 那张降采样模糊图）；`glass` = ui.rs 的 `Quad::glass` 标志（0/1）。
    /// 非玻璃 quad 照样写 uv（不额外分支），片元按 `glass` 决定用不用它。
    pub(crate) uv_glass: [f32; 3],
}
// HUD 覆盖层自己的步长契约（与主管线的 `Vertex` 无关）：`hud.vert.spv` 按
// `pos vec2 + color vec4 + uv_glass vec3` 取属性，步长由这里推导 ⇒ 改动同样必须在这里被挡住。
/// 立方体 24 顶点（每面 4 个，CCW 外侧绕序；每面 UV 铺满 0..1）
pub(crate) const VERTICES: [Vertex; 24] = [
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
pub(crate) const INDICES: [u32; 36] = [
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
pub(crate) const FAR_VERTS: [Vertex; 8] = [
    Vertex { pos: [-1.0, -1.0,  0.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [ 1.0, -1.0,  0.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [ 1.0,  1.0,  0.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [-1.0,  1.0,  0.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
    Vertex { pos: [ 0.0, -1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [ 0.0, -1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [ 0.0,  1.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [ 0.0,  1.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
];
pub(crate) const FAR_INDICES: [u32; 12] = [
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
pub(crate) const GROUND_VERTS: [Vertex; 4] = [
    Vertex { pos: [-1.0, 0.0,  1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 0.0] },
    Vertex { pos: [ 1.0, 0.0,  1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 0.0] },
    Vertex { pos: [ 1.0, 0.0, -1.0], color: [1.0, 1.0, 1.0], uv: [1.0, 1.0] },
    Vertex { pos: [-1.0, 0.0, -1.0], color: [1.0, 1.0, 1.0], uv: [0.0, 1.0] },
];
pub(crate) const GROUND_INDICES: [u32; 6] = [0, 2, 1, 0, 3, 2];
/// 实例网格（256×256 = 65536）
pub(crate) const GRID_SIZE: u32 = 256;
pub(crate) const INSTANCE_COUNT: u32 = GRID_SIZE * GRID_SIZE;
/// 世界障碍 marker 上限（程序化地图每关障碍盒数远小于此；同时决定实例 buffer 的额外容量）
pub(crate) const MAX_MARKER_INSTANCES: u32 = 8192;
/// marker 在实例 buffer 中的起始 slot：跳过 0..=INSTANCE_COUNT。
///
/// slot 65536 是地形 identity（shader 硬编码 TERRAIN_INSTANCE_INDEX=65536 读取，
/// cull_and_upload 只写 0..visible-1，永不触碰），marker 从 65537 起，互不干扰。
pub(crate) const MARKER_SLOT_BASE: u32 = INSTANCE_COUNT + 1;
/// NPC 士兵可视化段数上限（每几何区；人形 15 段/人 × 8 NPC = 120 段，余量充足；
/// 同时决定实例 buffer 的额外容量。三几何区：盒（躯干/脚/枪）、圆柱（四肢）、球（头））
pub(crate) const MAX_NPC_INSTANCES: u32 = 3072; // 2026-08-23：128v128 + 尸体/存活混合实测峰值 2220
/// NPC 盒体段区起始 slot：紧接 marker 区之后（见 MARKER_SLOT_BASE）
pub(crate) const NPC_SLOT_BASE: u32 = MARKER_SLOT_BASE + MAX_MARKER_INSTANCES;
/// NPC 圆柱段区（四肢）起始 slot
pub(crate) const NPC_CYL_SLOT_BASE: u32 = NPC_SLOT_BASE + MAX_NPC_INSTANCES;
/// NPC 球体段区（头）起始 slot
pub(crate) const NPC_SPH_SLOT_BASE: u32 = NPC_CYL_SLOT_BASE + MAX_NPC_INSTANCES;
/// 自发光实体上限（爆炸闪光等瞬时特效，并发数远小于此）
pub(crate) const MAX_EMISSIVE_INSTANCES: u32 = 64;
/// 自发光实体在实例 buffer 中的起始 slot：紧接 NPC 区之后（见 NPC_SPH_SLOT_BASE）。
/// 必须与 build.rs 的 `EMISSIVE_INSTANCE_BASE`（`NPC_INSTANCE_BASE + 9216`，即
/// **三个 NPC 几何区各 3072**）同步 —— 这条注释 2026-09-26 之前写的是 `+ 3072`，
/// 照它改容量会算错 9216-3072=6144 个槽。判据 = `instance_slot_layout_tests::
/// gun_slot_layout_is_pinned` + `marker_band_does_not_bleed_into_npc_band`。
pub(crate) const EMISSIVE_SLOT_BASE: u32 = NPC_SPH_SLOT_BASE + MAX_NPC_INSTANCES;
/// 枪模专用 identity 槽（走 flat=1 纯色路径，与 build.rs 顶点 shader 同步）
pub(crate) const GUN_INSTANCE_INDEX: u32 =
    INSTANCE_COUNT + 1 + MAX_MARKER_INSTANCES + MAX_NPC_INSTANCES * 3 + MAX_EMISSIVE_INSTANCES;
/// GLB 道具合并网格专用 identity 槽。
///
/// 道具的位姿在 CPU 上就烘进顶点（`props::merge`），所以 GPU 侧只需要一个 identity 矩阵，
/// 和地形/枪模同一个套路——**不必为道具新增一整段实例区**，只多占一个槽位。
/// 该槽的 `tint.w = Shape::Authored.tag()`(=6.0) 是给 shader 看的，`tint.rgb = 1` 让
/// 片元直出顶点色（`input.color = vertexColor × tint.rgb`）。
pub(crate) const PROP_INSTANCE_INDEX: u32 = GUN_INSTANCE_INDEX + 1;
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
pub(crate) const SOLDIER_INSTANCE_BASE: u32 = PROP_INSTANCE_INDEX + 1;
/// 士兵实例槽容量。取 `MAX_AI`(768) 的上限：压力模式红蓝各 128，加上普通波次也够。
/// **超出的 NPC 直接不画真网格**（退回 18 段箱体），不是静默越界：
/// `set_npc_visuals` 里 `len() < MAX_SOLDIER_INSTANCES` 就不再 push，`upload_soldiers`
/// 上传时再按容量取一次 min。（2026-09-22 复查：这里原先引用的 `write_soldier_instances`
/// 已不存在，是过期名字，已改正。）
pub(crate) const MAX_SOLDIER_INSTANCES: u32 = 768;
/// 士兵网格的顶点/索引预留容量（`soldier.glb` 实测 1082 顶点 / 540 索引，留 4 倍余量）。
/// ⚠️ 换更细的士兵模型要同步放大，否则 `set_soldier_mesh` 会拒绝上传并打 error。
pub(crate) const SOLDIER_MESH_VERTS: u32 = 4096;
pub(crate) const SOLDIER_MESH_INDICES: u32 = 8192;
/// 实例 storage buffer 的总元素数。**唯一权威定义**——历史上它是三份互相抄写的副本
/// （`buffer_elems` + 主管线 descriptor `.range()` + 阴影 pass descriptor `.range()`），
/// 加一个槽位只要漏改任一份，shader 就会对那一槽越界读 storage buffer：驱动不会报错，
/// 只会返回全零，于是 `inst.model` 变成零矩阵、所有顶点塌到一点、几何**完全不显示**且
/// 没有任何日志或 VUID 提示（2026-09-04 加道具槽时正好踩中，靠"红屏探针 + 换槽对照"才定位）。
/// 现在由最高槽位反推，结构上不可能再漏。
pub(crate) const INSTANCE_BUFFER_ELEMS: u64 =
    SOLDIER_INSTANCE_BASE as u64 + MAX_SOLDIER_INSTANCES as u64;
/// 并行剔除的**段数上限** = `cull_and_upload` 里两张栈上前缀和表 `[u32; N]` 的长度。
///
/// 🔴 2026-09-22 复查补：段数原来是裸的 `pool.workers() + 1`，只靠一句
/// `debug_assert!(nw <= 64)` 兜着 —— 而 **release 里 `debug_assert` 根本不存在**
/// （本仓 `[profile.release]` 用默认：`debug-assertions = false`、`overflow-checks = false`）。
/// 64 个 worker 以上的机器（64C/128T 起）会**每帧**在 `near_prefix[w]` 上
/// `index out of bounds: the len is 64 but the index is 64` —— 高核数机器一启动就崩。
/// 现在由 [`cull_segment_count`] 硬夹上限；本机的 `workers + 1` 远小于 64 ⇒ **行为零变化**。
pub(crate) const CULL_MAX_SEGMENTS: usize = 64;
pub(crate) const PROP_BIN_CELL_M: f32 = 10.0;
// 🔴 槽位布局断言改成**写死具体数字**。
// 原来这里写的是 `INSTANCE_BUFFER_ELEMS > GUN_INSTANCE_INDEX`，而它永远成立
// （本常量就是由 `SOLDIER_INSTANCE_BASE + MAX_SOLDIER_INSTANCES` 定义出来的，而
//  `SOLDIER_INSTANCE_BASE` 又是 `GUN_INSTANCE_INDEX + 2`）⇒ 按教训 14「永远成立的断言等于没写」。
// 现在任何槽位/容量改动都会**编译失败**，改动者必须回来同步 `build.rs` 的槽位常量
// 与三处 `.range()`（建 buffer / 主管线 / 阴影 pass）—— 那正是 2026-09-04 静默越界
// （几何整体消失、无 VUID）的入口。
/// 实例数据（model 4x4 + tint vec4，std430 步长 80 字节）
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct InstanceData {
    pub(crate) model: [f32; 16],
    pub(crate) tint: [f32; 4],
}
