//! 中文字形（预烘焙 12x12 像素点阵，**Noto Sans SC / SIL OFL 1.1**）
//!
//! 🔴 **2026-09-14 换源，理由是授权**：本模块头一度写着"SimSun 宋体 12px"，
//! 而 **SimSun 是专有字体、禁止再分发** —— 本仓要能对外分发并且**卖商业授权**，
//! 带着它就是把授权风险直接发给下游。现字模全部来自 **Noto Sans SC（SIL OFL 1.1）**，
//! 许可全文随仓在 `assets/fonts/OFL-NotoSansCJK.txt`。
//! ⚠️ **模块头曾经与数据不符**（数据换了、注释没换），这比"没写"更危险 ——
//! 谁来做授权核查，读到的会是"它用了 SimSun"。**换源时必须同时改这里。**
//!
//! 生成方式（**可复现，别再手工提取**）：`tools/extract_cjk_glyphs.py`
//! - `--scan` 扫描 `src/` + `build.rs` 收集**真正用到的码点** → `tools/cjk_used_codepoints.txt`
//! - `--font <NotoSansSC路径>` 据此重新生成 `engine/cjk_glyphs.rs`
//! - 表**只含用到的码点**：21,486 条 → **1,580 条**（2.26 MB → 163 KB，−92.6%）
//!
//! 查询 O(log n) 二分查找，跨平台无依赖。
//! **表内容的硬契约在 `tests::cjk_glyph_generates`**（三条断言：清单非空、缺字为零、条数不多余）
//! ——旧断言是 `len() > 20000`，那条**用死数据就能满足**，正是 92.6% 冗余的来源。

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
        // 🔴 2026-09-14：这条断言原本是 `CJK_GLYPHS.len() > 20000`，理由是"表应覆盖
        // 完整简体字集"。**那句话把 92.6% 的死数据写成了要求** —— 实测源码只用到
        // 1591 个 CJK 码点，而旧表有 21486 条，其中 19895 条没有任何引用。
        // 一条能被"多塞两万字"满足的断言，守不住任何东西（教训 14 的形态）。
        //
        // 真正该守的契约只有一个：**表必须覆盖源码用到的每一个码点**，
        // 否则 HUD 会渲染成空白。清单由 `tools/extract_cjk_glyphs.py --scan` 生成，
        // 那是唯一知道"哪些字被用到"的地方，所以直接把它编进来比对。
        let used: Vec<char> = include_str!("../../tools/cjk_used_codepoints.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .filter_map(|l| u32::from_str_radix(l.trim(), 16).ok())
            .filter_map(char::from_u32)
            .collect();
        assert!(
            used.len() > 500,
            "用到码点清单只有 {} 条，像是没生成成功",
            used.len()
        );
        let missing: Vec<char> = used
            .iter()
            .copied()
            .filter(|ch| glyph(*ch).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "字模表缺 {} 个源码用到的字，HUD 会渲染成空白：{}",
            missing.len(),
            missing.iter().take(40).collect::<String>()
        );
        assert_eq!(
            CJK_GLYPHS.len(),
            used.len(),
            "字模表应当**不含**源码用不到的字（旧版多带了 19906 个死条目）"
        );
    }

    /// 🔴 **清单是生成物，所以上面那条断言守不住"清单本身过期"。**
    ///
    /// `cjk_used_codepoints.txt` 由 `--scan` 生成，而**没有任何东西强制它重跑**。
    /// 2026-09-14 实测到过这个缺口：源码里已经躺着一批注释用字，清单却还是一小时前
    /// 那一版（1580 → 实际 1591）。当时那 11 个字只在注释里，**屏幕上看不出任何异常** ——
    /// 等到有人把某个缺字写进真正会渲染的字符串，症状就是"HUD 上少一个字的空白"，
    /// 而那时离根因已经隔了一次手动的生成步骤。
    ///
    /// 所以这里**独立地**重扫一遍 `src/`：判据不取自清单，而取自源码本身。
    /// 过滤器用 `is_cjk_char`（本模块那份）—— `ui.rs::is_cjk` 在 Windows 上转调它，
    /// 而生成器 `tools/extract_cjk_glyphs.py::is_cjk` 用的是**同一条范围**：
    /// 两边必须同一个判据，否则测试会因为"判据本身漂移"而红/绿得不讲道理。
    /// 新加一个中文只要没重跑 `--scan`，这条就会红，并直接报出缺的是哪个字、在哪个文件。
    #[test]
    fn source_cjk_codepoints_all_have_glyphs() {
        use std::collections::HashSet;
        use std::path::PathBuf;

        // 生成物自己不算"源码用到"（它是坐标点表本身）；
        // 本模块也必须跳过 —— 它按设计就引用表外的字（`'中'` / `'！'` 等断言样本）。
        const SKIP: [&str; 2] = ["cjk_glyphs.rs", "font_cjk.rs"];

        fn collect(dir: &PathBuf, found: &mut Vec<(PathBuf, char)>, files: &mut usize) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect(&path, found, files);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if SKIP.contains(&name) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                *files += 1;
                for ch in text.chars() {
                    if is_cjk_char(ch) {
                        found.push((path.clone(), ch));
                    }
                }
            }
        }

        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut found = Vec::new();
        let mut files = 0usize;
        collect(&manifest.join("src"), &mut found, &mut files);
        // 目录走空时这条会静默通过 —— 那正是教训 27 的形态（"没测到"与"测到 0"分不清）
        assert!(files > 10, "只扫到 {} 个 .rs，路径大概不对", files);

        let have: HashSet<char> = CJK_GLYPHS.iter().map(|&(c, _)| c).collect();
        let mut missing: Vec<(PathBuf, char)> = found
            .into_iter()
            .filter(|(_, ch)| !have.contains(ch))
            .collect();
        missing.sort_by_key(|(p, c)| (p.clone(), *c));
        missing.dedup();

        let mut unreadable = String::new();
        for (path, ch) in missing.iter().take(20) {
            unreadable.push_str(&format!(
                "\n  '{}' (U+{:04X}) 出现在 {}",
                ch,
                *ch as u32,
                path.strip_prefix(&manifest).unwrap_or(path).display()
            ));
        }
        assert!(
            missing.is_empty(),
            "源码里有 {} 个 CJK 码点没有字模，HUD 会渲染成空白。\
             \n修复：python tools/extract_cjk_glyphs.py --scan 然后 --font <NotoSansSC.otf>{}",
            missing.len(),
            unreadable
        );
    }
}
