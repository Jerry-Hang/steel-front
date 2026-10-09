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

struct LogcatLogger;

impl log::Log for LogcatLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
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
        let msg = cstr_lossy(&format!("[{}] {}", record.target(), record.args()));
        unsafe {
            __android_log_write(prio, tag.as_ptr(), msg.as_ptr());
        }
    }

    fn flush(&self) {}
}

/// 安装 logcat 后端（Android 上替代 env_logger）。
pub fn init() {
    let _ = log::set_boxed_logger(Box::new(LogcatLogger));
    log::set_max_level(log::LevelFilter::Info);
    log::info!("android_log: 日志已接入 logcat（tag=steel_front）");
}