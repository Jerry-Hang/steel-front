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
    fn CreateFontW(
        cHeight: c_int, cWidth: c_int, cEscapement: c_int, cOrientation: c_int,
        cWeight: c_int, bItalic: c_uint, bUnderline: c_uint, bStrikeOut: c_uint,
        iCharSet: c_uint, iOutPrecision: c_uint, iClipPrecision: c_uint, iQuality: c_uint,
        iPitchAndFamily: c_uint, pszFaceName: *const u16,
    ) -> *mut c_void;
    fn SelectObject(hdc: *mut c_void, h: *mut c_void) -> *mut c_void;
    fn DeleteObject(h: *mut c_void) -> c_int;
    fn SetBkMode(hdc: *mut c_void, mode: c_int) -> c_int;
    fn SetTextColor(hdc: *mut c_void, color: u32) -> u32;
    fn GetGlyphOutlineW(
        hdc: *mut c_void, uChar: c_uint, fuFormat: c_uint, lpgm: *mut c_void,
        cbBuffer: c_uint, lpvBuffer: *mut c_void, lpmat2: *const c_void,
    ) -> c_uint;
}

#[link(name = "user32")]
extern "C" {
    fn GetDC(hwnd: *mut c_void) -> *mut c_void;
    fn ReleaseDC(hwnd: *mut c_void, hdc: *mut c_void) -> c_int;
}

#[repr(C)]
#[allow(non_snake_case)] // Win32 SDK 字段名
struct GlyphMetrics {
    gmBlackBoxX: u32,
    gmBlackBoxY: u32,
    gmptGlyphOrigin: [i32; 2],
    gmCellIncX: i16,
    gmCellIncY: i16,
}

const TRANSPARENT: c_int = 1;
const GGO_BITMAP: c_uint = 1;
const DEFAULT_CHARSET: c_uint = 1;
const OUT_TT_ONLY_PRECIS: c_uint = 7;
const CLIP_DEFAULT_PRECIS: c_uint = 0;
const ANTIALIASED_QUALITY: c_uint = 4;
const DEFAULT_PITCH: c_uint = 0;

static CACHE: Mutex<Option<std::collections::HashMap<char, [u8; 8]>>> = Mutex::new(None);

/// 取中文字形（8x8 点阵，行主序每行 1 字节，bit7=左侧）。
/// 首次访问某字符时经 GDI 光栅化并缓存；失败返回 None。
pub fn glyph(ch: char) -> Option<[u8; 8]> {
    // 非 CJK 不进缓存（ASCII 走 5x7 内置字体）
    let cp = ch as u32;
    if !(0x4E00..=0x9FFF).contains(&cp) && !(0x3000..=0x303F).contains(&cp) {
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
        let face: Vec<u16> = "Microsoft YaHei"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let font = CreateFontW(
            -8, 0, 0, 0, 400, 0, 0, 0,
            DEFAULT_CHARSET, OUT_TT_ONLY_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY,
            DEFAULT_PITCH, face.as_ptr(),
        );
        if font.is_null() {
            DeleteDC(dc);
            return None;
        }
        SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, 0x00FF_FFFF);
        let mut gm = std::mem::zeroed::<GlyphMetrics>();
        let mat2 = [0i32, 0, 0, 0, 65536, 0, 0, 65536];
        let buf: Vec<u8> = vec![0u8; 8 * 8 * 4];
        let size = GetGlyphOutlineW(
            dc,
            ch as u32,
            GGO_BITMAP,
            &mut gm as *mut GlyphMetrics as *mut c_void,
            buf.len() as u32,
            buf.as_ptr() as *mut c_void,
            &mat2 as *const [i32; 8] as *const c_void,
        );
        DeleteObject(font);
        DeleteDC(dc);
        if size == u32::MAX || gm.gmBlackBoxX == 0 || gm.gmBlackBoxY == 0 {
            return None;
        }
        let row_stride = ((gm.gmBlackBoxX + 31) / 32 * 4) as usize;
        let mut out = [0u8; 8];
        let (bw, bh) = (gm.gmBlackBoxX as usize, gm.gmBlackBoxY as usize);
        for j in 0..8 {
            let sy = if bh <= 8 { j + (8 - bh) / 2 } else { j * bh / 8 };
            for i in 0..8 {
                let sx = if bw <= 8 { i + (8 - bw) / 2 } else { i * bw / 8 };
                let src = buf[sy * row_stride + sx];
                if src > 0x40 {
                    out[j] |= 1 << (7 - i);
                }
            }
        }
        Some(out)
    }
}