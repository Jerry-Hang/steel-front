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
    fn GetDeviceCaps(hdc: *mut c_void, index: c_int) -> c_int;
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
/// GetDeviceCaps 索引：垂直 DPI（每逻辑英寸像素数）
const LOGPIXELSY: c_int = 90;

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

/// 取中文字形（16x16 点阵，行主序每行 1 字节，bit15=左侧）。
/// 首次访问某字符时经 GDI 光栅化并缓存；失败返回 None。
/// 16x16 分辨率：8x8 对复杂汉字（设/置/暴）笔画过密会糊成方块且内容占不满
/// 格子导致视觉压扁（2026-08-20 升级）。
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
        // 诊断（RV3D_CJK_DIAG=1）：打印实际生成的字形，验证 DPI 路径
        if std::env::var("RV3D_CJK_DIAG").as_deref() == Ok("1") {
            log::info!("cjk-diag: '{}' OK rows={:02X?}", ch, bm);
        }
        Some(bm)
    } else {
        // 诊断：光栅化失败路径（定位 DPI/位图问题）
        if std::env::var("RV3D_CJK_DIAG").as_deref() == Ok("1") {
            log::info!("cjk-diag: '{}' RASTERIZE-FAILED", ch);
        }
        None
    }
}

fn rasterize(ch: char) -> Option<[u8; 8]> {
    // TextOutW → GetDIBits 路径：GDI 文本引擎直接渲染到内存位图再读回，CJK 稳定。
    // DPI 感知（2026-08-19）：游戏进程是 DPI-aware，屏幕兼容 DC 的字体渲染按实际
    // DPI 缩放（150%/200% 屏 -16px 字体实际 24/32px）——固定 16x16 位图会把字形
    // 裁成"横条"（压扁）。按 GetDeviceCaps 取 DC 实际 DPI 动态计算字体像素，
    // 位图 32x32 容纳 200% 缩放，再按实际字体像素精确采样到 8x8。
    unsafe {
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return None;
        }
        let dc = CreateCompatibleDC(screen_dc);
        let dpi = GetDeviceCaps(dc, LOGPIXELSY).max(96);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        if dc.is_null() {
            return None;
        }
        // 逻辑 16px 字符 → 物理像素（96dpi=16，200% = 32，400% clamp 64）。
        // 16px 字体 1:1 采样到 16x16（1px/格）细节最完整——24px 字体 1.5px/格
        // 采样会把"风"的乂、"暴"的底部混叠成团。笔画加粗交给采样后的
        // 单向膨胀（1→2 格，屏幕与英文 5x7 同粗），不引入采样混叠。
        let font_px = ((16.0 * dpi as f32 / 96.0).round() as i32).clamp(16, 64);
        let bmp = CreateCompatibleBitmap(dc, 64, 64);
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
            -font_px, 0, 0, 0, 400, 0, 0, 0,
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
        // 清背景
        let brush = CreateSolidBrush(0x0000_0000);
        let rect = Rect { left: 0, top: 0, right: 64, bottom: 64 };
        FillRect(dc, &rect as *const Rect as *const c_void, brush);
        DeleteObject(brush);
        // 画字符（BMP 内单 UTF-16 单元）
        let mut buf16 = [0u16; 2];
        let n = ch.encode_utf16(&mut buf16).len();
        TextOutW(dc, 0, 0, buf16.as_ptr(), n as c_int);
        // 读回 32bpp（top-down：行 0 = 顶部）
        let mut bmi = std::mem::zeroed::<BitmapInfoHeader>();
        bmi.biSize = 40;
        bmi.biWidth = 64;
        bmi.biHeight = -64;
        bmi.biPlanes = 1;
        bmi.biBitCount = 32;
        let mut px: Vec<u8> = vec![0u8; 64 * 64 * 4];
        let got = GetDIBits(
            dc,
            bmp,
            0,
            64,
            px.as_mut_ptr() as *mut c_void,
            &mut bmi as *mut BitmapInfoHeader as *mut c_void,
            DIB_RGB_COLORS,
        );
        SelectObject(dc, old_bmp);
        DeleteObject(bmp);
        DeleteObject(font);
        DeleteDC(dc);
        if got == 0 {
            return None;
        }
        sample_glyph(&px, font_px as usize)
    }
}

