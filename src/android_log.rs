//! Android 日志后端：把 `log` 宏的输出送进 logcat。
//!
//! 为什么需要它（`docs/HANDOFF-mobile.md` 5.2）：桌面用 `env_logger` 写 stderr，
//! 而 Android 应用的 stderr 没有人看（被系统丢弃）—— 手机上一出问题就是"什么都看不到"。
//! 这里直接把日志写进 logcat（`__android_log_write`），`adb logcat` / `logcat` 即可读。
//!
//! 实现方式：直接 FFI 到系统的 `liblog.so`，**不引入新依赖**。

use std::ffi::CString;

// android/log.h 的优先级常量
const ANDROID_LOG_DEBUG: i32 = 3;
const ANDROID_LOG_INFO: i32 = 4;
const ANDROID_LOG_WARN: i32 = 5;
const ANDROID_LOG_ERROR: i32 = 6;

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: i32, tag: *const std::os::raw::c_char, text: *const std::os::raw::c_char) -> i32;
}

/// 去掉 NUL（`CString::new` 遇到内嵌 NUL 会失败，日志不该因此丢）
fn cstr_lossy(s: &str) -> CString {
    let cleaned: String = s.chars().filter(|c| *c != '\0').collect();
    CString::new(cleaned).unwrap_or_default()
}

/// 按**字节上限**切分，但只在 UTF-8 字符边界上断开。
///
/// 为什么不用 `as_bytes().chunks()`：那是按字节切，而中文一个字 3 字节，
/// 切在中间就得到**非法 UTF-8** ⇒ logcat 里显示成乱码/问号。
/// 这里走 `char_indices`，保证每一段都是合法 UTF-8。
fn split_by_bytes(s: &str, max: usize) -> Vec<&str> {
    if s.len() <= max {
        return vec![s];
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, _) in s.char_indices() {
        if i - start >= max {
            out.push(&s[start..i]);
            start = i;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

struct LogcatLogger;

impl log::Log for LogcatLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        let prio = match record.level() {
            log::Level::Error => ANDROID_LOG_ERROR,
            log::Level::Warn => ANDROID_LOG_WARN,
            log::Level::Info => ANDROID_LOG_INFO,
            _ => ANDROID_LOG_DEBUG,
        };
        // tag 固定，便于 `logcat -s steel_front` 过滤
        let tag = cstr_lossy("steel_front");
        let full = format!("[{}] {}", record.target(), record.args());
        // 🔴 **单条超长日志必须切分**（外部复核指出第 5 条）：
        // Android 的 `LOGGER_ENTRY_MAX_PAYLOAD` = **4068 字节**，一次写超过它的部分
        // 会被 logd **静默丢弃**（不报错、不留痕）。留 500 余量给 tag 与分隔符。
        const CHUNK: usize = 3500;
        for part in split_by_bytes(&full, CHUNK) {
            let msg = cstr_lossy(part);
            unsafe {
                __android_log_write(prio, tag.as_ptr(), msg.as_ptr());
            }
        }
    }

    fn flush(&self) {}
}

/// 安装 logcat 后端（Android 上替代 env_logger）。
pub fn init() {
    let _ = log::set_boxed_logger(Box::new(LogcatLogger));
    // 🔴 **级别可配**（外部复核指出第 5 条）：原来写死 `Info` ⇒ Android 真机上
    // 永远看不到 `log::debug!`，而桌面端设 `RUST_LOG=debug` 就能看 —— **两端行为不一致**，
    // 而 Android 恰恰是最需要日志的那一端（`adb logcat` 是唯一现场）。
    // 优先级：`RV3D_LOG` > `RUST_LOG` > 默认 `info`。
    // ⚠️ **必须走 `syscfg`**：直接 `std::env::var("RV3D_*")` 在 Android 上会静默失效
    // （配置走的是另一条通道）—— 判据 `rv3d_switches_never_read_std_env` 当场抓到了我这版。
    let raw = crate::syscfg::cfg("RV3D_LOG")
        .or_else(|| crate::syscfg::cfg("RUST_LOG"))
        .unwrap_or_else(|| "info".to_string());
    let level = match raw.trim().to_ascii_lowercase().as_str() {
        "off" => log::LevelFilter::Off,
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        _ => log::LevelFilter::Info,
    };
    log::set_max_level(level);
    log::info!("android_log: 日志已接入 logcat（tag=steel_front）");
}