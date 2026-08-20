//! 钢铁前线启动器（零依赖 Win32 原生 GUI）
//! 功能：启动 / 安装向导 / 自动更新 / 桌面快捷方式 / 反馈 / 壁纸(用户选图) / 资源路径导向

#![cfg(windows)]
#![windows_subsystem = "windows"]

mod config;
mod install;
mod shortcut;
mod update;
mod wallpaper;

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::ptr;

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HDC = *mut c_void;
type HMENU = *mut c_void;
type HBRUSH = *mut c_void;
type LPCWSTR = *const u16;
type WNDPROC = unsafe extern "system" fn(HWND, u32, usize, isize) -> isize;

const WM_DESTROY: u32 = 0x0002;
const WM_COMMAND: u32 = 0x0111;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF0000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_CHILD: u32 = 0x40000000;
const WS_TABSTOP: u32 = 0x00010000;
const WS_BORDER: u32 = 0x00800000;
const BS_PUSHBUTTON: u32 = 0;
const SS_LEFT: u32 = 0;
const ES_AUTOHSCROLL: u32 = 0x0080;
const SW_SHOWNORMAL: i32 = 1;
const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;
const IMAGE_BITMAP: u32 = 0;
const LR_LOADFROMFILE: u32 = 0x0010;

const ID_BTN_START: i32 = 1001;
const ID_BTN_FEEDBACK: i32 = 1003;
const ID_BTN_UPDATE: i32 = 1004;
const ID_BTN_BROWSE: i32 = 1005;
const ID_EDIT_PATH: i32 = 1006;
const ID_LBL_TITLE: i32 = 1007;
const ID_LBL_STATUS: i32 = 1008;
const ID_BTN_SETTINGS: i32 = 1009;
const ID_BTN_SHORTCUT: i32 = 1010;
// 设置页
const ID_BTN_BACK: i32 = 1011;
const ID_BTN_WALLPAPER: i32 = 1012;
const ID_BTN_WALLPAPER_CLEAR: i32 = 1013;
const ID_EDIT_WALLPAPER: i32 = 1014;
const ID_EDIT_MAPS: i32 = 1015;
const ID_BTN_MAPS: i32 = 1016;
const ID_EDIT_SOUNDS: i32 = 1017;
const ID_BTN_SOUNDS: i32 = 1018;
const ID_EDIT_MODELS: i32 = 1019;
const ID_BTN_MODELS: i32 = 1020;
const ID_BTN_SAVE: i32 = 1021;
// 安装页
const ID_BTN_INSTALL: i32 = 1022;
const ID_BTN_INSTALL_BROWSE: i32 = 1023;
const ID_EDIT_INSTALL: i32 = 1024;

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(wc: *const WNDCLASS) -> u16;
    fn CreateWindowExW(
        ex: u32, class: LPCWSTR, text: LPCWSTR, style: u32,
        x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: HMENU,
        inst: HINSTANCE, param: *mut c_void,
    ) -> HWND;
    fn DefWindowProcW(h: HWND, msg: u32, wp: usize, lp: isize) -> isize;
    fn PostQuitMessage(code: i32);
    fn GetMessageW(msg: *mut MSG, h: HWND, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> isize;
    fn ShowWindow(h: HWND, cmd: i32);
    fn UpdateWindow(h: HWND);
    fn SetWindowTextW(h: HWND, text: LPCWSTR) -> i32;
    fn GetWindowTextW(h: HWND, buf: *mut u16, len: i32) -> i32;
    fn BeginPaint(h: HWND, ps: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(h: HWND, ps: *const PAINTSTRUCT) -> i32;
    fn GetWindowRect(h: HWND, rect: *mut RECT) -> i32;
    fn MessageBoxW(h: HWND, text: LPCWSTR, caption: LPCWSTR, ty: u32) -> i32;
    fn LoadImageW(inst: *mut c_void, name: LPCWSTR, ty: u32, w: i32, h: i32, flags: u32) -> *mut c_void;
    fn InvalidateRect(h: HWND, rect: *const c_void, erase: i32) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> i32;
    fn SelectObject(hdc: HDC, obj: *mut c_void) -> *mut c_void;
    fn StretchBlt(
        dst: HDC, dx: i32, dy: i32, dw: i32, dh: i32,
        src: HDC, sx: i32, sy: i32, sw: i32, sh: i32, mode: u32,
    ) -> i32;
    fn CreateSolidBrush(color: u32) -> HBRUSH;
    fn DeleteObject(obj: *mut c_void) -> i32;
    fn FillRect(hdc: HDC, rect: *const RECT, brush: HBRUSH) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        h: HWND, op: LPCWSTR, file: LPCWSTR, args: LPCWSTR,
        dir: LPCWSTR, show: i32,
    ) -> isize;
    fn SHBrowseForFolderW(info: *const BROWSEINFO) -> *mut c_void;
    fn SHGetPathFromIDListW(pidl: *const c_void, path: *mut u16) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateProcessW(
        app: LPCWSTR, cmd: *mut u16, pa: *const c_void, ta: *const c_void,
        inherit: i32, flags: u32, env: *const c_void, dir: LPCWSTR,
        si: *const STARTUPINFO, pi: *mut PROCESS_INFORMATION,
    ) -> i32;
    fn CloseHandle(h: *mut c_void) -> i32;
    fn GetModuleFileNameW(m: *mut c_void, buf: *mut u16, size: u32) -> u32;
    fn GetModuleHandleW(name: LPCWSTR) -> HINSTANCE;
}

