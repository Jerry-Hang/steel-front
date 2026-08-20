//! 配置读写（ini 风格，UTF-8）
use std::collections::HashMap;
use std::ffi::c_void;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleFileNameW(m: *mut c_void, buf: *mut u16, size: u32) -> u32;
}

const CONFIG_FILE: &str = "launcher.ini";

/// 启动器所在目录
pub fn launcher_dir() -> String {
    let mut buf = [0u16; 1024];
    unsafe { GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), 1024); }
    let path = String::from_utf16_lossy(&buf).trim_end_matches('\0').to_string();
    path.rsplit_once('\\').map(|(d, _)| d.to_string()).unwrap_or_else(|| ".".to_string())
}

fn config_path() -> String {
    format!("{}\\{}", launcher_dir(), CONFIG_FILE)
}

pub fn load() -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    map
}

pub fn save(map: &HashMap<String, String>) {
    let mut out = String::new();
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for k in keys {
        out.push_str(&format!("{}={}\n", k, map[k]));
    }
    let _ = std::fs::write(config_path(), out);
}

pub fn get(key: &str) -> Option<String> {
    load().get(key).cloned()
}

pub fn set(key: &str, val: &str) {
    let mut map = load();
    map.insert(key.to_string(), val.to_string());
    save(&map);
}

/// 游戏目录：配置优先，否则启动器同目录
pub fn game_dir() -> String {
    if let Some(p) = get("game_path") {
        if !p.is_empty() { return p; }
    }
    launcher_dir()
}

pub fn game_exe() -> String {
    format!("{}\\steel-front.exe", game_dir())
}

pub fn game_installed() -> bool {
    std::path::Path::new(&game_exe()).exists()
}

/// 资源路径导向（启动器只写配置，游戏侧读取）
pub fn res_paths() -> HashMap<String, String> {
    let mut m = HashMap::new();
    let cfg = load();
    for k in ["maps_path", "sounds_path", "models_path"] {
        if let Some(v) = cfg.get(k) {
            m.insert(k.to_string(), v.clone());
        }
    }
    m
}

/// 写资源路径到游戏目录 resource_paths.ini（游戏侧后续读取）
pub fn write_resource_paths() {
    let cfg = load();
    let mut out = String::new();
    for k in ["maps_path", "sounds_path", "models_path"] {
        if let Some(v) = cfg.get(k) {
            out.push_str(&format!("{}={}\n", k, v));
        }
    }
    if !out.is_empty() {
        let _ = std::fs::write(format!("{}\\resource_paths.ini", game_dir()), out);
    }
}