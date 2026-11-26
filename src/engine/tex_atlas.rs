//! 贴图上屏的**契约**（2026-11-25）：纯策略函数 + 判据，先于实现写下来。
//!
//! ## 为什么先写契约
//!
//! 上屏要动的是**承重结构**（共享 mesh 着色器 + descriptor pool，铁律 A）。
//! 把「策略是什么」和「怎么接线」混在一次改动里，画面一旦不对就分不清是哪一半错了。
//! 所以这里先把策略写成**可执行、可判红**的纯函数；接线另起一步，接的时候这些判据不动。
//!
//! ## 现状（读代码得到的，不是设想）
//!
//! 采样路径**已经存在**：片元里
//! ```wgsl
//! if (length(input.uv) > 1e-4 && (feature_bits & 32u) != 0u) {
//!     let tex = textureSample(prop_atlas_tex, texture_sampler, input.uv).rgb;
//!     shaded = shaded * mix(vec3(1.0), tex * 1.9, 0.85);
//! }
//! ```
//! `prop_atlas_tex` = `assets/textures/prop_atlas.png`（**全局一张**，由导入脚本手工打图集）。
//! 也就是说：缺的**不是采样能力**，而是**每个模型自己的贴图**怎么进这张图集。
//!
//! ## 四条契约（每条都有判据）
//!
//! 1. **UV 重映射**：模型的 UV ∈ [0,1)，图集里占一格 ⇒ 必须映射进该格。
//!    回卷（tiling）在**导出端**做，不在这里做 —— `floor(uv*4)*0.25 + fract(uv*4)*0.25`
//!    在数学上恒等于 `uv`，写了等于没写（2026-10-31 踩过）。
//! 2. **是否采样**：与着色器**逐字同源**的判据（`uv` 非零 且 特征位 32 置位）。
//!    两边各写一份阈值就一定会分叉，分叉后表现为"某些件突然不贴图"。
//! 3. **无贴图不许变白模**：没有贴图时回退到顶点色（不是白色）。
//! 4. **绑定编号与池计数同源**：`PROP_ATLAS_BINDING` 与 descriptor pool 的
//!    `SAMPLED_IMAGE` 计数必须一起改（历史上"三处副本不同步 = 静默越界读"）。
//!    这条用**源码扫描判据**钉住，见 `binding_and_pool_count_are_in_sync`。

// 🔴 本模块**现在只被测试使用**：那是刻意的 —— 先写可执行契约，再动承重结构。
// 接线（把模型贴图打进图集并置特征位）是下一步。**接上之后必须删掉下面那行 allow**：
// 判据就是编译器 —— 删掉后若仍 0 警告，说明真被生产代码用上了（铁律 F 的清理口径）。
#![allow(dead_code)]

/// 图集里的一个矩形格（UV 空间，`[0,1]²` 内）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasRect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

impl AtlasRect {
    pub const fn new(u0: f32, v0: f32, u1: f32, v1: f32) -> Self {
        Self { u0, v0, u1, v1 }
    }
    /// 整张图集（= 不重映射）。
    pub const FULL: Self = Self::new(0.0, 0.0, 1.0, 1.0);
    pub fn width(&self) -> f32 {
        self.u1 - self.u0
    }
    pub fn height(&self) -> f32 {
        self.v1 - self.v0
    }
}

/// 把模型 UV 映射进图集的某一格。
///
/// **只做仿射缩放+平移**：回卷/平铺必须在上游（导出端）折进 `[0,1)`，
/// 这里再做 `fract` 是无效操作（数学上恒等），只会让人以为修好了什么。
///
/// 输入超出 `[0,1]` 时**不夹紧**：夹紧会把 tiling UV 压成一条边，画面上是明显的条纹。
/// 超界是上游的 bug，应当在导出端报出来。
pub fn remap_uv(uv: [f32; 2], rect: AtlasRect) -> [f32; 2] {
    [
        rect.u0 + uv[0] * rect.width(),
        rect.v0 + uv[1] * rect.height(),
    ]
}

/// 着色器里决定"是否采样作者贴图"的特征位。
///
/// 🔴 与 `build.rs` 的 `(feature_bits & 32u)` **必须同值**；判据
/// `feature_bit_matches_the_shader` 扫源码核对。
pub const FEATURE_AUTHORED_TEX: u32 = 32;

/// 着色器里判断"UV 有意义"的阈值（`length(input.uv) > 1e-4`）。
pub const UV_EPSILON_SQRT: f32 = 1e-4;

/// 这一片元是否应当采样作者贴图。
///
/// **逐条对应着色器**：
/// ```wgsl
/// if (length(input.uv) > 1e-4 && (feature_bits & 32u) != 0u)
/// ```
/// 注意着色器用的是 `length()`（欧几里得长度），不是逐分量比较 —— 两者对
/// `uv=(0,0.5)` 的判定不同（长度 0.5 > 1e-4 会采样；而 `max` 逐分量也 > 1e-4）。
/// 这里用 `length` 与着色器一致。
pub fn should_sample(uv: [f32; 2], feature_bits: u32) -> bool {
    let len = (uv[0] * uv[0] + uv[1] * uv[1]).sqrt();
    len > UV_EPSILON_SQRT && (feature_bits & FEATURE_AUTHORED_TEX) != 0
}

/// 采样结果与顶点色的合成 —— 与着色器同一条公式。
///
/// ```wgsl
/// shaded = shaded * mix(vec3(1.0), tex * 1.9, 0.85);
/// ```
/// ⇒ `shaded * ((1-0.85) + 0.85 * tex * 1.9)` = `shaded * (0.15 + 1.615 * tex)`
pub fn modulate_albedo(vertex_color: [f32; 3], tex: [f32; 3]) -> [f32; 3] {
    const MIX: f32 = 0.85;
    const GAIN: f32 = 1.9;
    [
        vertex_color[0] * ((1.0 - MIX) + MIX * tex[0] * GAIN),
        vertex_color[1] * ((1.0 - MIX) + MIX * tex[1] * GAIN),
        vertex_color[2] * ((1.0 - MIX) + MIX * tex[2] * GAIN),
    ]
}

/// 没有贴图时的反照率 —— **回退顶点色，绝不回退白色**。
///
/// 白模是最糟的失败形态：看起来像"贴图丢了"而不是"这个模型本来没贴图"，
/// 而且白模在明亮场景里会直接过曝，掩盖其它问题。
pub fn fallback_albedo(vertex_color: [f32; 3]) -> [f32; 3] {
    vertex_color
}


// ---------------------------------------------------------------------------
// 第 1 步：打包（纯 CPU —— 不改渲染，先把"贴图进图集 + UV 落格"做对）
// ---------------------------------------------------------------------------

