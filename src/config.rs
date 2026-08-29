//! 轻量配置持久化：键位 + 音量 + 灵敏度，写入 `$HOME/.steel_front.cfg`（零第三方依赖）。
//!
//! 不用仓库目录，避免污染 git 工作区；解析宽松（坏行忽略），文件缺失回退默认值，
//! 因此首次运行/配置损坏时游戏始终可用默认配置启动。

use std::fs;
use std::path::{Path, PathBuf};

use crate::ui::{BindingAction, KeyBindings, QUALITY_LABELS, RESOLUTIONS};

/// 可持久化的配置子集（对应 HudState 中 volume/sensitivity/key_bindings/resolution_index/quality_index）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameConfig {
    pub volume: f32,
    pub music_volume: f32,
    pub sensitivity: f32,
    pub bindings: KeyBindings,
    /// 分辨率 (宽, 高)，默认 1280x720（与 ui.rs RESOLUTIONS 选项对齐）
    pub resolution: (u32, u32),
    /// 配置文件中是否显式保存过 resolution 行（首次运行 = false，用于按显示器宽高比选默认）
    pub resolution_explicit: bool,
    /// 画质索引（0=LOW / 1=MEDIUM / 2=HIGH，与 ui.rs QUALITY_LABELS 对齐），默认 MEDIUM
    pub quality: u32,
    /// 路径追踪全景渲染（2026-08-29：默认开启——整帧 RT core 路径追踪）
    pub pt_enable: bool,
    /// 光线追踪增量（阴影/反射射线；pt_enable 的补充开关）
    pub rt_enable: bool,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            volume: 0.8,
            music_volume: 0.6,
            sensitivity: 0.5,
            bindings: KeyBindings::defaults(),
            resolution: RESOLUTIONS[0],
            resolution_explicit: false,
            quality: 2, // HIGH（2026-08-28：用户机器全高实测 —— RTX 5060L + Zen4）
            pt_enable: true, // 2026-08-29：默认全程路径追踪（RTX 5060 RT core）+ 设置可关
            rt_enable: true,
        }
    }
}

