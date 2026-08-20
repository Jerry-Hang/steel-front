//! 壁纸：GDI+（系统自带 gdiplus.dll）加载 PNG/JPG/BMP → HBITMAP；
//! cover 缩放（铺满保持纵横比裁剪）在渲染层做。
use std::os::windows::ffi::OsStrExt;
use std::ffi::c_void;

type GpStatus = i32;

#[repr(C)]
struct GdiplusStartupInput {
    gdiplus_version: u32,
    debug_event_callback: *mut c_void,
    suppress_background_thread: i32,
    suppress_external_codecs: i32,
}

#[link(name = "gdiplus")]
extern "system" {
    fn GdiplusStartup(token: *mut usize, input: *const GdiplusStartupInput, output: *mut c_void) -> GpStatus;
    fn GdiplusShutdown(token: usize);
    fn GdipLoadImageFromFile(filename: *const u16, image: *mut *mut c_void) -> GpStatus;
    fn GdipGetImageWidth(image: *mut c_void, width: *mut u32) -> GpStatus;
    fn GdipGetImageHeight(image: *mut c_void, height: *mut u32) -> GpStatus;
    fn GdipCreateHBITMAPFromBitmap(bitmap: *mut c_void, hbm: *mut *mut c_void, background: u32) -> GpStatus;
    fn GdipDisposeImage(image: *mut c_void) -> GpStatus;
}

static mut TOKEN: usize = 0;

pub fn init() {
    unsafe {
        if TOKEN == 0 {
            let input = GdiplusStartupInput {
                gdiplus_version: 1,
                debug_event_callback: std::ptr::null_mut(),
                suppress_background_thread: 0,
                suppress_external_codecs: 0,
            };
            GdiplusStartup(&mut TOKEN, &input, std::ptr::null_mut());
        }
    }
}

/// 加载图片文件为 HBITMAP；失败返回 None。调用方负责 DeleteObject。
pub fn load_bitmap(path: &str) -> Option<(*mut c_void, u32, u32)> {
    init();
    unsafe {
        let wpath: Vec<u16> = std::ffi::OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut img: *mut c_void = std::ptr::null_mut();
        if GdipLoadImageFromFile(wpath.as_ptr(), &mut img) != 0 || img.is_null() {
            return None;
        }
        let mut w: u32 = 0;
        let mut h: u32 = 0;
        GdipGetImageWidth(img, &mut w);
        GdipGetImageHeight(img, &mut h);
        let mut hbm: *mut c_void = std::ptr::null_mut();
        let status = GdipCreateHBITMAPFromBitmap(img, &mut hbm, 0x00223344);
        GdipDisposeImage(img);
        if status != 0 || hbm.is_null() || w == 0 || h == 0 {
            return None;
        }
        Some((hbm, w, h))
    }
}

/// cover 缩放：目标 w×h 内铺满（保持纵横比，超出裁剪）。
/// 返回 (src_x, src_y, src_w, src_h)——原图裁剪区域（渲染时 StretchBlt 到全窗口）。
pub fn cover_rect(img_w: u32, img_h: u32, win_w: i32, win_h: i32) -> (i32, i32, i32, i32) {
    let iw = img_w as f64;
    let ih = img_h as f64;
    let ww = win_w as f64;
    let wh = win_h as f64;
    if iw <= 0.0 || ih <= 0.0 || ww <= 0.0 || wh <= 0.0 {
        return (0, 0, img_w as i32, img_h as i32);
    }
    let scale = (ww / iw).max(wh / ih);
    let cw = (ww / scale) as i32;
    let ch = (wh / scale) as i32;
    let sx = ((iw as i32 - cw) / 2).max(0);
    let sy = ((ih as i32 - ch) / 2).max(0);
    (sx, sy, cw.min(img_w as i32), ch.min(img_h as i32))
}