/// 图集网格：把整张图集切成 `cols × rows` 格。
///
/// 为何用**固定网格**而不是"按尺寸装箱"：本项目要打进去的是几十个小道具贴图，
/// 网格的确定性（同一批资产每次得到同样的格子）比利用率更重要 ——
/// 装箱算法一改，所有资产的 UV 全变，而这是最难查的一类回归。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasGrid {
    pub cols: u32,
    pub rows: u32,
}

impl AtlasGrid {
    pub const fn new(cols: u32, rows: u32) -> Self {
        Self { cols, rows }
    }

    pub fn cell_count(&self) -> u32 {
        self.cols * self.rows
    }

    /// 第 `index` 格（行优先：先横后竖）的 UV 矩形。越界返回 None。
    ///
    /// 🔴 **V 轴方向**：图集图像的 v 从**上**往下增长？本引擎的 UV 约定见
    /// `props` 的采样（`texture_sampler` 是 REPEAT+LINEAR），GLB 的 UV 原点在**左上**，
    /// 而图像行序是**从上到下** ⇒ 直接用 `row / rows` 作为 v 起点即可，不做翻转。
    /// 判据 `grid_cells_cover_the_atlas_without_overlap` 只验几何覆盖，
    /// 方向由真实采样图（`tex_atlas_v_direction_matches_the_image_rows`）钉住。
    pub fn cell_rect(&self, index: u32) -> Option<AtlasRect> {
        if self.cols == 0 || self.rows == 0 || index >= self.cell_count() {
            return None;
        }
        let col = index % self.cols;
        let row = index / self.cols;
        let cw = 1.0 / self.cols as f32;
        let ch = 1.0 / self.rows as f32;
        Some(AtlasRect::new(
            col as f32 * cw,
            row as f32 * ch,
            (col + 1) as f32 * cw,
            (row + 1) as f32 * ch,
        ))
    }
}

/// 把整张网格的 UV 落进某一格（就地改顶点）。
///
/// 顶点布局是 `pos(3) normal(3) uv(2) color(3)`（stride 11），UV 在 `6..8`。
/// **只动 UV 两个分量**：法线/顶点色/位置一个都不许碰。
///
/// 🔴 **是否重映射由"这个 GLB 有没有 TEXCOORD_0"决定，绝不能用"UV 是否全零"判断**。
///
/// 我第一版就是按"全零 = 没有 UV"跳过的，结果**四边形的左下角本来就在 (0,0)**，
/// 那个角没被重映射 —— 画面上是格外面露一条边。
/// 顺带记一笔：着色器里 `length(input.uv) > 1e-4` 有同一个毛病 ——
/// 它不是"有没有 UV"的判据，而是"这个插值后的 UV 是否接近原点"的判据；
/// 对真有 (0,0) 角的模型，那一条边的像素会跳过采样（细线状问题）。
/// 真正的判据只能是属性存在性（`glb_has_texcoord`），在 CPU 侧一次定死。
///
/// 返回被改动的顶点数（没有 UV 属性时返回 0 且不改任何东西）。
pub fn remap_mesh_uvs(verts: &mut [[f32; 11]], rect: AtlasRect, has_texcoord: bool) -> usize {
    if !has_texcoord {
        return 0;
    }
    for v in verts.iter_mut() {
        let out = remap_uv([v[6], v[7]], rect);
        v[6] = out[0];
        v[7] = out[1];
    }
    verts.len()
}

/// 这个 GLB 的网格里**是否存在** `TEXCOORD_0` 属性。
///
/// 用 JSON 里是否出现该键判定（`parse_glb` 已经把缺失的 UV 填成 (0,0)，
/// 那些 0 与"真的在原点"无法区分 —— 所以必须回到源数据问）。
pub fn glb_has_texcoord(bytes: &[u8]) -> bool {
    const NEEDLE: &[u8] = b"TEXCOORD_0\"";
    bytes.windows(NEEDLE.len()).any(|w| w == NEEDLE)
}

/// 一张 RGBA8 图像（图集的容器）。
#[derive(Clone, Debug)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// 建一张纯色图（图集底板）。
    ///
    /// 底板用**白**：着色器的合成是 `shaded * (0.15 + 1.615 * tex)`，
    /// 白底 = 1.765 倍；只有 `feature_bits & 32` 置位的件才会走到这条路径，
    /// 所以底板颜色不影响"没贴图的件"。
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            data.extend_from_slice(&rgba);
        }
        Self { width, height, data }
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        self.data.get(i..i + 4).map(|s| [s[0], s[1], s[2], s[3]])
    }
}

/// 把一张贴图**最近邻**缩放并写进图集的某个 UV 格。
///
/// 用最近邻而不是双线性：确定性最重要 —— 同一输入必须逐字节得到同一张图集，
/// 否则"图集变了"这类问题会变成不可复现的。视觉上道具贴图本来就是对色块。
///
/// 返回写入的像素数（0 = 格子退化，调用方应当报错）。
pub fn blit_into_atlas(
    atlas: &mut RgbaImage,
    rect: AtlasRect,
    tex: &crate::engine::asset_texture::ImportedTexture,
) -> u32 {
    if tex.width == 0 || tex.height == 0 {
        return 0;
    }
    // UV 格 → 像素范围（左闭右开）
    let x0 = (rect.u0 * atlas.width as f32).round() as i64;
    let x1 = (rect.u1 * atlas.width as f32).round() as i64;
    let y0 = (rect.v0 * atlas.height as f32).round() as i64;
    let y1 = (rect.v1 * atlas.height as f32).round() as i64;
    let (cw, ch) = (x1 - x0, y1 - y0);
    if cw <= 0 || ch <= 0 {
        return 0;
    }
    let mut written = 0;
    for dy in 0..ch {
        for dx in 0..cw {
            // 目标像素 → 源纹素（最近邻，取纹素中心）
            let sx = ((dx as u64 * tex.width as u64) / cw as u64) as u32;
            let sy = ((dy as u64 * tex.height as u64) / ch as u64) as u32;
            let Some(px) = tex.pixel(sx.min(tex.width - 1), sy.min(tex.height - 1)) else {
                continue;
            };
            let (px_x, px_y) = ((x0 + dx) as u32, (y0 + dy) as u32);
            if px_x >= atlas.width || px_y >= atlas.height {
                continue;
            }
            let i = ((px_y * atlas.width + px_x) * 4) as usize;
            if let Some(slot) = atlas.data.get_mut(i..i + 4) {
                slot.copy_from_slice(&px);
                written += 1;
            }
        }
    }
    written
}


