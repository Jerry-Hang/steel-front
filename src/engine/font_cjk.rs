//! 中文字形光栅化（Windows GDI，零第三方依赖）
//!
//! 用系统字体（微软雅黑）把 CJK 字符渲染为 8x8 点阵（1 位 alpha 掩码），
//! 供 ui.rs 的 CJK 字形表使用。字形按需生成并缓存，进程生命周期内有效。
//! 仅 Windows 可用（gdi32/user32 FFI）；非 Windows 平台回退 None（渲染为 '?'）。

#![cfg(windows)]

use std::ffi::c_void;
use std::os::raw::{c_int, c_uint};
use std::sync::Mutex;

#[link(name = "gdi32")]
extern "C" {
    fn CreateCompatibleDC(hdc: *mut c_void) -> *mut c_void;
    fn DeleteDC(hdc: *mut c_void) -> c_int;
    fn CreateCompatibleBitmap(hdc: *mut c_void, w: c_int, h: c_int) -> *mut c_void;
    fn CreateFontW(
        cHeight: c_int, cWidth: c_int, cEscapement: c_int, cOrientation: c_int,
        cWeight: c_int, bItalic: c_uint, bUnderline: c_uint, bStrikeOut: c_uint,
        iCharSet: c_uint, iOutPrecision: c_uint, iClipPrecision: c_uint, iQuality: c_uint,
        iPitchAndFamily: c_uint, pszFaceName: *const u16,
    ) -> *mut c_void;
    fn SelectObject(hdc: *mut c_void, h: *mut c_void) -> *mut c_void;
    fn DeleteObject(h: *mut c_void) -> c_int;
    fn SetBkMode(hdc: *mut c_void, mode: c_int) -> c_int;
    fn SetBkColor(hdc: *mut c_void, color: u32) -> u32;
    fn SetTextColor(hdc: *mut c_void, color: u32) -> u32;
    fn TextOutW(
        hdc: *mut c_void, x: c_int, y: c_int, s: *const u16, len: c_int,
    ) -> c_int;
    fn FillRect(hdc: *mut c_void, rect: *const c_void, brush: *mut c_void) -> c_int;
    fn CreateSolidBrush(color: u32) -> *mut c_void;
    fn GetDIBits(
        hdc: *mut c_void, hbm: *mut c_void, start: c_uint, lines: c_uint,
        buf: *mut c_void, bmi: *mut c_void, usage: c_uint,
    ) -> c_int;
}

#[link(name = "user32")]
extern "C" {
    fn GetDC(hwnd: *mut c_void) -> *mut c_void;
    fn ReleaseDC(hwnd: *mut c_void, hdc: *mut c_void) -> c_int;
}

#[repr(C)]
#[allow(non_snake_case)] // Win32 SDK 字段名
struct BitmapInfoHeader {
    biSize: u32,
    biWidth: i32,
    biHeight: i32,
    biPlanes: u16,
    biBitCount: u16,
    biCompression: u32,
    biSizeImage: u32,
    biXPelsPerMeter: i32,
    biYPelsPerMeter: i32,
    biClrUsed: u32,
    biClrImportant: u32,
}

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

const OPAQUE: c_int = 2;
const DEFAULT_CHARSET: c_uint = 1;
const OUT_TT_ONLY_PRECIS: c_uint = 7;
const CLIP_DEFAULT_PRECIS: c_uint = 0;
const ANTIALIASED_QUALITY: c_uint = 4;
const DEFAULT_PITCH: c_uint = 0;
const DIB_RGB_COLORS: c_uint = 0;

static CACHE: Mutex<Option<std::collections::HashMap<char, [u8; 8]>>> = Mutex::new(None);

/// CJK/全角字符判定（2026-08-16 扩展：补全角形式 0xFF00-0xFFEF 与扩展区，
/// 修复中文输入法标点"！（）"等渲染成 '?' 的问题）
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
/// 首次访问某字符时经 GDI 光栅化并缓存；失败返回 None。
pub fn glyph(ch: char) -> Option<[u8; 8]> {
    // 非 CJK 不进缓存（ASCII 走 5x7 内置字体）
    if !is_cjk_char(ch) {
        return None;
    }
    {
        let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(std::collections::HashMap::new());
        }
        let map = guard.as_mut().unwrap();
        if let Some(g) = map.get(&ch) {
            return Some(*g);
        }
    }
    // 缓存未命中：GDI 光栅化（每次调用一个字符，成本可接受；缓存后零开销）
    if let Some(bm) = rasterize(ch) {
        if let Ok(mut guard) = CACHE.lock() {
            if let Some(map) = guard.as_mut() {
                map.insert(ch, bm);
            }
        }
        Some(bm)
    } else {
        None
    }
}