#[repr(C)]
struct WNDCLASS {
    style: u32,
    lpfnWndProc: WNDPROC,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: HINSTANCE,
    hIcon: *mut c_void,
    hCursor: *mut c_void,
    hbrBackground: HBRUSH,
    lpszMenuName: LPCWSTR,
    lpszClassName: LPCWSTR,
}

#[repr(C)]
struct MSG { hwnd: HWND, message: u32, wParam: usize, lParam: isize, time: u32, pt: POINT }

#[repr(C)]
struct POINT { x: i32, y: i32 }

#[repr(C)]
struct RECT { left: i32, top: i32, right: i32, bottom: i32 }

#[repr(C)]
struct PAINTSTRUCT { hdc: HDC, fErase: i32, rcPaint: RECT, fRestore: i32, fIncUpdate: i32, rgbReserved: [u8; 32] }

#[repr(C)]
struct STARTUPINFO {
    cb: u32, lpReserved: *mut u16, lpDesktop: *mut u16, lpTitle: *mut u16,
    dwX: u32, dwY: u32, dwXSize: u32, dwYSize: u32,
    dwXCountChars: u32, dwYCountChars: u32, dwFillAttribute: u32, dwFlags: u32,
    wShowWindow: u16, cbReserved2: u16, lpReserved2: *mut u8,
    hStdInput: *mut c_void, hStdOutput: *mut c_void, hStdError: *mut c_void,
}

#[repr(C)]
struct PROCESS_INFORMATION { hProcess: *mut c_void, hThread: *mut c_void, dwProcessId: u32, dwThreadId: u32 }

#[repr(C)]
struct BROWSEINFO {
    hwndOwner: HWND, pidlRoot: *const c_void, pszDisplayName: *mut u16,
    lpszTitle: LPCWSTR, ulFlags: u32, lpfn: *const c_void, lParam: isize, iImage: i32,
}

fn wstr(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

struct App {
    page: i32,
    hwnd: HWND,
    // 主页控件
    home: Vec<HWND>,
    h_edit_path: HWND,
    h_status: HWND,
    // 设置页控件
    settings: Vec<HWND>,
    h_edit_wallpaper: HWND,
    h_edit_maps: HWND,
    h_edit_sounds: HWND,
    h_edit_models: HWND,
    // 安装页控件
    install_page: Vec<HWND>,
    h_edit_install: HWND,
    // 壁纸
    wallpaper: *mut c_void,
    wallpaper_w: u32,
    wallpaper_h: u32,
}

static mut APP: Option<App> = None;

unsafe fn app() -> &'static mut App {
    APP.as_mut().unwrap()
}