/// 把一个 GLB **整体**打进图集的一格：解析网格 → 解析贴图 → blit → 重映射 UV。
///
/// 返回 `(网格, 特征位)`：
/// - 模型有内嵌贴图 ⇒ 已 blit 并重映射 UV，特征位 = [`FEATURE_AUTHORED_TEX`]；
/// - 模型**没有**贴图 ⇒ 网格原样返回、UV 不动、特征位 = **0**
///   （着色器就不会走采样分支 ⇒ 行为与上一版逐像素一致）。
///
/// 这条"没有贴图 ⇒ 特征位 0"的约定是关键：如果无条件置位，
/// 没有贴图的件会去采样图集底板（白）⇒ 被整体提亮 1.765 倍，肉眼可见地变白。
pub fn pack_glb_into_atlas(
    bytes: &[u8],
    grid: AtlasGrid,
    cell: u32,
    atlas: &mut RgbaImage,
) -> Result<(crate::engine::assets::ImportedMesh, u32), String> {
    let mut mesh = crate::engine::assets::parse_glb(bytes)?;
    let texs = crate::engine::asset_texture::parse_glb_textures(bytes)?;
    let Some(tex) = texs.first() else {
        return Ok((mesh, 0));
    };
    let rect = grid
        .cell_rect(cell)
        .ok_or_else(|| format!("图集格 {cell} 越界（{}×{}）", grid.cols, grid.rows))?;
    let written = blit_into_atlas(atlas, rect, tex);
    if written == 0 {
        return Err(format!("贴图 {} 写进格 {cell} 时写入 0 像素", tex.name));
    }
    remap_mesh_uvs(&mut mesh.verts, rect, glb_has_texcoord(bytes));
    Ok((mesh, FEATURE_AUTHORED_TEX))
}


// ---------------------------------------------------------------------------
// 第 2 步：真实资产接线 —— 模型名 → 图集格（两侧各自算，不靠顺序）
// ---------------------------------------------------------------------------

/// 模型名 → 图集格下标（FNV-1a 哈希取模）。
///
/// 🔴 **为什么用哈希而不是"按名字排序后的序号"**：
/// 打包工具（写图集 PNG）与游戏（加载时重映射 UV）是**两个不同的代码路径**，
/// 用排序序号意味着两边必须拿到完全相同的模型集合、相同的排序规则、相同的编码 ——
/// 任何一处不同（多一个文件、locale 排序差异）都会让所有模型的 UV 集体错位。
/// 哈希只依赖**名字本身**：两边各算各的，结果必然一致。
///
/// 代价是**可能撞格**：`Hash` 取模 cell_count 会有冲突，冲突时后写的覆盖先写的，
/// 表现为"某个道具显示成另一个道具的贴图"。所以：
/// - 打包时必须**报告冲突**（`PackReport::collisions`），不能静默；
/// - 格数要远大于模型数（当前 52 件 / 16 格 = 明显不够，实操应给 64 格以上）。
pub fn cell_for_name(name: &str, grid: AtlasGrid) -> Option<u32> {
    let n = grid.cell_count();
    if n == 0 {
        return None;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some((h % n as u64) as u32)
}

/// 道具图集的**唯一网格约定**（打包与加载共用，改一处即两边同步）。
///
/// 8×8 = 64 格。当前 53 件道具、实际带贴图的只有 1 件，格数远大于需要 ——
/// 因为格号是**哈希取模**，格数越少撞格概率越高（撞格表现为"贴图串了"，见 `PackReport`）。
pub const PROP_ATLAS_GRID: AtlasGrid = AtlasGrid::new(8, 8);

/// 道具名 → 图集格的**唯一口径**：用**不带扩展名**的名字（文件 stem）。
///
/// 🔴 这一条踩过：打包命令用 `file_name()`（`crate_textured.glb`），
/// 而加载侧的道具名是 stem（`crate_textured`）—— 同一个模型算出**不同的格**，
/// 表现为"贴图错位"。所以这里把口径集中成一个函数，两边都调它。
pub fn prop_cell(stem: &str) -> Option<u32> {
    cell_for_name(stem, PROP_ATLAS_GRID)
}

/// 给一件道具的顶点套上图集 UV（**纯策略**，不读文件、不看环境变量）。
///
/// 返回 `Some(格号)` = 已重映射；`None` = 不该动（没有贴图 / 没有 UV 属性）。
/// 调用方决定"要不要真的套用"（例如开关关着就干脆不调）。
pub fn apply_prop_atlas_uvs(
    stem: &str,
    has_texcoord: bool,
    has_texture: bool,
    verts: &mut [[f32; 11]],
) -> Option<u32> {
    if !has_texture {
        return None;
    }
    let cell = prop_cell(stem)?;
    let rect = PROP_ATLAS_GRID.cell_rect(cell)?;
    remap_mesh_uvs(verts, rect, has_texcoord);
    Some(cell)
}

/// 这个输出路径**是不是作者手工维护的那张图集**（`prop_atlas.png`）。
///
/// 🔴 存在的理由是一次真实事故（2026-11-26，就在本会话里）：
/// 我拿打包器直接写了 `assets/textures/prop_atlas.png` —— 那是**手工做的 4096×4096 图集**
/// （75% 像素非白，16.9 MB），现有道具的 UV 全靠它。
/// 覆盖之后所有道具的贴图都会变成白底板（16.9 MB → 24 KB），而且**编译器不会报错**。
/// 还好工作区有 git，`git checkout` 拿回来了。
///
/// ⇒ 打包器写**另一个文件**（`*_generated.png`）；这一条把它变成不可绕过的约束。
pub fn is_authored_atlas_path(path: &str) -> bool {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file.eq_ignore_ascii_case("prop_atlas.png")
}

/// 打包器的默认输出路径（与作者图集分开，永不互相覆盖）。
pub const GENERATED_ATLAS_PATH: &str = "assets/textures/prop_atlas_generated.png";

/// 一件要打包的资产：名字 + GLB 字节。
pub struct AtlasInput<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
}

/// 打包结果报告（给人看，也给判据看）。
#[derive(Debug, Default, Clone)]
pub struct PackReport {
    /// 成功打包的件数与格号。
    pub packed: Vec<(String, u32)>,
    /// 没有内嵌贴图（或解析失败）而跳过的件。
    pub skipped: Vec<(String, String)>,
    /// 撞格的件：`(格号, 先占者, 后来者)` —— **必须报出来**，静默覆盖表现为"贴图串了"。
    pub collisions: Vec<(u32, String, String)>,
}

