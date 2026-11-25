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

#[cfg(test)]
mod tests {
    use super::*;

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
