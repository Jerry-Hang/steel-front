//! 钢铁前线启动器（零依赖 Win32 原生 GUI）
//!
//! 功能：启动游戏 / 设置安装路径 / 反馈 bug（GitHub issues）/
//! 检查更新（GitHub releases，curl 子进程）/ 壁纸背景。

#![cfg(windows)]
#![windows_subsystem = "windows"]

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
type LRESULT = isize;

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
const IMAGE_BITMAP: u32 = 0;
const LR_LOADFROMFILE: u32 = 0x0010;
const SW_SHOWNORMAL: i32 = 1;
const IDC_ARROW: LPCWSTR = 32512usize as LPCWSTR;
const WM_CTLCOLORSTATIC: u32 = 0x0138;

const ID_BTN_START: i32 = 1001;
const ID_BTN_FEEDBACK: i32 = 1003;
const ID_BTN_UPDATE: i32 = 1004;
const ID_BTN_BROWSE: i32 = 1005;
const ID_EDIT_PATH: i32 = 1006;
const ID_LBL_TITLE: i32 = 1007;
const ID_LBL_STATUS: i32 = 1008;

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(wc: *const WNDCLASS) -> u16;
    fn CreateWindowExW(
        ex: u32, class: LPCWSTR, title: LPCWSTR, style: u32,
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
    fn BeginPaint(h: HWND, ps: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(h: HWND, ps: *const PAINTSTRUCT) -> i32;
    fn GetWindowRect(h: HWND, rect: *mut RECT) -> i32;
    fn MessageBoxW(h: HWND, text: LPCWSTR, caption: LPCWSTR, ty: u32) -> i32;
    fn LoadImageW(
        inst: *mut c_void, name: LPCWSTR, ty: u32, w: i32, h: i32, flags: u32,
    ) -> *mut c_void;
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
    fn GetModuleFileNameW(mod_: *mut c_void, buf: *mut u16, size: u32) -> u32;
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
struct MSG {
    hwnd: HWND,
    message: u32,
    wParam: usize,
    lParam: isize,
    time: u32,
    pt: POINT,
}

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct PAINTSTRUCT {
    hdc: HDC,
    fErase: i32,
    rcPaint: RECT,
    fRestore: i32,
    fIncUpdate: i32,
    rgbReserved: [u8; 32],
}

#[repr(C)]
struct STARTUPINFO {
    cb: u32,
    lpReserved: *mut u16,
    lpDesktop: *mut u16,
    lpTitle: *mut u16,
    dwX: u32,
    dwY: u32,
    dwXSize: u32,
    dwYSize: u32,
    dwXCountChars: u32,
    dwYCountChars: u32,
    dwFillAttribute: u32,
    dwFlags: u32,
    wShowWindow: u16,
    cbReserved2: u16,
    lpReserved2: *mut u8,
    hStdInput: *mut c_void,
    hStdOutput: *mut c_void,
    hStdError: *mut c_void,
}

#[repr(C)]
struct PROCESS_INFORMATION {
    hProcess: *mut c_void,
    hThread: *mut c_void,
    dwProcessId: u32,
    dwThreadId: u32,
}

#[repr(C)]
struct BROWSEINFO {
    hwndOwner: HWND,
    pidlRoot: *const c_void,
    pszDisplayName: *mut u16,
    lpszTitle: LPCWSTR,
    ulFlags: u32,
    lpfn: *const c_void,
    lParam: isize,
    iImage: i32,
}

fn wstr(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

const CONFIG_FILE: &str = "launcher.ini";

fn launcher_dir() -> String {
    let mut buf = [0u16; 1024];
    unsafe { GetModuleFileNameW(ptr::null_mut(), buf.as_mut_ptr(), 1024); }
    let path = String::from_utf16_lossy(&buf).trim_end_matches('\0').to_string();
    path.rsplit_once('\\').map(|(d, _)| d.to_string()).unwrap_or_else(|| ".".to_string())
}

fn config_path() -> String {
    format!("{}\\{}", launcher_dir(), CONFIG_FILE)
}

fn config_load() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    map
}

fn config_save(map: &std::collections::HashMap<String, String>) {
    let mut out = String::new();
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for k in keys {
        out.push_str(&format!("{}={}\n", k, map[k]));
    }
    let _ = std::fs::write(config_path(), out);
}

fn game_dir() -> String {
    if let Some(p) = config_load().get("game_path") {
        if !p.is_empty() {
            return p.clone();
        }
    }
    launcher_dir()
}

fn game_exe_path() -> String {
    format!("{}\\steel-front.exe", game_dir())
}

fn game_installed() -> bool {
    std::path::Path::new(&game_exe_path()).exists()
}

struct App {
    h_edit_path: HWND,
    h_status: HWND,
    wallpaper: *mut c_void,
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
                    let mem = CreateCompatibleDC(hdc);
                    let old = SelectObject(mem, a.wallpaper);
                    StretchBlt(hdc, 0, 0, w, h, mem, 0, 0, 640, 360, 0x00CC0020);
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
                    if game_installed() {
                        let exe = wstr(&game_exe_path());
                        let mut si = std::mem::zeroed::<STARTUPINFO>();
                        si.cb = std::mem::size_of::<STARTUPINFO>() as u32;
                        let mut pi = std::mem::zeroed::<PROCESS_INFORMATION>();
                        let dir = wstr(&game_dir());
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
                        set_status("未检测到游戏，请先设置路径");
                    }
                }
                ID_BTN_BROWSE => {
                    let title = wstr("选择游戏安装目录（含 steel-front.exe）");
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
                            let mut cfg = config_load();
                            cfg.insert("game_path".to_string(), p.clone());
                            config_save(&cfg);
                            let t = wstr(&p);
                            SetWindowTextW(app().h_edit_path, t.as_ptr());
                            if std::path::Path::new(&format!("{}\\steel-front.exe", p)).exists() {
                                set_status("路径有效：已找到游戏");
                            } else {
                                set_status("路径已保存（未找到 steel-front.exe）");
                            }
                        }
                    }
                }
                ID_BTN_FEEDBACK => {
                    let url = wstr("https://github.com/Jerry-Hang/steel-front/issues/new");
                    let open = wstr("open");
                    ShellExecuteW(hwnd, open.as_ptr(), url.as_ptr(), ptr::null(), ptr::null(), SW_SHOWNORMAL);
                }
                ID_BTN_UPDATE => {
                    set_status("正在检查更新...");
                    check_update(hwnd);
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

fn check_update(hwnd: HWND) {
    let url = "https://api.github.com/repos/Jerry-Hang/steel-front/releases/latest";
    let tmp = std::env::temp_dir().join("sf_release.json");
    let cmd = format!(
        "curl.exe -sL -o \"{}\" -H \"User-Agent: launcher\" \"{}\"",
        tmp.display(),
        url
    );
    let _ = std::process::Command::new("cmd")
        .args(["/C", &cmd])
        .creation_flags(0x08000000)
        .status();
    if let Ok(text) = std::fs::read_to_string(&tmp) {
        if let Some(idx) = text.find("\"tag_name\":\"") {
            let rest = &text[idx + 13..];
            let tag: String = rest.chars().take_while(|c| *c != '"').collect();
            let local = env!("CARGO_PKG_VERSION");
            let latest = tag.trim_start_matches('v');
            let mut msg = format!("最新版本：{}（本地 {}）", tag, local);
            if latest != local {
                msg.push_str("\n检测到新版本！请前往下载：\nhttps://github.com/Jerry-Hang/steel-front/releases");
            } else {
                msg.push_str("\n已是最新版本");
            }
            let t = wstr(&msg);
            let cap = wstr("检查更新");
            unsafe { MessageBoxW(hwnd, t.as_ptr(), cap.as_ptr(), 0x40) };
        } else {
            let t = wstr("无法解析更新信息（网络或 API 异常）");
            let cap = wstr("检查更新");
            unsafe { MessageBoxW(hwnd, t.as_ptr(), cap.as_ptr(), 0x10) };
        }
    } else {
        let t = wstr("网络请求失败：请检查网络连接");
        let cap = wstr("检查更新");
        unsafe { MessageBoxW(hwnd, t.as_ptr(), cap.as_ptr(), 0x10) };
    }
}

fn main() {
    unsafe {
        let inst = GetModuleHandleW(ptr::null());
        let class = wstr("SteelFrontLauncherClass");
        let wc = WNDCLASS {
            style: 0,
            lpfnWndProc: wnd_proc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);

        let mut wp = ptr::null_mut();
        let wp_file = format!("{}\\launcher_wallpaper.bmp", launcher_dir());
        if std::path::Path::new(&wp_file).exists() {
            wp = LoadImageW(
                ptr::null_mut(), wstr(&wp_file).as_ptr(), IMAGE_BITMAP, 0, 0, LR_LOADFROMFILE,
            );
        }

        let title = wstr("钢铁前线 Steel Front - 启动器");
        let hwnd = CreateWindowExW(
            0, class.as_ptr(), title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100, 60, 720, 480, ptr::null_mut(), ptr::null_mut(), inst, ptr::null_mut(),
        );
        ShowWindow(hwnd, SW_SHOWNORMAL);
        UpdateWindow(hwnd);

        let btn_style = WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON | WS_TABSTOP;
        let create_btn = |id: i32, text: &str, x: i32, y: i32, w: i32, h: i32| -> HWND {
            CreateWindowExW(
                0, wstr("BUTTON").as_ptr(), wstr(text).as_ptr(), btn_style,
                x, y, w, h, hwnd, id as HMENU, inst, ptr::null_mut(),
            )
        };
        let _title_lbl = CreateWindowExW(
            0, wstr("STATIC").as_ptr(), wstr("钢铁前线 启动器").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT, 24, 20, 400, 36, hwnd, ID_LBL_TITLE as HMENU, inst, ptr::null_mut(),
        );
        let edit_path = CreateWindowExW(
            0, wstr("EDIT").as_ptr(), wstr("").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | ES_AUTOHSCROLL,
            24, 80, 520, 26, hwnd, ID_EDIT_PATH as HMENU, inst, ptr::null_mut(),
        );
        let _browse = create_btn(ID_BTN_BROWSE, "浏览...", 560, 80, 110, 26);
        let _start = create_btn(ID_BTN_START, "启动游戏", 24, 140, 180, 44);
        let _feedback = create_btn(ID_BTN_FEEDBACK, "反馈 Bug", 220, 140, 120, 44);
        let _update = create_btn(ID_BTN_UPDATE, "检查更新", 356, 140, 130, 44);
        let status = CreateWindowExW(
            0, wstr("STATIC").as_ptr(), wstr("就绪").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT, 24, 210, 640, 24, hwnd, ID_LBL_STATUS as HMENU, inst, ptr::null_mut(),
        );

        let game_path = game_dir();
        let t = wstr(&game_path);
        SetWindowTextW(edit_path, t.as_ptr());

        APP = Some(App {
            h_edit_path: edit_path,
            h_status: status,
            wallpaper: wp,
        });
        if game_installed() {
            set_status("游戏就绪，点击启动");
        } else {
            set_status("未找到游戏：点击 浏览 选择目录，或把游戏放入本目录");
        }

        let mut msg = std::mem::zeroed::<MSG>();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}