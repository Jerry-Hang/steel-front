# -*- coding: utf-8 -*-
import io
p = 'src/engine/assets.rs'
s = io.open(p, encoding='utf-8').read()
add = '''

// ---------------------------------------------------------------------------
// PNG/JPEG 解码：Windows GDI+（系统组件，零外部库）→ RGBA8
// 复用 launcher wallpaper.rs 的 GdiplusStartup 模式；LockBits 取 32bpp ARGB
// ---------------------------------------------------------------------------
#[cfg(windows)]
pub mod gdi_img {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    type GpStatus = i32;
    #[repr(C)]
    struct Startup { ver: u32, cb: *mut c_void, bg: i32, codecs: i32 }
    #[link(name = "gdiplus")]
    extern "system" {
        fn GdiplusStartup(t: *mut usize, i: *const Startup, o: *mut c_void) -> GpStatus;
        fn GdipLoadImageFromFile(f: *const u16, img: *mut *mut c_void) -> GpStatus;
        fn GdipGetImageWidth(img: *mut c_void, w: *mut u32) -> GpStatus;
        fn GdipGetImageHeight(img: *mut c_void, h: *mut u32) -> GpStatus;
        fn GdipBitmapLockBits(bmp: *mut c_void, rect: *mut c_void, flags: u32, fmt: u32, locked: *mut c_void) -> GpStatus;
        fn GdipBitmapUnlockBits(bmp: *mut c_void, locked: *mut c_void) -> GpStatus;
        fn GdipDisposeImage(img: *mut c_void) -> GpStatus;
        fn GdiplusShutdown(t: usize);
    }

    static mut TOKEN: usize = 0;
    fn startup() {
        unsafe {
            if TOKEN == 0 {
                let i = Startup { ver: 1, cb: std::ptr::null_mut(), bg: 0, codecs: 0 };
                GdiplusStartup(&mut TOKEN, &i, std::ptr::null_mut());
            }
        }
    }

    /// 解码图片文件 → RGBA8（32bpp ARGB 内存序 = R,G,B,A）
    pub fn load_rgba(path: &str) -> Result<(Vec<u8>, u32, u32), String> {
        startup();
        let wide: Vec<u16> = std::path::Path::new(path).as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let mut img: *mut c_void = std::ptr::null_mut();
            let st = GdipLoadImageFromFile(wide.as_ptr(), &mut img);
            if st != 0 || img.is_null() {
                return Err(format!("图片加载失败（GDI+ 状态 {st}）: {path}"));
            }
            let (mut w, mut h) = (0u32, 0u32);
            GdipGetImageWidth(img, &mut w);
            GdipGetImageHeight(img, &mut h);
            // PixelFormat32bppARGB = 0x0026200A；LockUserInputBuffer = 0x00000001
            #[repr(C)]
            struct Rect { x: i32, y: i32, w: i32, h: i32 }
            let rect = Rect { x: 0, y: 0, w: w as i32, h: h as i32 };
            #[repr(C)]
            #[derive(Default)]
            struct Locked { rect: Rect, stride: i32, data: *mut u8, size: u32, flags: u32 }
            let mut locked: Locked = std::mem::zeroed();
            let st = GdipBitmapLockBits(img, &rect as *const _ as *mut c_void, 0x00000001u32, 0x0026200Au32, &mut locked as *mut _ as *mut c_void);
            if st != 0 || locked.data.is_null() {
                GdipDisposeImage(img);
                return Err(format!("图片锁定失败（GDI+ 状态 {st}）"));
            }
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row = std::slice::from_raw_parts(locked.data.add(y * locked.stride as usize), (w as usize) * 4);
                for p in row.chunks_exact(4) {
                    out.extend_from_slice(&[p[2], p[1], p[0], p[3]]); // BGRA→RGBA
                }
            }
            GdipBitmapUnlockBits(img, &mut locked as *mut _ as *mut c_void);
            GdipDisposeImage(img);
            Ok((out, w, h))
        }
    }
}
'''
s += add
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('gdi added')