/// 配置文件路径：`$HOME/.steel_front.cfg`；HOME 不可用时退回当前目录（如无 HOME 的嵌入环境）。
/// Windows 下 cmd/双击启动没有 HOME，回退 USERPROFILE（2026-08-15：BAT 启动分辨率丢
/// 失的根因——配置文件落到 CWD 找不到，回退默认 1280x800）。
fn config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".steel_front.cfg");
        }
    }
    #[cfg(windows)]
    if let Ok(profile) = std::env::var("USERPROFILE") {
        if !profile.is_empty() {
            return PathBuf::from(profile).join(".steel_front.cfg");
        }
    }
    PathBuf::from(".steel_front.cfg")
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
    // 键位格式版本：>=1 才解析 bind_* 行（旧版键码是已废弃的 USB HID 码，整体回退默认）
    let mut bindings_ok = false;
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
            "bindings_version" => bindings_ok = value == "1",
            "volume" => cfg.volume = value.parse::<f32>().unwrap_or(cfg.volume).clamp(0.0, 1.0),
            "music_volume" => {
                cfg.music_volume = value.parse::<f32>().unwrap_or(cfg.music_volume).clamp(0.0, 1.0);
            }
            "sensitivity" => {
                cfg.sensitivity = value.parse::<f32>().unwrap_or(cfg.sensitivity).clamp(0.0, 1.0);
            }
            "resolution" => {
                // 格式 "宽x高"；必须落在 ui.rs 选项表内才采纳（缺失/非法时保持默认）
                if let Some((w, h)) = value.split_once('x') {
                    let w = w.trim().parse::<u32>().unwrap_or(0);
                    let h = h.trim().parse::<u32>().unwrap_or(0);
                    if RESOLUTIONS.contains(&(w, h)) {
                        cfg.resolution = (w, h);
                        cfg.resolution_explicit = true;
                    }
                }
            }
            "quality" => {
                let max = (QUALITY_LABELS.len() - 1) as u32;
                cfg.quality = value.parse::<u32>().unwrap_or(cfg.quality).min(max);
            }
            // 键码 = winit 0.30 KeyCode 枚举序号（KeyW=41/KeyS=37/KeyA=19/KeyD=22/
            // KeyR=36/Space=62/ContextMenu=54），见 ui.rs KeyBindings::defaults
            "bind_forward" => {
                if bindings_ok {
                    cfg.bindings.bind(BindingAction::Forward, parse_u32(41));
                }
            }
            "bind_backward" => {
                if bindings_ok {
                    cfg.bindings.bind(BindingAction::Backward, parse_u32(37));
                }
            }
            "bind_left" => {
                if bindings_ok {
                    cfg.bindings.bind(BindingAction::Left, parse_u32(19));
                }
            }
            "bind_right" => {
                if bindings_ok {
                    cfg.bindings.bind(BindingAction::Right, parse_u32(22));
                }
            }
            "bind_reload" => {
                if bindings_ok {
                    cfg.bindings.bind(BindingAction::Reload, parse_u32(36));
                }
            }
            "bind_fire" => {
                if bindings_ok {
                    cfg.bindings.bind(BindingAction::Fire, parse_u32(0));
                }
            }
            "bind_jump" => {
                if bindings_ok {
                    cfg.bindings.bind(BindingAction::Jump, parse_u32(62));
                }
            }
            "bind_menu" => {
                if bindings_ok {
                    cfg.bindings.bind(BindingAction::Menu, parse_u32(54));
                }
            }
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
    text.push_str(&format!("music_volume={:.3}\n", cfg.music_volume));
    text.push_str(&format!("sensitivity={:.3}\n", cfg.sensitivity));
    text.push_str(&format!("resolution={}x{}\n", cfg.resolution.0, cfg.resolution.1));
    text.push_str(&format!("quality={}\n", cfg.quality));
    // 键位格式版本：旧版（无此行）键码是 USB HID 码，与 winit 0.30 KeyCode 序号错位，
    // 加载时忽略旧 bind_* 行回退默认键位（见 load_from 的 bindings_ok）
    text.push_str("bindings_version=1\n");
    let rows = [
        ("bind_forward", cfg.bindings.code_for(BindingAction::Forward)),
        ("bind_backward", cfg.bindings.code_for(BindingAction::Backward)),
        ("bind_left", cfg.bindings.code_for(BindingAction::Left)),
        ("bind_right", cfg.bindings.code_for(BindingAction::Right)),
        ("bind_reload", cfg.bindings.code_for(BindingAction::Reload)),
        ("bind_fire", cfg.bindings.code_for(BindingAction::Fire)),
        ("bind_jump", cfg.bindings.code_for(BindingAction::Jump)),
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
        cfg.resolution = (1600, 900);
        cfg.resolution_explicit = true; // save 会写 resolution 行，load 后应还原为显式
        cfg.quality = 2;
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
        fs::write(
            &garbage,
            "not a config\nvolume=abc\nsensitivity=0.25\nbind_forward=zz\n\
             resolution=999x999\nquality=99\n",
        )
        .unwrap();
        let cfg = load_from(&garbage);
        let _ = fs::remove_file(&garbage);
        assert!((cfg.volume - 0.8).abs() < 1e-6, "bad volume line ignored");
        assert!((cfg.sensitivity - 0.25).abs() < 1e-6, "valid sensitivity applied");
        assert_eq!(
            cfg.bindings.code_for(BindingAction::Forward),
            41,
            "bad bind ignored"
        );
        assert!(!cfg.resolution_explicit, "非法分辨率不应标记为显式");
        assert_eq!(
            cfg.resolution,
            RESOLUTIONS[0],
            "选项表外的分辨率应回退默认"
        );
        assert_eq!(cfg.quality, 2, "越界画质应 clamp 到最高档");
    }

    /// bindings_version=1 的配置文件：bind_* 行按 winit 0.30 KeyCode 序号解析
    #[test]
    fn bindings_version_1_loads_codes() {
        let path = std::env::temp_dir().join(format!(
            "steel_front_cfg_bindv1_{}.cfg",
            std::process::id()
        ));
        fs::write(
            &path,
            "bindings_version=1\nbind_forward=38\nbind_menu=31\nbind_fire=zz\n",
        )
        .unwrap();
        let cfg = load_from(&path);
        let _ = fs::remove_file(&path);
        assert_eq!(cfg.bindings.code_for(BindingAction::Forward), 38, "T 应生效");
        assert_eq!(cfg.bindings.code_for(BindingAction::Menu), 31, "M 应生效");
        assert_eq!(cfg.bindings.code_for(BindingAction::Fire), 0, "坏行回退默认（无键盘开火）");
        assert_eq!(cfg.bindings.code_for(BindingAction::Jump), 62, "坏行回退默认 SPACE（跳跃）");
    }

    /// 分辨率/画质 roundtrip：save 后 load 应还原新字段（临时文件，不碰真实 HOME）
    #[test]
    fn display_fields_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "steel_front_cfg_display_{}.cfg",
            std::process::id()
        ));
        let mut cfg = GameConfig::default();
        cfg.resolution = (1920, 1080);
        cfg.quality = 2;
        save_to(&path, &cfg);
        let loaded = load_from(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("cfg.tmp"));
        assert_eq!(loaded.resolution, (1920, 1080), "分辨率应 roundtrip");
        assert_eq!(loaded.quality, 2, "画质应 roundtrip");
    }

    /// 旧版配置文件（无 bindings_version，键码是已废弃的 USB HID 码）：
    /// 键位整体回退默认，音量/灵敏度照常加载，分辨率/画质回退默认
    #[test]
    fn load_tolerates_old_format_without_display_fields() {
        let path = std::env::temp_dir().join(format!(
            "steel_front_cfg_old_{}.cfg",
            std::process::id()
        ));
        fs::write(&path, "volume=0.3\nsensitivity=0.6\nbind_forward=5\n").unwrap();
        let cfg = load_from(&path);
        let _ = fs::remove_file(&path);
        assert!((cfg.volume - 0.3).abs() < 1e-6, "旧格式音量应照常加载");
        assert!((cfg.sensitivity - 0.6).abs() < 1e-6, "旧格式灵敏度应照常加载");
        assert_eq!(
            cfg.bindings,
            KeyBindings::defaults(),
            "旧格式键位（HID 码）应整体回退默认"
        );
        assert!(!cfg.resolution_explicit, "旧格式缺 resolution 行");
        assert_eq!(
            cfg.resolution,
            RESOLUTIONS[0],
            "旧格式缺 resolution 行应回退默认"
        );
        assert_eq!(cfg.quality, 2, "旧格式缺 quality 行应回退默认（默认 HIGH）");
    }
}