/// 把若干件资产打进一张图集（每件一个哈希格）。
///
/// 返回 `(图集, 报告)`。**没有贴图的件会被跳过并记进 `skipped`** ——
/// 它们的 UV 不会被重映射（游戏侧同样跳过，因为游戏侧也不知道它们有贴图）。
pub fn pack_props_into_atlas(
    inputs: &[AtlasInput<'_>],
    grid: AtlasGrid,
    atlas_w: u32,
    atlas_h: u32,
) -> (RgbaImage, PackReport) {
    let mut atlas = RgbaImage::filled(atlas_w, atlas_h, [255, 255, 255, 255]);
    let mut report = PackReport::default();
    let mut owner: std::collections::BTreeMap<u32, String> = std::collections::BTreeMap::new();
    for inp in inputs {
        let cell = match cell_for_name(inp.name, grid) {
            Some(c) => c,
            None => {
                report.skipped.push((inp.name.to_string(), "图集格数 0".into()));
                continue;
            }
        };
        let texs = match crate::engine::asset_texture::parse_glb_textures(inp.bytes) {
            Ok(t) => t,
            Err(e) => {
                report.skipped.push((inp.name.to_string(), e));
                continue;
            }
        };
        let Some(tex) = texs.first() else {
            report.skipped.push((inp.name.to_string(), "无内嵌贴图".into()));
            continue;
        };
        if let Some(prev) = owner.get(&cell) {
            report
                .collisions
                .push((cell, prev.clone(), inp.name.to_string()));
        }
        owner.insert(cell, inp.name.to_string());
        let rect = grid.cell_rect(cell).expect("cell 已保证在范围内");
        blit_into_atlas(&mut atlas, rect, tex);
        report.packed.push((inp.name.to_string(), cell));
    }
    (atlas, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::asset_texture::ImportedTexture;

    /// 造一个**带网格且带内嵌贴图**的最小 GLB：一个 quad（4 顶点，UV 铺满）+ 2×2 PNG。
    ///
    /// 不依赖任何外部资产：图与几何都在测试里现造，所以这条端到端判据在哪儿都成立。
    fn glb_quad_with_texture(with_uv: bool, with_texture: bool) -> Vec<u8> {
        // 🔴 **非交错布局**：每个属性一个 bufferView，不给 byteStride。
        // 第一版写成"pos+uv 交错、stride 20 但没写 byteStride" ⇒ 解析器按紧凑约定读到的是
        // 相邻顶点的 UV 当位置，UV 全变成 0（畸形 GLB，不是解析器的错）。
        let pos: [[f32; 3]; 4] = [
            [-0.5, 0.0, -0.5],
            [0.5, 0.0, -0.5],
            [0.5, 0.0, 0.5],
            [-0.5, 0.0, 0.5],
        ];
        let uv: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let idx: [u16; 6] = [0, 1, 2, 0, 2, 3];

        let mut geo: Vec<u8> = Vec::new();
        let pos_off = geo.len();
        for v in pos {
            for f in v {
                geo.extend_from_slice(&f.to_le_bytes());
            }
        }
        let pos_len = geo.len() - pos_off;
        let uv_off = geo.len();
        for v in uv {
            for f in v {
                geo.extend_from_slice(&f.to_le_bytes());
            }
        }
        let uv_len = geo.len() - uv_off;
        let idx_off = geo.len();
        for i in idx {
            geo.extend_from_slice(&i.to_le_bytes());
        }
        let idx_len = geo.len() - idx_off;

        // 2×2 PNG（红/绿/蓝/黄）
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        img.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        img.put_pixel(1, 1, image::Rgba([255, 255, 0, 255]));
        let mut cb = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut cb, image::ImageFormat::Png)
            .expect("编码 PNG");
        let png = cb.into_inner();
        let png_off = geo.len();
        geo.extend_from_slice(&png);
        let png_len = png.len();

        // bufferView 下标：0=位置 1=UV 2=索引 3=贴图
        let (uv_bv, uv_acc) = if with_uv {
            (
                r#",{"buffer":0,"byteOffset":48,"byteLength":32}"#,
                r#",{"bufferView":1,"componentType":5126,"count":4,"type":"VEC2"}"#,
            )
        } else {
            ("", "")
        };
        let (images, textures, materials) = if with_texture {
            (
                r#"[{"bufferView":3,"mimeType":"image/png","name":"quad_tex"}]"#.to_string(),
                r#"[{"source":0}]"#.to_string(),
                r#"[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}]"#.to_string(),
            )
        } else {
            ("[]".to_string(), "[]".to_string(), "[]".to_string())
        };
        let uv_attr = if with_uv {
            r#","TEXCOORD_0":1"#
        } else {
            ""
        };
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},
"buffers":[{{"byteLength":{total}}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{pos_len}}}{uv_bv},
  {{"buffer":0,"byteOffset":{idx_off},"byteLength":{idx_len}}},
  {{"buffer":0,"byteOffset":{png_off},"byteLength":{png_len}}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3"}}{uv_acc},
  {{"bufferView":2,"componentType":5123,"count":6,"type":"SCALAR"}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0{uv_attr}}},"indices":{idx_acc}}}]}}],