fn set_status(text: &str) {
    unsafe {
        if let Some(a) = APP.as_mut() {
            let t = wstr(text);
            SetWindowTextW(a.h_status, t.as_ptr());
        }
    }
}

fn show_controls(list: &[HWND], show: bool) {
    unsafe {
        for &h in list {
            ShowWindow(h, if show { SW_SHOW } else { SW_HIDE });
        }
    }
}

fn switch_page(page: i32) {
    unsafe {
        let a = app();
        show_controls(&a.home, page == 0);
        show_controls(&a.settings, page == 1);
        show_controls(&a.install_page, page == 2);
        a.page = page;
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        WM_PAINT => {
            let mut ps = std::mem::zeroed::<PAINTSTRUCT>();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetWindowRect(hwnd, &mut rc);
            let w = rc.right - rc.left;
            let h = rc.bottom - rc.top;
            if let Some(a) = APP.as_mut() {
                if !a.wallpaper.is_null() {
                    let (sx, sy, sw, sh) = wallpaper::cover_rect(a.wallpaper_w, a.wallpaper_h, w, h);
                    let mem = CreateCompatibleDC(hdc);
                    let old = SelectObject(mem, a.wallpaper);
                    StretchBlt(hdc, 0, 0, w, h, mem, sx, sy, sw, sh, 0x00CC0020);
                    SelectObject(mem, old);
                    DeleteDC(mem);
                } else {
                    let brush = CreateSolidBrush(0x00223344);
                    let r = RECT { left: 0, top: 0, right: w, bottom: h };
                    FillRect(hdc, &r, brush);
                    DeleteObject(brush);
                }
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as i32;
            match id {
                ID_BTN_START => {
                    if config::game_installed() {
                        let exe = wstr(&config::game_exe());
                        let mut si = std::mem::zeroed::<STARTUPINFO>();
                        si.cb = std::mem::size_of::<STARTUPINFO>() as u32;
                        let mut pi = std::mem::zeroed::<PROCESS_INFORMATION>();
                        let dir = wstr(&config::game_dir());
                        let ok = CreateProcessW(
                            exe.as_ptr(), ptr::null_mut(), ptr::null(), ptr::null(),
                            0, 0, ptr::null(), dir.as_ptr(), &si, &mut pi,
                        );
                        if ok != 0 {
                            CloseHandle(pi.hThread);
                            CloseHandle(pi.hProcess);
                            set_status("游戏已启动");
                        } else {
                            set_status("启动失败：请检查游戏路径");
                        }
                    } else {
                        set_status("未检测到游戏，请先安装或设置路径");
                    }
                }
                ID_BTN_BROWSE | ID_BTN_INSTALL_BROWSE => {
                    let is_install = id == ID_BTN_INSTALL_BROWSE;
                    let title = wstr("选择游戏安装目录");
                    let mut buf = [0u16; 1024];
                    let bi = BROWSEINFO {
                        hwndOwner: hwnd,
                        pidlRoot: ptr::null(),
                        pszDisplayName: buf.as_mut_ptr(),
                        lpszTitle: title.as_ptr(),
                        ulFlags: 0x0001 | 0x0040,
                        lpfn: ptr::null(),
                        lParam: 0,
                        iImage: 0,
                    };
                    let pidl = SHBrowseForFolderW(&bi);
                    if !pidl.is_null() {
                        let mut path = [0u16; 1024];
                        if SHGetPathFromIDListW(pidl, path.as_mut_ptr()) != 0 {
                            let p = String::from_utf16_lossy(&path).trim_end_matches('\0').to_string();
                            if is_install {
                                let t = wstr(&p);
                                SetWindowTextW(app().h_edit_install, t.as_ptr());
                            } else {
                                config::set("game_path", &p);
                                let t = wstr(&p);
                                SetWindowTextW(app().h_edit_path, t.as_ptr());
                                if config::game_installed() { set_status("路径有效：已找到游戏"); }
                                else { set_status("路径已保存（未找到 steel-front.exe）"); }
                            }
                        }
                    }
                }
                ID_BTN_SETTINGS => switch_page(1),
                ID_BTN_BACK => switch_page(0),
                ID_BTN_FEEDBACK => {
                    let url = wstr("https://github.com/Jerry-Hang/steel-front/issues/new");
                    let open = wstr("open");
                    ShellExecuteW(hwnd, open.as_ptr(), url.as_ptr(), ptr::null(), ptr::null(), SW_SHOWNORMAL);
                }
                ID_BTN_SHORTCUT => {
                    set_status("正在创建桌面快捷方式...");
                    match shortcut::create_desktop_shortcut(&config::game_exe(), &config::game_dir(), "Steel Front") {
                        Ok(_) => set_status("桌面快捷方式已创建"),
                        Err(e) => set_status(&format!("快捷方式失败: {}", e)),
                    }
                }
                ID_BTN_UPDATE => {
                    set_status("正在检查更新...");
                    check_update_ui(hwnd);
                }
                ID_BTN_WALLPAPER => {
                    // 选壁纸图片（GDI+ 支持 png/jpg/bmp）
                    let picked = pick_image_file(hwnd);
                    if let Some(p) = picked {
                        if let Some((hbm, w, h)) = wallpaper::load_bitmap(&p) {
                            let a = app();
                            if !a.wallpaper.is_null() { DeleteObject(a.wallpaper); }
                            a.wallpaper = hbm;
                            a.wallpaper_w = w;
                            a.wallpaper_h = h;
                            let t = wstr(&p);
                            SetWindowTextW(a.h_edit_wallpaper, t.as_ptr());
                            config::set("wallpaper", &p);
                            InvalidateRect(hwnd, ptr::null(), 1);
                            set_status("壁纸已更换");
                        } else {
                            set_status("图片加载失败（支持 PNG/JPG/BMP）");
                        }
                    }
                }
                ID_BTN_WALLPAPER_CLEAR => {
                    let a = app();
                    if !a.wallpaper.is_null() { DeleteObject(a.wallpaper); }
                    a.wallpaper = ptr::null_mut();
                    let t = wstr("");
                    SetWindowTextW(a.h_edit_wallpaper, t.as_ptr());
                    config::set("wallpaper", "");
                    InvalidateRect(hwnd, ptr::null(), 1);
                    set_status("壁纸已移除");
                }
                ID_BTN_MAPS | ID_BTN_SOUNDS | ID_BTN_MODELS => {
                    let key = match id { ID_BTN_MAPS => "maps_path", ID_BTN_SOUNDS => "sounds_path", _ => "models_path" };
                    let title = wstr("选择资源目录");
                    let mut buf = [0u16; 1024];
                    let bi = BROWSEINFO {
                        hwndOwner: hwnd, pidlRoot: ptr::null(), pszDisplayName: buf.as_mut_ptr(),
                        lpszTitle: title.as_ptr(), ulFlags: 0x0001 | 0x0040, lpfn: ptr::null(), lParam: 0, iImage: 0,
                    };
                    let pidl = SHBrowseForFolderW(&bi);
                    if !pidl.is_null() {
                        let mut path = [0u16; 1024];
                        if SHGetPathFromIDListW(pidl, path.as_mut_ptr()) != 0 {
                            let p = String::from_utf16_lossy(&path).trim_end_matches('\0').to_string();
                            config::set(key, &p);
                            let a = app();
                            let target = match id { ID_BTN_MAPS => a.h_edit_maps, ID_BTN_SOUNDS => a.h_edit_sounds, _ => a.h_edit_models };
                            let t = wstr(&p);
                            SetWindowTextW(target, t.as_ptr());
                        }
                    }
                }
                ID_BTN_SAVE => {
                    config::write_resource_paths();
                    set_status("资源路径已保存（写入 resource_paths.ini）");
                    switch_page(0);
                }
                ID_BTN_INSTALL => {
                    // 读取安装路径并执行安装
                    let mut buf = [0u16; 1024];
                    GetWindowTextW(app().h_edit_install, buf.as_mut_ptr(), 1024);
                    let dest = String::from_utf16_lossy(&buf).trim_end_matches('\0').to_string();
                    if dest.is_empty() { set_status("请先选择安装路径"); return 0; }
                    set_status("正在安装...");
                    match install::install_to(&dest) {
                        Ok(_) => {
                            config::set("game_path", &dest);
                            let t = wstr(&dest);
                            SetWindowTextW(app().h_edit_path, t.as_ptr());
                            config::write_resource_paths();
                            set_status("安装完成！可点击启动游戏");
                            let _ = shortcut::create_desktop_shortcut(&config::game_exe(), &dest, "Steel Front");
                            switch_page(0);
                        }
                        Err(e) => set_status(&format!("安装失败: {}", e)),
                    }
                }
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// 文件选择对话框（壁纸图片）——GetOpenFileName 简化：用文件对话框需要 commdlg，
/// 这里用系统 PowerShell 的 OpenFileDialog 替代（零依赖）
fn pick_image_file(hwnd: HWND) -> Option<String> {
    let ps = "$d=New-Object Windows.Forms.OpenFileDialog;$d.Filter='图片 (*.png;*.jpg;*.jpeg;*.bmp)|*.png;*.jpg;*.jpeg;*.bmp';if($d.ShowDialog() -eq [Windows.Forms.DialogResult]::OK){$d.FileName}else{''}";
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", "Add-Type -AssemblyName System.Windows.Forms;", ps])
        .creation_flags(0x08000000)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn check_update_ui(hwnd: HWND) {
    match update::check_latest() {
        Ok((tag, zip_url)) => {
            let local = env!("CARGO_PKG_VERSION");
            let latest = tag.trim_start_matches('v');
            if latest != local {
                let mut msg = format!("发现新版本：{}（本地 {}）", tag, local);
                if let Some(url) = zip_url.as_ref() {
                    msg.push_str(&format!("\n是否下载并自动更新？\n下载源：{}", url));
                } else {
                    msg.push_str("\n（该版本无 zip 资产，请前往 GitHub Releases 手动下载）");
                }
                let t = wstr(&msg);
                let cap = wstr("发现新版本");
                let r = unsafe { MessageBoxW(hwnd, t.as_ptr(), cap.as_ptr(), 0x24) }; // YESNO|ICONQUESTION
                if r == 6 {
                    if let Some(url) = zip_url {
                        set_status("正在下载更新...");
                        match update::download_and_apply(&url, &config::game_dir()) {
                            Ok(_) => set_status("更新完成！请重新启动游戏"),
                            Err(e) => set_status(&format!("更新失败: {}", e)),
                        }
                    }
                }
            } else {
                let msg = format!("已是最新版本：{}（本地 {}）", tag, local);
                let t = wstr(&msg);
                let cap = wstr("检查更新");
                unsafe { MessageBoxW(hwnd, t.as_ptr(), cap.as_ptr(), 0x40) };
            }
        }
        Err(e) => {
            let msg = format!("检查更新失败：{}", e);
            let t = wstr(&msg);
            let cap = wstr("检查更新");
            unsafe { MessageBoxW(hwnd, t.as_ptr(), cap.as_ptr(), 0x10) };
        }
    }
}

fn main() {
    unsafe {
        wallpaper::init();
        let inst = GetModuleHandleW(ptr::null());
        let class = wstr("SteelFrontLauncherClass");
        let wc = WNDCLASS {
            style: 0, lpfnWndProc: wnd_proc, cbClsExtra: 0, cbWndExtra: 0,
            hInstance: inst, hIcon: ptr::null_mut(), hCursor: ptr::null_mut(),
            hbrBackground: ptr::null_mut(), lpszMenuName: ptr::null(), lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);

        let title = wstr("钢铁前线 Steel Front - 启动器");
        let hwnd = CreateWindowExW(
            0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100, 60, 760, 520, ptr::null_mut(), ptr::null_mut(), inst, ptr::null_mut(),
        );
        ShowWindow(hwnd, SW_SHOWNORMAL);
        UpdateWindow(hwnd);

        // 控件工厂
        let btn_style = WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON | WS_TABSTOP;
        let create_btn = |id: i32, text: &str, x: i32, y: i32, w: i32, h: i32| -> HWND {
            CreateWindowExW(
                0, wstr("BUTTON").as_ptr(), wstr(text).as_ptr(), btn_style,
                x, y, w, h, hwnd, id as HMENU, inst, ptr::null_mut(),
            )
        };
        let create_static = |id: i32, text: &str, x: i32, y: i32, w: i32, h: i32| -> HWND {
            CreateWindowExW(
                0, wstr("STATIC").as_ptr(), wstr(text).as_ptr(), WS_CHILD | WS_VISIBLE | SS_LEFT,
                x, y, w, h, hwnd, id as HMENU, inst, ptr::null_mut(),
            )
        };
        let create_edit = |id: i32, x: i32, y: i32, w: i32, h: i32| -> HWND {
            CreateWindowExW(
                0, wstr("EDIT").as_ptr(), wstr("").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_BORDER | ES_AUTOHSCROLL,
                x, y, w, h, hwnd, id as HMENU, inst, ptr::null_mut(),
            )
        };

        // ---- 主页 ----
        let lbl_title = create_static(ID_LBL_TITLE, "钢铁前线 启动器", 24, 20, 400, 30);
        let edit_path = create_edit(ID_EDIT_PATH, 24, 80, 540, 26);
        let browse = create_btn(ID_BTN_BROWSE, "浏览...", 580, 80, 110, 26);
        let start = create_btn(ID_BTN_START, "启动游戏", 24, 140, 160, 44);
        let feedback = create_btn(ID_BTN_FEEDBACK, "反馈 Bug", 196, 140, 110, 44);
        let update = create_btn(ID_BTN_UPDATE, "检查更新", 318, 140, 120, 44);
        let settings = create_btn(ID_BTN_SETTINGS, "设置", 450, 140, 90, 44);
        let shortcut_btn = create_btn(ID_BTN_SHORTCUT, "创建桌面快捷方式", 24, 200, 190, 36);
        let status = create_static(ID_LBL_STATUS, "就绪", 24, 250, 640, 24);

        // ---- 设置页 ----
        let back = create_btn(ID_BTN_BACK, "← 返回", 24, 20, 90, 30);
        let lbl_wp = create_static(ID_LBL_TITLE, "壁纸（用户自选图片，自动缩放铺满）", 24, 66, 400, 22);
        let edit_wp = create_edit(ID_EDIT_WALLPAPER, 24, 94, 430, 26);
        let btn_wp = create_btn(ID_BTN_WALLPAPER, "选择图片...", 470, 94, 110, 26);
        let btn_wp_clear = create_btn(ID_BTN_WALLPAPER_CLEAR, "移除", 590, 94, 80, 26);
        let lbl_res = create_static(ID_LBL_TITLE, "资源目录导向（启动器只写配置，游戏读取）", 24, 136, 420, 22);
        let edit_maps = create_edit(ID_EDIT_MAPS, 24, 164, 430, 26);
        let btn_maps = create_btn(ID_BTN_MAPS, "地图...", 470, 164, 110, 26);
        let edit_sounds = create_edit(ID_EDIT_SOUNDS, 24, 200, 430, 26);
        let btn_sounds = create_btn(ID_BTN_SOUNDS, "音效...", 470, 200, 110, 26);
        let edit_models = create_edit(ID_EDIT_MODELS, 24, 236, 430, 26);
        let btn_models = create_btn(ID_BTN_MODELS, "建模...", 470, 236, 110, 26);
        let save = create_btn(ID_BTN_SAVE, "保存设置", 24, 290, 120, 36);

        // ---- 安装页 ----
        let lbl_install = create_static(ID_LBL_TITLE, "欢迎安装钢铁前线", 24, 20, 400, 30);
        let lbl_install2 = create_static(ID_LBL_TITLE, "请选择安装目录（将复制 game 游戏包）", 24, 56, 400, 22);
        let edit_install = create_edit(ID_EDIT_INSTALL, 24, 90, 540, 26);
        let btn_install_browse = create_btn(ID_BTN_INSTALL_BROWSE, "浏览...", 580, 90, 110, 26);
        let btn_install = create_btn(ID_BTN_INSTALL, "开始安装", 24, 140, 160, 44);

        // 组装页面控件列表
        let home = vec![lbl_title, edit_path, browse, start, feedback, update, settings, shortcut_btn, status];
        let settings_v = vec![back, lbl_wp, edit_wp, btn_wp, btn_wp_clear, lbl_res, edit_maps, btn_maps, edit_sounds, btn_sounds, edit_models, btn_models, save];
        let install_v = vec![lbl_install, lbl_install2, edit_install, btn_install_browse, btn_install];

        // 初始路径与壁纸
        let game_path = config::game_dir();
        let t = wstr(&game_path);
        SetWindowTextW(edit_path, t.as_ptr());
        let cfg = config::load();
        if let Some(wp) = cfg.get("wallpaper") {
            if !wp.is_empty() {
                let _ = wallpaper::load_bitmap(wp);
            }
        }
        let mut wp_bitmap: *mut c_void = ptr::null_mut();
        let mut wp_w: u32 = 0;
        let mut wp_h: u32 = 0;
        if let Some(wp) = cfg.get("wallpaper") {
            if !wp.is_empty() {
                if let Some((hbm, w, h)) = wallpaper::load_bitmap(wp) {
                    wp_bitmap = hbm; wp_w = w; wp_h = h;
                    let tw = wstr(wp);
                    SetWindowTextW(edit_wp, tw.as_ptr());
                }
            }
        }
        for (key, edit) in [("maps_path", edit_maps), ("sounds_path", edit_sounds), ("models_path", edit_models)] {
            if let Some(v) = cfg.get(key) {
                let tv = wstr(v);
                SetWindowTextW(edit, tv.as_ptr());
            }
        }

        APP = Some(App {
            page: 0, hwnd,
            home, h_edit_path: edit_path, h_status: status,
            settings: settings_v, h_edit_wallpaper: edit_wp, h_edit_maps: edit_maps,
            h_edit_sounds: edit_sounds, h_edit_models: edit_models,
            install_page: install_v, h_edit_install: edit_install,
            wallpaper: wp_bitmap, wallpaper_w: wp_w, wallpaper_h: wp_h,
        });

        // 首次运行检测：无游戏 + 有安装源 → 进入安装页
        let need_install = !config::game_installed() && !install::install_sources().is_empty();
        if config::game_installed() {
            set_status("游戏就绪，点击启动");
            switch_page(0);
        } else if need_install {
            switch_page(2);
            set_status("检测到游戏包：请选择安装路径");
        } else {
            set_status("未找到游戏：请点击 设置/浏览 选择目录，或把游戏包放入本目录");
            switch_page(0);
        }

        let mut msg = std::mem::zeroed::<MSG>();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}