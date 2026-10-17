//! 与平台无关的小工具。
//!
//! 🔴 **为什么 `split_by_bytes` 放在这里而不是 `android_log.rs`**（2026-10-17 实测踩到）：
//! `android_log` 整个模块是 `#[cfg(target_os = "android")]` ⇒ **它的 `#[cfg(test)]`
//! 在开发机（Linux/Windows）上根本不编译**，我给它写的判据跑出来是
//! `0 passed; 750 filtered out` —— **一条永远不编译的判据等于没写**。
//! ⇒ 可测逻辑必须放在**跨平台**模块里。

/// 按**字节上限**切分，但只在 UTF-8 字符边界上断开。
///
/// 为什么不用 `as_bytes().chunks()`：那是按字节切，而中文一个字 3 字节，
/// 切在中间就得到**非法 UTF-8** ⇒ logcat 里显示成乱码/问号。
/// 这里走 `char_indices`，保证每一段都是合法 UTF-8。
/// 只在 Android（唯一消费者）与测试里编译 —— 开发机上的 release 构建里没有调用方，
/// 直接放着会报 `never used`；而**无理由的 `#[allow(dead_code)]` 是仓库明令禁止的**。
#[cfg(any(target_os = "android", test))]
pub fn split_by_bytes(s: &str, max: usize) -> Vec<&str> {
    if s.len() <= max {
        return vec![s];
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    // 🔴 **判据必须是「加上这个字符会不会超」**（2026-10-17，外部复核指出）：
    // `char_indices()` 给的 `i` 是字符的**起点**偏移，不是终点
    // ⇒ 原来 `i - start >= max` 会让一段最长到 `max + 3`
    // （3 字节汉字正好跨在边界上：`split_by_bytes("a中", 2)` 原来返回 4 字节的 `"a中"`）。
    for (i, ch) in s.char_indices() {
        // 单字符本身就超 `max` 时无解（不能把字符切开），只在 `i > start` 时切
        if i > start && i + ch.len_utf8() - start > max {
            out.push(&s[start..i]);
            start = i;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}


#[cfg(test)]
mod log_split_tests {
    use super::split_by_bytes;

    /// 🔴 **`split_by_bytes` 的每一段都必须 ≤ `max`**（单字符本身超 `max` 时除外）。
    ///
    /// 2026-10-17 外部复核指出：原实现用 `char_indices()` 的**字符起点**判阈值
    /// ⇒ 一段最长可到 `max + 3`（3 字节汉字正好跨在边界上）。
    /// 最简反例：`split_by_bytes("a中", 2)` 原来返回整串 `"a中"`（**4 字节 > 2**）。
    ///
    /// 两条一起钉：**拼回去逐字节相等** + **每段 ≤ max**（单字符例外）。
    #[test]
    fn split_never_exceeds_the_byte_limit() {
        let cases: &[&str] = &["a中", "中a", "ab中cd", "中中中", "hello world 中文测试 abc", ""];
        for max in [2usize, 3, 4, 7, 8, 3500] {
            for s in cases {
                let parts = split_by_bytes(s, max);
                assert_eq!(parts.concat(), *s, "max={max} s={s:?} 拼回与原串不等");
                for (i, part) in parts.iter().enumerate() {
                    let single_char = part.chars().count() == 1;
                    assert!(
                        part.len() <= max || single_char,
                        "max={max} s={s:?} 第 {i} 段 {} 字节 > {max}（且不是单字符）",
                        part.len()
                    );
                }
            }
        }
    }
}