fn rasterize(ch: char) -> Option<[u8; 8]> {
    // TextOutW → GetDIBits 路径（2026-08-19 重写）：GetGlyphOutlineW 在部分环境下
    // 输出的 GGO_BITMAP 内容异常（黑盒 13x15 却只有一根竖线），改用 GDI 文本引擎
    // 直接渲染到 16x16 内存位图再读回，CJK 渲染稳定可靠。
    unsafe {
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return None;
        }
        let dc = CreateCompatibleDC(screen_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        if dc.is_null() {
            return None;
        }
        // 16x16 兼容位图（32bpp）
        let bmp = CreateCompatibleBitmap(dc, 16, 16);
        if bmp.is_null() {
            DeleteDC(dc);
            return None;
        }
        let old_bmp = SelectObject(dc, bmp);
        let face: Vec<u16> = "Microsoft YaHei"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let font = CreateFontW(
            -16, 0, 0, 0, 400, 0, 0, 0,
            DEFAULT_CHARSET, OUT_TT_ONLY_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY,
            DEFAULT_PITCH, face.as_ptr(),
        );
        if font.is_null() {
            SelectObject(dc, old_bmp);
            DeleteObject(bmp);
            DeleteDC(dc);
            return None;
        }
        SelectObject(dc, font);
        SetBkMode(dc, OPAQUE);
        SetBkColor(dc, 0x0000_0000); // 黑底
        SetTextColor(dc, 0x00FF_FFFF); // 白字
        // 清背景（FillRect 黑）
        let brush = CreateSolidBrush(0x0000_0000);
        let rect = Rect { left: 0, top: 0, right: 16, bottom: 16 };
        FillRect(dc, &rect as *const Rect as *const c_void, brush);
        DeleteObject(brush);
        // 画字符（1 个 UTF-16 单元；中文 BMP 内均为单单元）
        let mut buf16 = [0u16; 2];
        let n = ch.encode_utf16(&mut buf16).len();
        TextOutW(dc, 0, 0, buf16.as_ptr(), n as c_int);
        // 读回 32bpp（top-down：biHeight 为负，行 0 = 顶部）
        let mut bmi = std::mem::zeroed::<BitmapInfoHeader>();
        bmi.biSize = 40;
        bmi.biWidth = 16;
        bmi.biHeight = -16;
        bmi.biPlanes = 1;
        bmi.biBitCount = 32;
        let mut px: Vec<u8> = vec![0u8; 16 * 16 * 4];
        let got = GetDIBits(
            dc,
            bmp,
            0,
            16,
            px.as_mut_ptr() as *mut c_void,
            &mut bmi as *mut BitmapInfoHeader as *mut c_void,
            DIB_RGB_COLORS,
        );
        // 还原并释放
        SelectObject(dc, old_bmp);
        DeleteObject(bmp);
        DeleteObject(font);
        DeleteDC(dc);
        if got == 0 {
            return None;
        }
        // 16x16 → 8x8：每 2x2 块任一亮像素即置 1（笔画保真）
        let mut out = [0u8; 8];
        for j in 0..8 {
            for i in 0..8 {
                let mut lit = false;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let x = i * 2 + dx;
                        let y = j * 2 + dy;
                        let off = (y * 16 + x) * 4;
                        if px[off] > 128 || px[off + 1] > 128 || px[off + 2] > 128 {
                            lit = true;
                        }
                    }
                }
                if lit {
                    out[j] |= 1 << (7 - i);
                }
            }
        }
        Some(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    /// TextOutW 光栅化回归：中文/全角标点字形应生成且内容非空
    #[test]
    fn cjk_glyph_generates() {
        assert!(is_cjk_char('中'), "中 应为 CJK");
        assert!(is_cjk_char('！'), "全角标点应为 CJK");
        assert!(!is_cjk_char('A'), "ASCII 不应判为 CJK");
        let g = glyph('中');
        assert!(g.is_some(), "中文字形生成失败（GDI 路径）");
        if let Some(rows) = g {
            // 字形应有多行笔画（不是单竖线/碎点）
            let filled_rows = rows.iter().filter(|b| **b != 0).count();
            let filled_cols = (0..8)
                .filter(|i| rows.iter().any(|b| (b >> (7 - i)) & 1 == 1))
                .count();
            assert!(
                filled_rows >= 3 && filled_cols >= 3,
                "字形过稀疏（rows={} cols={}）：{:?}",
                filled_rows,
                filled_cols,
                rows
            );
            // 缓存命中路径
            let g2 = glyph('中');
            assert_eq!(g, g2, "缓存应返回相同字形");
        }
    }
}
