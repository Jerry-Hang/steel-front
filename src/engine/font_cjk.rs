//! 中文字形（预烘焙 8x8 像素点阵，Fusion Pixel Font 8px，SIL OFL 1.1 开源授权）
//!
//! 2026-08-20 彻底换路线：抛弃 GDI 运行时动态生成（四轮修复证明该路线必然
//! 产生压扁/肿胀/混叠/糊块问题）。改用开源手工点阵字体——设计师逐像素优化，
//! 8x8 下"暴/风/设"等复杂字结构完整。查询 O(log n) 二分查找，跨平台无依赖。

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

/// 取中文字形（8x8 点阵，行主序每行 1 字节，bit7=左侧）。
/// 查表（二分查找）；表外字符返回 None（渲染回退 '?'）。
pub fn glyph(ch: char) -> Option<[u8; 8]> {
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
            let filled_rows = rows.iter().filter(|b| **b != 0).count();
            let filled_cols = (0..8)
                .filter(|i| rows.iter().any(|b| (b >> (7 - i)) & 1 == 1))
                .count();
            assert!(
                filled_rows >= 5 && filled_cols >= 5,
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