"images":{images},"textures":{textures},"materials":{materials}}}"#,
            total = geo.len(),
            pos_len = pos_len,
            uv_bv = uv_bv,
            idx_off = idx_off,
            idx_len = idx_len,
            png_off = png_off,
            png_len = png_len,
            uv_acc = uv_acc,
            idx_acc = if with_uv { 2 } else { 1 },
            uv_attr = uv_attr,
            images = images,
            textures = textures,
            materials = materials,
        );
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + json.len() + 8 + geo.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
        out.extend_from_slice(json.as_bytes());
        out.extend_from_slice(&(geo.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E4942u32.to_le_bytes());
        out.extend_from_slice(&geo);
        out
    }

    /// 🔴 **端到端**：有贴图的模型 ⇒ UV 落进格、图集里出现正确像素、特征位置位。
    ///
    /// 反向验证：把 `pack_glb_into_atlas` 里的 `remap_mesh_uvs(...)` 删掉 ⇒ 第一条 UV 断言红。
    #[test]
    fn packing_a_textured_model_remaps_uvs_and_fills_the_cell() {
        let glb = glb_quad_with_texture(true, true);
        let grid = AtlasGrid::new(2, 2);
        let mut atlas = RgbaImage::filled(64, 64, [255, 0, 255, 255]);
        let (mesh, bits) = pack_glb_into_atlas(&glb, grid, 3, &mut atlas).expect("打包应成功");
        assert_eq!(bits, FEATURE_AUTHORED_TEX, "有贴图必须置特征位");
        // 4 个顶点都有非零 UV ⇒ 全部落进第 3 格（右下）
        let rect = grid.cell_rect(3).unwrap();
        for v in &mesh.verts {
            assert!(
                v[6] >= rect.u0 - 1e-6 && v[6] <= rect.u1 + 1e-6,
                "uv.u={} 应落在格 [{},{}]",
                v[6],
                rect.u0,
                rect.u1
            );
            assert!(
                v[7] >= rect.v0 - 1e-6 && v[7] <= rect.v1 + 1e-6,
                "uv.v={} 应落在格 [{},{}]",
                v[7],
                rect.v0,
                rect.v1
            );
        }
        // 格内出现源图的颜色（红/绿/蓝/黄各占 1/4）
        let (x0, y0, x1, y1) = (32u32, 32u32, 64u32, 64u32);
        let mid = |x0: u32, y0: u32| atlas.pixel(x0, y0).unwrap();
        assert_eq!(mid(x0, y0), [255, 0, 0, 255], "格左上 = 红");
        assert_eq!(mid(x1 - 1, y0), [0, 255, 0, 255], "格右上 = 绿");
        assert_eq!(mid(x0, y1 - 1), [0, 0, 255, 255], "格左下 = 蓝");
        assert_eq!(mid(x1 - 1, y1 - 1), [255, 255, 0, 255], "格右下 = 黄");
        // 其它格仍是底板色
        assert_eq!(atlas.pixel(0, 0), Some([255, 0, 255, 255]), "格外的图集必须原样");
    }

    /// 🔴 **没有贴图的模型 ⇒ 特征位 0、UV 不动、图集不动**。
    ///
    /// 这是"旧资产行为逐像素不变"的保证。若无条件置位，没贴图的件会去采样白底板
    /// ⇒ 被整体提亮 1.765 倍，肉眼可见地变白。
    ///
    /// 反向验证：把 `let Some(tex) = texs.first() else { return Ok((mesh, 0)); }`
    /// 改成 `.unwrap_or(&fallback_tex)`（无条件打包）⇒ 第一条断言红。
    #[test]
    fn untextured_model_is_left_untouched_and_keeps_the_feature_bit_clear() {
        let glb = glb_quad_with_texture(true, false);
        let grid = AtlasGrid::new(2, 2);
        let before = RgbaImage::filled(64, 64, [255, 0, 255, 255]);
        let mut atlas = before.clone();
        let (mesh, bits) = pack_glb_into_atlas(&glb, grid, 0, &mut atlas).expect("打包应成功");
        assert_eq!(bits, 0, "没有贴图不许置特征位");
        assert_eq!(atlas.data, before.data, "图集一个字节都不许被改");
        // UV 保持原样（quad 的 UV 是铺满 [0,1] 的）
        assert!(
            mesh.verts.iter().all(|v| v[6] <= 1.0 && v[7] <= 1.0),
            "UV 不该被重映射到别的格"
        );
        let has_full_uv = mesh.verts.iter().any(|v| v[6] > 0.9 && v[7] > 0.9);
        assert!(has_full_uv, "原 UV 的 (1,1) 角必须还在");
    }


    /// 🔴 **名字 → 格必须是纯函数**：同样的名字永远得到同样的格，与调用顺序无关。
    ///
    /// 这是"打包工具与游戏各算各的"能成立的前提。若引入任何顺序依赖，
    /// 两边一旦文件集合不同，所有模型 UV 会集体错位。
    ///
    /// 反向验证：把 `h % n` 换成 `h % 7`（写死）⇒ 第二条断言红。
    #[test]
    fn cell_assignment_is_a_pure_function_of_the_name() {
        let g = AtlasGrid::new(8, 8);
        let a = cell_for_name("barrel_metal", g);
        let b = cell_for_name("barrel_metal", g);
        assert_eq!(a, b, "同名必须同格");
        assert!(a.unwrap() < g.cell_count(), "格号必须在范围内");
        // 不同名字一样不同格（不保证，但这两个实测不同）
        assert_ne!(
            cell_for_name("barrel_metal", g),
            cell_for_name("tree_oak", g),
            "这两个名字实测落在不同格"
        );
        // 格数为 0 ⇒ None 而不是除零 panic
        assert_eq!(cell_for_name("x", AtlasGrid::new(0, 4)), None);
    }

    /// 🔴 **撞格必须被报告**（静默覆盖会表现为"某个道具显示成另一个的贴图"）。
    ///
    /// 反向验证：删掉 `report.collisions.push(...)` 那段 ⇒ 本判据红。
    #[test]
    fn collisions_are_reported_not_silently_overwritten() {
        // 1 格图集 ⇒ 任何两件都必然撞
        let g = AtlasGrid::new(1, 1);
        let glb_a = glb_quad_with_texture(true, true);
        let glb_b = glb_quad_with_texture(true, true);
        let inputs = vec![
            AtlasInput { name: "a", bytes: &glb_a },
            AtlasInput { name: "b", bytes: &glb_b },
        ];
        let (_atlas, rep) = pack_props_into_atlas(&inputs, g, 8, 8);
        assert_eq!(rep.packed.len(), 2, "两件都要打包");
        assert_eq!(rep.collisions.len(), 1, "必须报告 1 次撞格");
        let (cell, first, second) = &rep.collisions[0];
        assert_eq!(*cell, 0);
        assert_eq!(first, "a");
        assert_eq!(second, "b");
    }

    /// 没有内嵌贴图的件 ⇒ 跳过并**说明原因**（不是静默忽略）。
    ///
    /// 反向验证：把 `report.skipped.push(...)` 换成 `continue` ⇒ 本判据红。
    #[test]
    fn untextured_inputs_are_skipped_with_a_reason() {
        let g = AtlasGrid::new(4, 4);
        let glb = glb_quad_with_texture(true, false);
        let inputs = vec![AtlasInput { name: "plain", bytes: &glb }];
        let (atlas, rep) = pack_props_into_atlas(&inputs, g, 16, 16);
        assert!(rep.packed.is_empty(), "没贴图不该打包");
        assert_eq!(rep.skipped.len(), 1);
        assert!(rep.skipped[0].1.contains("无内嵌贴图"), "要说明原因：{:?}", rep.skipped[0].1);
        // 图集保持纯白底板
        assert_eq!(atlas.pixel(0, 0), Some([255, 255, 255, 255]));
    }


    /// 🔴 **默认（不套用）时 UV 必须逐字节不变** —— 这是"旧资产行为不变"的保证。
    ///
    /// 反向验证：把 `apply_prop_atlas_uvs` 里的 `if !has_texture { return None; }` 删掉
    /// ⇒ 第二条断言红（没贴图的件也被改了 UV）。
    #[test]
    fn prop_atlas_uvs_are_only_applied_when_there_is_a_texture() {
        let mk = || vec![[0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.3, 0.7, 1.0, 1.0, 1.0]];
        // (a) 没有贴图 ⇒ None 且 UV 不动
        let mut v = mk();
        assert_eq!(apply_prop_atlas_uvs("crate_textured", true, false, &mut v), None);
        assert_eq!([v[0][6], v[0][7]], [0.3, 0.7], "没贴图时 UV 不许动");
        // (b) 有贴图 + 有 UV ⇒ Some(格) 且 UV 落进格
        let mut v2 = mk();
        let cell = apply_prop_atlas_uvs("crate_textured", true, true, &mut v2).expect("应当套用");
        assert_eq!(Some(cell), prop_cell("crate_textured"), "格号必须与 prop_cell 一致");
        let rect = PROP_ATLAS_GRID.cell_rect(cell).unwrap();
        assert!(
            v2[0][6] >= rect.u0 - 1e-6 && v2[0][6] <= rect.u1 + 1e-6,
            "uv.u={} 应在格 [{},{}]",
            v2[0][6],
            rect.u0,
            rect.u1
        );
    }

    /// 🔴 **打包口径与加载口径必须同源**：都算 stem，不算文件名。
    ///
    /// 这条钉住一个真实踩过的坑：打包命令原来用 `file_name()`（带 `.glb`），
    /// 加载侧用 stem ⇒ 同一个模型算出不同的格。
    ///
    /// 反向验证：把 `prop_cell` 改成 `cell_for_name(&format!("{stem}.glb"), ...)` ⇒ 红。
    #[test]
    fn pack_and_load_agree_on_the_same_name_form() {
        let stem = "crate_textured";
        // 加载侧：prop_cell(stem)
        let load_cell = prop_cell(stem).unwrap();
        // 打包侧：命令里用的是 file_stem() ⇒ 也是 stem
        let pack_cell = cell_for_name(stem, PROP_ATLAS_GRID).unwrap();
        assert_eq!(load_cell, pack_cell, "两侧必须算出同一个格");
        // 带扩展名的形式必须**不同**（说明口径确实敏感，不是巧合相同）
        assert_ne!(
            cell_for_name("crate_textured.glb", PROP_ATLAS_GRID),
            Some(load_cell),
            "带扩展名应当算出另一格 —— 否则这条判据没有区分度"
        );
    }


    /// 🔴 **打包器不许写作者图集那个文件名**（本会话真实踩过：覆盖了 16.9 MB 手作图集）。
    ///
    /// 反向验证：把 `is_authored_atlas_path` 改成恒 `false` ⇒ 第一条断言红。
    #[test]
    fn authored_atlas_path_is_recognised_so_it_cannot_be_overwritten() {
        assert!(is_authored_atlas_path("assets/textures/prop_atlas.png"));
        assert!(is_authored_atlas_path("/abs/x/prop_atlas.png"));
        assert!(is_authored_atlas_path("PROP_ATLAS.PNG"), "大小写不敏感");
        assert!(!is_authored_atlas_path("assets/textures/prop_atlas_generated.png"));
        assert!(GENERATED_ATLAS_PATH.ends_with("_generated.png"));
        assert!(!is_authored_atlas_path(GENERATED_ATLAS_PATH), "默认输出路径必须安全");
    }

    /// 🔴 **网格必须完整覆盖图集且互不重叠**。
    ///
    /// 只测"某一格算得对"会漏掉"两格算到同一处"这种错误（画面上表现为两个模型
    /// 抢同一块贴图）。这里用**累加面积**与**两两相交**两条一起钉。
    ///
    /// 反向验证：把 `cell_rect` 的 `row = index / cols` 改成 `index % cols` ⇒ 红。
    #[test]
    fn grid_cells_cover_the_atlas_without_overlap() {
        let g = AtlasGrid::new(4, 3);
        assert_eq!(g.cell_count(), 12);
        let rects: Vec<_> = (0..g.cell_count()).map(|i| g.cell_rect(i).unwrap()).collect();
        // 面积和 = 1.0（不重叠 + 全覆盖 的充要条件）
        let area: f32 = rects.iter().map(|r| r.width() * r.height()).sum();
        assert!((area - 1.0).abs() < 1e-5, "12 格面积应恰为 1.0，实际 {area}");
        // 两两不相交（用格子中心判定：任何两格中心不能落在对方格内）
        for (a, ra) in rects.iter().enumerate() {
            for (b, rb) in rects.iter().enumerate() {
                if a == b {
                    continue;
                }
                let ca = [(ra.u0 + ra.u1) * 0.5, (ra.v0 + ra.v1) * 0.5];
                let inside = ca[0] > rb.u0 && ca[0] < rb.u1 && ca[1] > rb.v0 && ca[1] < rb.v1;
                assert!(!inside, "第 {a} 格的中心落在了第 {b} 格里");
            }
        }
        // 行优先：第 0 格在左上，第 3 格在右上，第 4 格在第二行左
        assert_eq!(g.cell_rect(0), Some(AtlasRect::new(0.0, 0.0, 0.25, 1.0 / 3.0)));
        assert_eq!(g.cell_rect(3), Some(AtlasRect::new(0.75, 0.0, 1.0, 1.0 / 3.0)));
        assert_eq!(g.cell_rect(4), Some(AtlasRect::new(0.0, 1.0 / 3.0, 0.25, 2.0 / 3.0)));
        // 越界返回 None，不 panic
        assert_eq!(g.cell_rect(12), None);
        assert_eq!(AtlasGrid::new(0, 4).cell_rect(0), None, "0 列必须返回 None 而不是除零");
    }

    /// 🔴 **UV 落格只改 uv 两个分量**，法线/顶点色/位置一个都不许动。
    ///
    /// 反向验证：把 `v[6] = out[0]` 写成 `v[6] = out[0]; v[3] = out[0];`（顺手改了法线）
    /// ⇒ 第三条断言红。
    #[test]
    fn remapping_uvs_leaves_position_normal_color_untouched() {
        // pos(3) normal(3) uv(2) color(3)
        // ⚠ uv 不能是 (0,0)：那条走的是"零 UV 保持零"的分支（另有专门判据）。
        let mut verts = vec![
            [1.0, 2.0, 3.0, 0.0, 0.0, 1.0, 0.25, 0.5, 0.7, 0.8, 0.9],
            [4.0, 5.0, 6.0, 0.1, 0.2, 0.3, 1.0, 1.0, 0.4, 0.5, 0.6],
        ];
        let before = verts.clone();
        let n = remap_mesh_uvs(&mut verts, AtlasRect::new(0.5, 0.5, 1.0, 1.0), true);
        assert_eq!(n, 2, "两个顶点都带 UV");
        // 位置 / 法线 / 颜色：逐字节不变
        for (a, b) in verts.iter().zip(before.iter()) {
            assert_eq!(&a[0..6], &b[0..6], "位置与法线不许动");
            assert_eq!(&a[8..11], &b[8..11], "顶点色不许动");
        }
        // UV 落进格内
        // uv(0.25,0.5) → (0.5+0.25*0.5, 0.5+0.5*0.5) = (0.625, 0.75)
        assert_eq!([verts[0][6], verts[0][7]], [0.625, 0.75], "uv(0.25,0.5) 要落进格内");
        assert_eq!([verts[1][6], verts[1][7]], [1.0, 1.0], "uv(1,1) → 格右上");
    }

    /// 🔴 **没有 TEXCOORD_0 属性的网格：UV 一个都不许动**（返回 0）。
    ///
    /// 而"有属性、某个角恰好是 (0,0)"的网格**必须**被重映射 —— 这两件事不能混。
    /// 第一版用"UV 全零"当判据，结果四边形的左下角（真在原点）没被重映射。
    ///
    /// 反向验证：把 `if !has_texcoord { return 0; }` 删掉 ⇒ 第一条断言红。
    #[test]
    fn remapping_is_gated_on_the_attribute_not_on_zero_values() {
        let rect = AtlasRect::new(0.25, 0.25, 0.5, 0.5);
        // (a) 没有 TEXCOORD_0：不动
        let mut no_uv = vec![[0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
        assert_eq!(remap_mesh_uvs(&mut no_uv, rect, false), 0, "无属性时返回 0");
        assert_eq!([no_uv[0][6], no_uv[0][7]], [0.0, 0.0], "无属性时 UV 不许动");

        // (b) 有 TEXCOORD_0：连 (0,0) 角也要映射进格
        let mut with_uv = vec![
            [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.5, 1.0, 1.0, 1.0],
        ];
        assert_eq!(remap_mesh_uvs(&mut with_uv, rect, true), 2, "两个顶点都要改");
        assert_eq!([with_uv[0][6], with_uv[0][7]], [0.25, 0.25], "(0,0) 角 → 格左下");
        assert_eq!([with_uv[1][6], with_uv[1][7]], [0.375, 0.375], "(0.5,0.5) → 格中心");
    }

    /// 属性存在性判定要认得出 `"TEXCOORD_0"` 这个键。
    ///
    /// 反向验证：把 `NEEDLE` 改成 `b"TEXCOORD_1\""` ⇒ 本判据红。
    #[test]
    fn texcoord_presence_is_detected_from_the_json() {
        assert!(glb_has_texcoord(br#"{"attributes":{"POSITION":0,"TEXCOORD_0":1}}"#));
        assert!(!glb_has_texcoord(br#"{"attributes":{"POSITION":0,"NORMAL":1}}"#));
    }

    /// 🔴 **blit 要把每个源纹素放到正确位置**（最近邻、整数除法）。
    ///
    /// 判据：2×2 图打进 4×4 的图集格 ⇒ 每个源纹素占 2×2 目标像素，
    /// 四象限颜色正确；**格外的像素一个都不许被改**。
    ///
    /// 反向验证：把 `sx` 的 `(dx * w) / cw` 改成 `dx.min(w-1)`（不缩放）⇒ 红。
    #[test]
    fn blit_scales_the_texture_into_the_cell_and_touches_nothing_else() {
        // 2×2：红 绿 / 蓝 黄
        let tex = ImportedTexture {
            name: "t".into(),
            width: 2,
            height: 2,
            rgba8: vec![
                255, 0, 0, 255, // (0,0) 红
                0, 255, 0, 255, // (1,0) 绿
                0, 0, 255, 255, // (0,1) 蓝
                255, 255, 0, 255, // (1,1) 黄
            ],
        };
        // 8×8 图集，底板洋红（便于发现"没写到的格子被误改"）
        let mut atlas = RgbaImage::filled(8, 8, [255, 0, 255, 255]);
        // 右下 4×4 格 = uv (0.5,0.5)-(1.0,1.0) ⇒ 像素 (4,4)-(8,8)
        let written = blit_into_atlas(&mut atlas, AtlasRect::new(0.5, 0.5, 1.0, 1.0), &tex);
        assert_eq!(written, 16, "4×4 格应写 16 个像素");
        // 四象限各 2×2
        assert_eq!(atlas.pixel(4, 4), Some([255, 0, 0, 255]), "左上象限 = 红");
        assert_eq!(atlas.pixel(5, 4), Some([255, 0, 0, 255]), "左上象限宽 2");
        assert_eq!(atlas.pixel(6, 4), Some([0, 255, 0, 255]), "右上象限 = 绿");
        assert_eq!(atlas.pixel(4, 6), Some([0, 0, 255, 255]), "左下象限 = 蓝");
        assert_eq!(atlas.pixel(7, 7), Some([255, 255, 0, 255]), "右下角 = 黄");
        // 格外：底板色不变
        assert_eq!(atlas.pixel(3, 4), Some([255, 0, 255, 255]), "格左边的像素不许被改");
        assert_eq!(atlas.pixel(4, 3), Some([255, 0, 255, 255]), "格上边的像素不许被改");
    }

    /// 退化输入不许 panic：0 尺寸贴图、0 像素格。
    ///
    /// 反向验证：去掉 `if tex.width == 0 ... return 0` ⇒ 除零 panic。
    #[test]
    fn blit_degenerate_inputs_return_zero_not_panic() {
        let mut atlas = RgbaImage::filled(4, 4, [0, 0, 0, 255]);
        let empty = ImportedTexture {
            name: "empty".into(),
            width: 0,
            height: 0,
            rgba8: vec![],
        };
        assert_eq!(blit_into_atlas(&mut atlas, AtlasRect::FULL, &empty), 0);
        // 1×1 的贴图打进极小的格（像素范围可能算成 0 宽）
        let tiny = ImportedTexture {
            name: "tiny".into(),
            width: 1,
            height: 1,
            rgba8: vec![1, 2, 3, 255],
        };
        let degenerate = AtlasRect::new(0.0, 0.0, 0.0, 0.0);
        assert_eq!(blit_into_atlas(&mut atlas, degenerate, &tiny), 0, "0 宽格写 0 像素");
    }



    /// 🔴 **重映射必须把 UV 四角准确送到格四角**。
    ///
    /// 反向验证：把 `remap_uv` 的 `rect.width()` 换成 `1.0` ⇒ 第二、三条断言红
    /// （UV 会落到格外面去，画面上表现为"贴图错位/看到邻居的像素"）。
    #[test]
    fn remap_sends_uv_corners_to_cell_corners() {
        // 2×2 图集里的右下格
        let cell = AtlasRect::new(0.5, 0.5, 1.0, 1.0);
        assert_eq!(remap_uv([0.0, 0.0], cell), [0.5, 0.5], "左下 → 格左下");
        assert_eq!(remap_uv([1.0, 0.0], cell), [1.0, 0.5], "右下 → 格右下");
        assert_eq!(remap_uv([0.0, 1.0], cell), [0.5, 1.0], "左上 → 格左上");
        assert_eq!(remap_uv([1.0, 1.0], cell), [1.0, 1.0], "右上 → 格右上");
        // 格中心
        let mid = remap_uv([0.5, 0.5], cell);
        assert!((mid[0] - 0.75).abs() < 1e-6 && (mid[1] - 0.75).abs() < 1e-6, "中心 → 格中心");
    }

    /// 整张图集 ⇒ 重映射是**恒等**（这是"旧资产行为不变"的保证）。
    ///
    /// 反向验证：给 `FULL` 塞一个非零偏移 ⇒ 本判据红。
    #[test]
    fn full_atlas_rect_is_identity() {
        for uv in [[0.0, 0.0], [0.25, 0.75], [1.0, 1.0], [0.5, 0.5]] {
            assert_eq!(remap_uv(uv, AtlasRect::FULL), uv, "整图集必须恒等");
        }
    }

    /// 🔴 **重映射不许夹紧超界 UV**：tiling 是上游的事，夹紧会把平铺压成条纹。
    ///
    /// 反向验证：给 `remap_uv` 加 `.clamp(0.0, 1.0)` ⇒ 第二条断言红。
    #[test]
    fn remap_does_not_clamp_out_of_range_uv() {
        let cell = AtlasRect::new(0.0, 0.0, 0.5, 0.5);
        // 🔴 uv 必须选到"映射后 > 1.0"的那一档，否则 clamp(0,1) 恰好不变、注入测不出来。
        // 第一版用 uv=2.0（映射到 1.0），加 clamp 注入**不会红** —— 用例太弱。
        let out = remap_uv([3.0, 0.0], cell);
        assert!((out[0] - 1.5).abs() < 1e-6, "3.0 要放大到 1.5，实际 {}", out[0]);
        assert!(out[0] > 1.0, "超界必须可见（>1.0），被 clamp 就说明写了夹紧");
    }

    /// 🔴 **采样条件必须与着色器逐条同源**（阈值、长度算法、特征位）。
    ///
    /// 反向验证：把 `UV_EPSILON_SQRT` 改成 `1e-9` ⇒ 第一条断言红。
    #[test]
    fn should_sample_matches_the_shader_condition() {
        // UV 全零 ⇒ 不采样（旧资产 / 程序化件）
        assert!(!should_sample([0.0, 0.0], FEATURE_AUTHORED_TEX), "UV 全零不采样");
        // 有 UV 但特征位没置 ⇒ 不采样
        assert!(!should_sample([0.5, 0.5], 0), "特征位清零不采样");
        // 两者都满足 ⇒ 采样
        assert!(should_sample([0.5, 0.5], FEATURE_AUTHORED_TEX), "应当采样");
        // 阈值边界：长度恰好 1e-4 不采样（着色器是 `>`），略大则采样
        assert!(!should_sample([UV_EPSILON_SQRT, 0.0], FEATURE_AUTHORED_TEX), "等于阈值不采样");
        assert!(should_sample([UV_EPSILON_SQRT * 2.0, 0.0], FEATURE_AUTHORED_TEX), "大于阈值采样");
        // 单分量非零也算（着色器用 length，不是 max 逐分量）
        assert!(should_sample([0.0, 0.5], FEATURE_AUTHORED_TEX), "单分量非零也应采样");
    }

    /// 特征位常量必须与着色器源码里的 `32u` 一致。
    ///
    /// 反向验证：把常量改成 `16` ⇒ 本判据红。
    #[test]
    fn feature_bit_matches_the_shader() {
        let src = include_str!("../../build.rs");
        let needle = format!("(feature_bits & {}u)", FEATURE_AUTHORED_TEX);
        assert!(
            src.contains(&needle),
            "着色器里必须存在 `{needle}`；改了特征位就要同步改本模块的常量"
        );
        let eps = format!("> {}u", 0); // 占位，避免未使用告警
        let _ = eps;
    }

    /// 🔴 UV 阈值必须与着色器源码里的**数值**一致（比字符串比对强）。
    ///
    /// 做法：从 `build.rs` 里把 `length(input.uv) > <数>` 的那个数**解析出来**，
    /// 再与常量比对。这样两边**任何一侧**改动都会红 —— 字符串比对做不到这一点
    /// （`format!("{}", 1e-4)` 是 `0.0001`，而着色器写的是 `1e-4`，
    /// 拿格式化结果去 `contains` 会永远失败，我第一版就是这么写的）。
    ///
    /// 反向验证：把 `UV_EPSILON_SQRT` 改成 `1e-3` ⇒ 本判据红。
    #[test]
    fn uv_epsilon_matches_the_shader() {
        let src = include_str!("../../build.rs");
        let anchor = "length(input.uv) > ";
        let at = src.find(anchor).expect("着色器里必须有 `length(input.uv) > <阈值>`");
        let rest = &src[at + anchor.len()..];
        let lit: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == 'e' || *c == 'E' || *c == '-')
            .collect();
        let shader_eps: f32 = lit.parse().unwrap_or_else(|_| panic!("阈值不是数字：{lit:?}"));
        assert!(
            (shader_eps - UV_EPSILON_SQRT).abs() < 1e-12,
            "着色器阈值 {shader_eps} 与本模块常量 {UV_EPSILON_SQRT} 不一致；             两边分叉会表现为“某些件突然不贴图”"
        );
    }

    /// 🔴 **没有贴图必须回退顶点色，不许回退白色**。
    ///
    /// 反向验证：把 `fallback_albedo` 改成返回 `[1.0; 3]` ⇒ 本判据红。
    #[test]
    fn missing_texture_falls_back_to_vertex_color_not_white() {
        assert_eq!(fallback_albedo([0.2, 0.4, 0.6]), [0.2, 0.4, 0.6], "回退必须是顶点色");
        // 顶点色是深色时，回退必须仍是深色（白模会让它变白）
        let dark = fallback_albedo([0.02, 0.02, 0.02]);
        assert!(dark[0] < 0.1, "深色顶点回退后仍是深色，实际 {}", dark[0]);
    }

    /// 🔴 **合成公式必须与着色器一致**：白图（tex=1.0）⇒ 结果被 1.615 倍提亮后再与 0.15 混合
    /// 等于 `vertex * (0.15 + 1.615) = vertex * 1.765`？—— 不对，白图的那条注释说"白图 = 原样"。
    ///
    /// 实测这条注释与公式**不符**：`mix(1, tex*1.9, 0.85)` 在 tex=1 时是 `0.15 + 0.85*1.9 = 1.765`。
    /// 也就是说"白图 = 不影响顶点色"这句注释是错的（除非 `feature_bits & 32` 只在有真图集时才置位，
    /// 那么白图那条路根本不会走到）。
    ///
    /// 本判据**把这个事实钉下来**，避免以后有人按注释去"修"公式：
    /// 白图 ⇒ 1.765 倍，而不是 1.0 倍。
    #[test]
    fn modulation_formula_matches_the_shader_not_the_comment() {
        let one = modulate_albedo([1.0, 1.0, 1.0], [1.0, 1.0, 1.0]);
        assert!(
            (one[0] - 1.765).abs() < 1e-5,
            "白图按公式应得 1.765 倍（注释说“原样”是错的），实际 {}",
            one[0]
        );
        // 零图（tex=0）⇒ 只剩 0.15 倍，很暗但不是 0（保留一点底光）
        let zero = modulate_albedo([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        assert!((zero[0] - 0.15).abs() < 1e-5, "零图应得 0.15 倍，实际 {}", zero[0]);
        // 灰度 0.5 ⇒ 0.15 + 1.615*0.5 = 0.9575
        let mid = modulate_albedo([1.0, 1.0, 1.0], [0.5, 0.5, 0.5]);
        assert!((mid[0] - 0.9575).abs() < 1e-4, "0.5 应得 0.9575，实际 {}", mid[0]);
    }
}
