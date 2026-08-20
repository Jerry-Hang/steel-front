//! 中文字形（预烘焙 12x12 像素点阵，SimSun 宋体 12px 硬边位图）
//!
//! 2026-08-20 换用宋体：Fusion Pixel 8px 黑体在 8x8 容量下笔画粗细不均、
//! 风格非宋体。SimSun 12px（Windows 系统字体，构建时一次性提取为硬边位图，
//! 运行时纯查表）——12x12 容量笔画均匀、横细竖粗宋体特征、结构正统。
//! 查询 O(log n) 二分查找，跨平台无依赖。

use crate::engine::cjk_glyphs::CJK_GLYPHS;

/// CJK/全角字符判定（含 CJK 标点、假名、全角形式等）
pub fn is_cjk_char(ch: char) -> bool {
    let cp = ch as u32;
    (0x2E80..=0x2FDF).contains(&cp) // 部首/康熙部首
        || (0x3000..=0x303F).contains(&cp) // CJK 标点
        || (0x3040..=0x30FF).contains(&cp) // 假名（界面兼容）
        || (0x3100..=0x31FF).contains(&cp) // 注音/笔画
        || (0x3200..=0x33FF).contains(&cp) // 带圈 CJK/兼容
        || (0x3400..=0x4DBF).contains(&cp) // 扩展 A
        || (0x4E00..=0x9FFF).contains(&cp) // 统一表意
        || (0xF900..=0xFAFF).contains(&cp) // 兼容表意
        || (0xFE30..=0xFE6F).contains(&cp) // 竖排/小写变体
        || (0xFF00..=0xFFEF).contains(&cp) // 全角形式（！（）等）
        || (0x20000..=0x2A6DF).contains(&cp) // 扩展 B
}

/// 取中文字形（12x12 点阵，行主序每行 u16 低 12 位，bit11=左侧）。
/// 查表（二分查找）；表外字符返回 None（渲染回退 '?'）。
pub fn glyph(ch: char) -> Option<[u16; 12]> {
    if !is_cjk_char(ch) {
        return None;
    }
    CJK_GLYPHS
        .binary_search_by_key(&ch, |&(c, _)| c)
        .ok()
        .map(|i| CJK_GLYPHS[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 预烘焙字形回归：常用字应全部可查且内容非空
    #[test]
    fn cjk_glyph_generates() {
        assert!(is_cjk_char('中'), "中 应为 CJK");
        assert!(is_cjk_char('！'), "全角标点应为 CJK");
        assert!(!is_cjk_char('A'), "ASCII 不应判为 CJK");
        // 复杂字（历史糊块重灾区）全部可查且笔画充分
        for ch in ['中', '风', '暴', '设', '歼', '灭', '敌', '人', '连', '发'] {
            let rows = glyph(ch).unwrap_or_else(|| panic!("{} 应有点阵", ch));
            assert_eq!(rows.len(), 12, "12x12 字形应为 12 行");
            let filled_rows = rows.iter().filter(|b| **b != 0).count();
            let filled_cols = (0..12)
                .filter(|i| rows.iter().any(|b| (b >> (11 - i)) & 1 == 1))
                .count();
            assert!(
                filled_rows >= 8 && filled_cols >= 8,
                "{} 字形过稀疏（rows={} cols={}）：{:?}",
                ch,
                filled_rows,
                filled_cols,
                rows
            );
        }
        assert!(CJK_GLYPHS.len() > 20000, "表应覆盖完整简体字集");
    }
}
