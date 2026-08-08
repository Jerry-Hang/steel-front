//! 轻量配置持久化：键位 + 音量 + 灵敏度，写入 `$HOME/.steel_front.cfg`（零第三方依赖）。
//!
//! 不用仓库目录，避免污染 git 工作区；解析宽松（坏行忽略），文件缺失回退默认值，
//! 因此首次运行/配置损坏时游戏始终可用默认配置启动。

use std::fs;
use std::path::{Path, PathBuf};

use crate::ui::{BindingAction, KeyBindings};

/// 可持久化的配置子集（对应 HudState 中 volume/sensitivity/key_bindings 三个字段）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameConfig {
    pub volume: f32,
    pub sensitivity: f32,
    pub bindings: KeyBindings,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            volume: 0.8,
            sensitivity: 0.5,
            bindings: KeyBindings::defaults(),
        }
    }
}

/// 配置文件路径：`$HOME/.steel_front.cfg`；HOME 不可用时退回当前目录（如无 HOME 的嵌入环境）
fn config_path() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => PathBuf::from(home).join(".steel_front.cfg"),
        _ => PathBuf::from(".steel_front.cfg"),
    }
}

/// 读取配置；文件缺失/解析失败回退默认值（不报错，游戏始终可启动）
pub fn load() -> GameConfig {
    load_from(&config_path())
}

/// 从指定路径读取配置（拆出供测试使用，避免测试依赖/污染真实 HOME 配置）
fn load_from(path: &Path) -> GameConfig {
    let mut cfg = GameConfig::default();
    let Ok(text) = fs::read_to_string(path) else {
        return cfg;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let parse_u32 = |default: u32| value.parse::<u32>().unwrap_or(default);
        match key.trim() {
            "volume" => cfg.volume = value.parse::<f32>().unwrap_or(cfg.volume).clamp(0.0, 1.0),
            "sensitivity" => {
                cfg.sensitivity = value.parse::<f32>().unwrap_or(cfg.sensitivity).clamp(0.0, 1.0);
            }
            "bind_forward" => cfg.bindings.bind(BindingAction::Forward, parse_u32(26)),
            "bind_backward" => cfg.bindings.bind(BindingAction::Backward, parse_u32(22)),
            "bind_left" => cfg.bindings.bind(BindingAction::Left, parse_u32(4)),
            "bind_right" => cfg.bindings.bind(BindingAction::Right, parse_u32(7)),
            "bind_reload" => cfg.bindings.bind(BindingAction::Reload, parse_u32(21)),
            "bind_fire" => cfg.bindings.bind(BindingAction::Fire, parse_u32(44)),
            "bind_menu" => cfg.bindings.bind(BindingAction::Menu, parse_u32(41)),
            _ => {}
        }
    }
    cfg
}

/// 写回配置（原子性：先写临时文件再改名，避免中途崩溃留下半截文件）。
///
/// `cargo test` 下跳过写盘（测试直接用 `save_to` 指定临时路径），
/// 避免 game.rs 的设置面板测试覆盖开发机真实配置。
pub fn save(cfg: &GameConfig) {
    if cfg!(test) {
        return;
    }
    save_to(&config_path(), cfg);
}

/// 写入指定路径（拆出供测试使用）
fn save_to(path: &Path, cfg: &GameConfig) {
    let mut text = String::with_capacity(256);
    text.push_str("# Steel Front config (auto-generated)\n");
    text.push_str(&format!("volume={:.3}\n", cfg.volume));
    text.push_str(&format!("sensitivity={:.3}\n", cfg.sensitivity));
    let rows = [
        ("bind_forward", cfg.bindings.code_for(BindingAction::Forward)),
        ("bind_backward", cfg.bindings.code_for(BindingAction::Backward)),
        ("bind_left", cfg.bindings.code_for(BindingAction::Left)),
        ("bind_right", cfg.bindings.code_for(BindingAction::Right)),
        ("bind_reload", cfg.bindings.code_for(BindingAction::Reload)),
        ("bind_fire", cfg.bindings.code_for(BindingAction::Fire)),
        ("bind_menu", cfg.bindings.code_for(BindingAction::Menu)),
    ];
    for (k, code) in rows {
        text.push_str(&format!("{}={}\n", k, code));
    }
    let tmp = path.with_extension("cfg.tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// roundtrip：save 后 load 应还原键位/音量/灵敏度（用临时文件，不碰真实 HOME）
    #[test]
    fn save_then_load_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "steel_front_cfg_roundtrip_{}.cfg",
            std::process::id()
        ));
        let mut cfg = GameConfig::default();
        cfg.volume = 0.42;
        cfg.sensitivity = 0.77;
        cfg.bindings.bind(BindingAction::Forward, 5); // T
        cfg.bindings.bind(BindingAction::Fire, 6); // Y
        save_to(&path, &cfg);
        let loaded = load_from(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("cfg.tmp"));
        assert_eq!(loaded, cfg, "roundtrip should restore exact config");
    }

    /// 损坏/缺失文件应回退默认值而不是崩溃
    #[test]
    fn load_tolerates_missing_and_garbage() {
        let missing = std::env::temp_dir().join("steel_front_cfg_missing.cfg");
        let _ = fs::remove_file(&missing);
        let cfg = load_from(&missing);
        assert_eq!(cfg, GameConfig::default());
        // 含坏行的文件：只认合法行
        let garbage = std::env::temp_dir().join("steel_front_cfg_garbage.cfg");
        fs::write(&garbage, "not a config\nvolume=abc\nsensitivity=0.25\nbind_forward=zz\n")
            .unwrap();
        let cfg = load_from(&garbage);
        let _ = fs::remove_file(&garbage);
        assert!((cfg.volume - 0.8).abs() < 1e-6, "bad volume line ignored");
        assert!((cfg.sensitivity - 0.25).abs() < 1e-6, "valid sensitivity applied");
        assert_eq!(cfg.bindings.code_for(BindingAction::Forward), 26, "bad bind ignored");
    }
}