/// 64x64 位图（字形占前 font_px 行/列）→ 8x8 掩码。
/// 内部：16x16 比例采样（细节完整）→ 垂直拉伸占满 → 2x2 取或降采样 8x8。
/// 渲染层每格 1×scale（与英文 5x7 格同尺寸）：笔画 1 格 = 英文同粗、
/// 屏幕字 8 格 ≈ 英文 7 行齐平（2026-08-20 最终方案）。
fn sample_glyph(px: &[u8], font_px: usize) -> Option<[u8; 8]> {
    let fp = font_px.max(1);
    let mut g16 = [0u16; 16];
    for j in 0..16 {
        let y0 = j * fp / 16;
        let y1 = ((j + 1) * fp) / 16;
        for i in 0..16 {
            let x0 = i * fp / 16;
            let x1 = ((i + 1) * fp) / 16;
            let mut lit = false;
            for y in y0..y1 {
                for x in x0..x1 {
                    let off = (y * 64 + x) * 4;
                    if off + 2 < px.len()
                        && (px[off] > 128 || px[off + 1] > 128 || px[off + 2] > 128)
                    {
                        lit = true;
                    }
                }
            }
            if lit {
                g16[j] |= 1 << (15 - i);
            }
        }
    }
    // 垂直拉伸：GDI 基线偏移使内容偏上（占不满 16 行 → 显示"扁"），映射铺满
    let mut top = 16usize;
    let mut bottom = 0usize;
    for j in 0..16 {
        if g16[j] != 0 {
            top = top.min(j);
            bottom = j;
        }
    }
    if top < bottom {
        let h = bottom - top + 1;
        if h < 16 {
            let mut v = [0u16; 16];
            for j in 0..16 {
                v[j] = g16[top + j * h / 16];
            }
            g16 = v;
        }
    }
    // 2x2 取或降采样 16x16 → 8x8：笔画（16 格中 2 格）→ 8 格中 1 格，
    // 渲染每格 1×scale → 屏幕笔画与英文同粗；细节（乂/米字）在 8x8 保留。
    let mut out = [0u8; 8];
    for j in 0..8 {
        for i in 0..8 {
            let mut lit = false;
            for dy in 0..2 {
                for dx in 0..2 {
                    if (g16[j * 2 + dy] >> (15 - (i * 2 + dx))) & 1 == 1 {
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
            assert_eq!(rows.len(), 8, "8x8 字形应为 8 行");
            let filled_rows = rows.iter().filter(|b| **b != 0).count();
            let filled_cols = (0..8)
                .filter(|i| rows.iter().any(|b| (b >> (7 - i)) & 1 == 1))
                .count();
            assert!(
                filled_rows >= 6 && filled_cols >= 6,
                "字形过稀疏（rows={} cols={}）：{:?}",
                filled_rows,
                filled_cols,
                rows
            );
            let g2 = glyph('中');
            assert_eq!(g, g2, "缓存应返回相同字形");
        }
    }

    /// DPI 一致性：同一字形在 96/150/200% DPI（16/24/32px）采样形状一致，
    /// 回归"中文字被压扁/糊成方块"问题。
    #[test]
    fn glyph_shape_consistent_across_dpi() {
        // 16px 的"中"：上下横 + 中竖 + 中横（简化图样）
        let mut base = [0u8; 256];
        for x in 0..16 {
            base[x] = 255;
            base[15 * 16 + x] = 255;
        }
        for y in 0..16 {
            base[y * 16 + 7] = 255;
        }
        for x in 2..14 {
            base[7 * 16 + x] = 255;
        }
        fn upscale(src: &[u8], size: usize) -> Vec<u8> {
            let mut d = vec![0u8; size * size];
            for y in 0..size {
                for x in 0..size {
                    d[y * size + x] = src[(y * 16 / size) * 16 + (x * 16 / size)];
                }
            }
            d
        }
        fn to32(src: &[u8], size: usize) -> Vec<u8> {
            let mut p = vec![0u8; 64 * 64 * 4];
            for y in 0..size {
                for x in 0..size {
                    let v = src[y * size + x];
                    let off = (y * 64 + x) * 4;
                    p[off] = v;
                    p[off + 1] = v;
                    p[off + 2] = v;
                }
            }
            p
        }
        let s16 = to32(&base, 16);
        let s24 = to32(&upscale(&base, 24), 24);
        let s32 = to32(&upscale(&base, 32), 32);
        let g16 = sample_glyph(&s16, 16).unwrap();
        let g24 = sample_glyph(&s24, 24).unwrap();
        let g32 = sample_glyph(&s32, 32).unwrap();
        // 区间取整允许边界行 ±1 像素差异（真实字体 hinting 同理），
        // 断言结构特征一致而非逐字节相等：
        // ① 顶部/底部有笔画（不压扁）② 左列有笔画（外框完整）③ 笔画行数接近
        let filled = |g: &[u8; 8]| g.iter().filter(|b| **b != 0).count();
        for g in [&g16, &g24, &g32] {
            assert!(g[0] != 0 && g[7] != 0, "顶部与底部都应有笔画: {:?}", g);
            assert!(
                (0..8).any(|r| (g[r] & 0x80) != 0),
                "左列应有笔画（外框完整）: {:?}",
                g
            );
        }
        let rows = [filled(&g16), filled(&g24), filled(&g32)];
        assert!(
            rows.iter().max().unwrap() - rows.iter().min().unwrap() <= 2,
            "不同 DPI 笔画行数应接近: {:?} vs {:?} vs {:?}",
            g16,
            g24,
            g32
        );
    }
}